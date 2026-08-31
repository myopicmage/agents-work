use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use agents_work::publish::{PublishError, publish};
use agents_work::validate::{sha256_bytes, validate_case};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

struct TemporaryCase {
    root: PathBuf,
    case: PathBuf,
}

impl TemporaryCase {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "agents-work-publish-{}-{label}-{unique}",
            std::process::id()
        ));
        let case = root.join("case");
        fs::create_dir_all(&case).expect("temporary case should be created");
        Self { root, case }
    }

    fn write_manifest(&self) {
        fs::write(self.case.join("work.toml"), manifest()).expect("manifest should be written");
    }

    fn prepare(&self, text: &str) -> PathBuf {
        let draft = self.root.join("draft.md");
        fs::write(&draft, text).expect("draft should be written");
        draft
    }

    fn resolved(&self) -> PathBuf {
        fs::canonicalize(&self.case).expect("temporary case should resolve")
    }
}

impl Drop for TemporaryCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn manifest() -> &'static str {
    r#"schema_version = 2
id = "test-case"
title = "Test case"
repository_name = "repo"
repository_path = "/tmp/repo"
phase = "planning"
status = "drafting"
next_agent = "codex"
requested_action = "Test."
implementation_branch = ""
pull_request_system = ""
pull_request_id = ""
reviewed_commit = ""
created_at = "2026-07-27T19:00:00+09:00"
updated_at = "2026-07-27T19:00:00+09:00"
"#
}

fn artifact(responds_to: &[&str]) -> String {
    let relationships = responds_to
        .iter()
        .map(|item| format!(r#""{item}""#))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"+++
artifact_schema_version = 1
artifact_id = "a1b2c3"
sequence = 1
kind = "plan"
topic = "test-plan"
author = "codex"
created_at = 2026-07-27T19:00:00+09:00
responds_to = [{relationships}]
supersedes = []
source_branch = ""
source_commit = ""
source_path = ""
subject_repository = ""
subject_path = ""
subject_commit = ""
+++
# Test artifact

Body.
"#
    )
}

#[test]
fn publish_installs_exact_bytes_and_sidecar_then_validates() {
    let fixture = TemporaryCase::new("success");
    fixture.write_manifest();
    let text = artifact(&[]);
    let draft = fixture.prepare(&text);
    let mut standard_output = Vec::new();

    let published = publish(&fixture.case, &draft, &mut standard_output)
        .expect("prepared draft should publish");
    let name = "001-test-plan-codex-a1b2c3.md";
    let expected = fixture.resolved().join(name);

    assert_eq!(published, expected);
    assert_eq!(
        standard_output,
        format!("{}\n", expected.display()).as_bytes()
    );
    assert_eq!(
        fs::read(&published).expect("artifact should be readable"),
        text.as_bytes()
    );
    assert_eq!(
        fs::read(published.with_file_name(format!("{name}.sha256")))
            .expect("sidecar should be readable"),
        format!("{}  {name}\n", sha256_bytes(text.as_bytes())).as_bytes()
    );
    assert_eq!(
        fs::read(&draft).expect("source draft should remain"),
        text.as_bytes()
    );
    assert_no_temporary_artifact(&fixture.case);
    assert!(
        validate_case(&fixture.case)
            .expect("published case should be traversable")
            .is_valid()
    );

    #[cfg(unix)]
    assert_eq!(
        fs::metadata(published)
            .expect("artifact metadata should be readable")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}

#[test]
fn publish_rejects_an_existing_final_or_sidecar_without_changes() {
    let name = "001-test-plan-codex-a1b2c3.md";

    for occupied in [name.to_owned(), format!("{name}.sha256")] {
        let fixture = TemporaryCase::new("no-clobber");
        fixture.write_manifest();
        let draft = fixture.prepare(&artifact(&[]));
        fs::write(fixture.case.join(&occupied), "keep me\n")
            .expect("occupied path should be written");
        let before = directory_bytes(&fixture.case);

        let error = publish(&fixture.case, &draft, &mut Vec::new())
            .expect_err("occupied publication path should fail");

        assert_eq!(
            error.to_string(),
            format!("{name}: final artifact or sidecar already exists; use a new ID")
        );
        assert_eq!(directory_bytes(&fixture.case), before);
    }
}

#[cfg(unix)]
#[test]
fn failed_exclusive_sidecar_create_does_not_delete_a_dangling_symlink() {
    use std::os::unix::fs::symlink;

    let fixture = TemporaryCase::new("dangling-sidecar");
    fixture.write_manifest();
    let draft = fixture.prepare(&artifact(&[]));
    let sidecar = fixture.case.join("001-test-plan-codex-a1b2c3.md.sha256");
    let missing_target = fixture.root.join("missing-sidecar-target");
    symlink(&missing_target, &sidecar).expect("dangling sidecar symlink should be created");

    assert!(!sidecar.exists());
    let error = publish(&fixture.case, &draft, &mut Vec::new())
        .expect_err("exclusive sidecar creation should lose to the symlink");

    assert!(matches!(
        error,
        PublishError::Io(ref error) if error.kind() == std::io::ErrorKind::AlreadyExists
    ));
    assert_eq!(
        fs::read_link(&sidecar).expect("competing symlink should remain"),
        missing_target
    );
    assert_no_temporary_artifact(&fixture.case);
}

#[cfg(unix)]
#[test]
fn failed_link_preserves_a_dangling_final_symlink_and_removes_our_sidecar() {
    use std::os::unix::fs::symlink;

    let fixture = TemporaryCase::new("dangling-final");
    fixture.write_manifest();
    let draft = fixture.prepare(&artifact(&[]));
    let final_path = fixture.case.join("001-test-plan-codex-a1b2c3.md");
    let sidecar = fixture.case.join("001-test-plan-codex-a1b2c3.md.sha256");
    let missing_target = fixture.root.join("missing-artifact-target");
    symlink(&missing_target, &final_path).expect("dangling final symlink should be created");

    assert!(!final_path.exists());
    let error = publish(&fixture.case, &draft, &mut Vec::new())
        .expect_err("hard link should lose to the symlink");

    assert!(matches!(
        error,
        PublishError::Io(ref error) if error.kind() == std::io::ErrorKind::AlreadyExists
    ));
    assert_eq!(
        fs::read_link(&final_path).expect("competing final symlink should remain"),
        missing_target
    );
    assert!(!sidecar.exists());
    assert_no_temporary_artifact(&fixture.case);
}

#[test]
fn malformed_prepared_artifacts_include_the_exact_draft_hint() {
    let cases = [
        (
            "missing-front-matter",
            "# No front matter\n".to_owned(),
            "draft.md: missing TOML front matter",
            "KIND",
            "AUTHOR",
        ),
        (
            "missing-sequence",
            artifact(&[]).replace("sequence = 1\n", ""),
            "draft.md: missing fields: sequence",
            "plan",
            "codex",
        ),
        (
            "mistyped-sequence",
            artifact(&[]).replace("sequence = 1", "sequence = \"1\""),
            "draft.md: sequence must be a positive integer",
            "plan",
            "codex",
        ),
    ];

    for (label, text, diagnostic, kind, author) in cases {
        let fixture = TemporaryCase::new(label);
        fixture.write_manifest();
        let draft = fixture.prepare(&text);

        let error = publish(&fixture.case, &draft, &mut Vec::new())
            .expect_err("malformed prepared artifact should fail");
        let expected_hint = format!(
            "hint: agents-work draft {} --kind {kind} --author {author} generates valid front matter",
            fixture.resolved().display(),
        );

        assert_eq!(error.to_string(), format!("{diagnostic}\n{expected_hint}"));
        assert_eq!(
            directory_bytes(&fixture.case),
            vec![("work.toml".to_owned(), manifest().as_bytes().to_vec())]
        );
    }
}

#[test]
fn missing_relationship_has_no_draft_hint_and_writes_nothing() {
    let fixture = TemporaryCase::new("missing-relationship");
    fixture.write_manifest();
    let missing = "001-missing-codex-a1b2c3.md";
    let draft = fixture.prepare(&artifact(&[missing]));
    let before = directory_bytes(&fixture.case);

    let error = publish(&fixture.case, &draft, &mut Vec::new())
        .expect_err("missing relationship should fail");

    assert_eq!(
        error.to_string(),
        format!("draft.md: responds_to target missing: {missing}")
    );
    assert!(!error.to_string().contains("agents-work draft"));
    assert_eq!(directory_bytes(&fixture.case), before);
}

#[test]
fn missing_manifest_is_reported_before_a_missing_draft() {
    let fixture = TemporaryCase::new("missing-inputs");
    let draft = fixture.root.join("missing.md");

    let missing_manifest = publish(&fixture.case, &draft, &mut Vec::new())
        .expect_err("manifest check should run first");
    assert_eq!(
        missing_manifest.to_string(),
        format!("{}: missing work.toml", fixture.resolved().display())
    );

    fixture.write_manifest();
    let missing_draft = publish(&fixture.case, &draft, &mut Vec::new())
        .expect_err("missing draft should fail after manifest exists");
    assert_eq!(
        missing_draft.to_string(),
        format!(
            "{}: draft does not exist",
            fs::canonicalize(&fixture.root)
                .expect("fixture root should resolve")
                .join("missing.md")
                .display()
        )
    );
}

#[test]
fn publish_command_routes_success_and_no_clobber_failure() {
    let fixture = TemporaryCase::new("cli");
    fixture.write_manifest();
    let draft = fixture.prepare(&artifact(&[]));
    let arguments = [
        OsStr::new("publish"),
        fixture.case.as_os_str(),
        draft.as_os_str(),
    ];

    let first = Command::new(env!("CARGO_BIN_EXE_agents-work"))
        .args(arguments)
        .output()
        .expect("publish command should run");
    let second = Command::new(env!("CARGO_BIN_EXE_agents-work"))
        .args(arguments)
        .output()
        .expect("second publish command should run");

    assert_eq!(first.status.code(), Some(0));
    assert!(first.stderr.is_empty());
    assert_eq!(
        first.stdout,
        format!(
            "{}\n",
            fixture
                .resolved()
                .join("001-test-plan-codex-a1b2c3.md")
                .display()
        )
        .as_bytes()
    );
    assert_eq!(second.status.code(), Some(1));
    assert!(second.stdout.is_empty());
    assert_eq!(
        String::from_utf8(second.stderr).expect("stderr should be UTF-8"),
        "error: 001-test-plan-codex-a1b2c3.md: final artifact or sidecar already exists; use a new ID\n"
    );
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn publish_cli_matches_the_python_reference() {
    let python_fixture = TemporaryCase::new("oracle-python-success");
    let rust_fixture = TemporaryCase::new("oracle-rust-success");
    python_fixture.write_manifest();
    rust_fixture.write_manifest();
    let text = artifact(&[]);
    let python_draft = python_fixture.prepare(&text);
    let rust_draft = rust_fixture.prepare(&text);

    let python = run_python_publish(&python_fixture.case, &python_draft);
    let rust = run_rust_publish(&rust_fixture.case, &rust_draft);

    assert_eq!(rust.status.code(), python.status.code());
    assert_eq!(rust.stderr, python.stderr);
    assert_eq!(
        normalize_case(&rust.stdout, &rust_fixture.resolved()),
        normalize_case(&python.stdout, &python_fixture.resolved())
    );
    assert_eq!(
        directory_bytes(&rust_fixture.case),
        directory_bytes(&python_fixture.case)
    );
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn publish_failure_diagnostics_match_the_python_reference() {
    for scenario in [
        FailureScenario::MissingManifest,
        FailureScenario::MissingDraft,
        FailureScenario::MissingFrontMatter,
        FailureScenario::MissingSequence,
        FailureScenario::MistypedSequence,
        FailureScenario::MissingRelationship,
        FailureScenario::ExistingFinal,
        FailureScenario::ExistingSidecar,
    ] {
        assert_failure_parity(scenario);
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn failed_exclusive_create_has_the_same_ownership_as_python() {
    use std::os::unix::fs::symlink;

    let python_fixture = TemporaryCase::new("oracle-python-sidecar-race");
    let rust_fixture = TemporaryCase::new("oracle-rust-sidecar-race");
    python_fixture.write_manifest();
    rust_fixture.write_manifest();
    let python_draft = python_fixture.prepare(&artifact(&[]));
    let rust_draft = rust_fixture.prepare(&artifact(&[]));
    let sidecar_name = "001-test-plan-codex-a1b2c3.md.sha256";
    let python_sidecar = python_fixture.case.join(sidecar_name);
    let rust_sidecar = rust_fixture.case.join(sidecar_name);
    let python_target = python_fixture.root.join("competing-writer");
    let rust_target = rust_fixture.root.join("competing-writer");
    symlink(&python_target, &python_sidecar).expect("Python fixture symlink should be created");
    symlink(&rust_target, &rust_sidecar).expect("Rust fixture symlink should be created");

    let python = run_python_publish(&python_fixture.case, &python_draft);
    let rust = run_rust_publish(&rust_fixture.case, &rust_draft);

    assert_eq!(rust.status.code(), python.status.code());
    assert_eq!(rust.status.code(), Some(1));
    assert!(rust.stdout.is_empty());
    assert!(python.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rust.stderr).contains("File exists"));
    assert!(String::from_utf8_lossy(&python.stderr).contains("File exists"));
    assert_eq!(
        fs::read_link(&rust_sidecar).expect("Rust must preserve competing path"),
        rust_target
    );
    assert_eq!(
        fs::read_link(&python_sidecar).expect("Python must preserve competing path"),
        python_target
    );
    assert_no_temporary_artifact(&rust_fixture.case);
    assert_no_temporary_artifact(&python_fixture.case);
}

#[derive(Clone, Copy)]
enum FailureScenario {
    MissingManifest,
    MissingDraft,
    MissingFrontMatter,
    MissingSequence,
    MistypedSequence,
    MissingRelationship,
    ExistingFinal,
    ExistingSidecar,
}

fn assert_failure_parity(scenario: FailureScenario) {
    let python_fixture = TemporaryCase::new("oracle-python-failure");
    let rust_fixture = TemporaryCase::new("oracle-rust-failure");
    let python_draft = prepare_failure(&python_fixture, scenario);
    let rust_draft = prepare_failure(&rust_fixture, scenario);
    let python_before = directory_bytes(&python_fixture.case);
    let rust_before = directory_bytes(&rust_fixture.case);

    let python = run_python_publish(&python_fixture.case, &python_draft);
    let rust = run_rust_publish(&rust_fixture.case, &rust_draft);

    assert_eq!(rust.status.code(), python.status.code());
    assert_eq!(rust.stdout, python.stdout);
    assert_eq!(
        normalize_paths(&rust.stderr, &rust_fixture.resolved(), &rust_draft,),
        normalize_paths(&python.stderr, &python_fixture.resolved(), &python_draft,)
    );
    assert_eq!(directory_bytes(&rust_fixture.case), rust_before);
    assert_eq!(directory_bytes(&python_fixture.case), python_before);
}

fn prepare_failure(fixture: &TemporaryCase, scenario: FailureScenario) -> PathBuf {
    if !matches!(scenario, FailureScenario::MissingManifest) {
        fixture.write_manifest();
    }

    let text = match scenario {
        FailureScenario::MissingFrontMatter => "# No front matter\n".to_owned(),
        FailureScenario::MissingSequence => artifact(&[]).replace("sequence = 1\n", ""),
        FailureScenario::MistypedSequence => {
            artifact(&[]).replace("sequence = 1", "sequence = \"1\"")
        }
        FailureScenario::MissingRelationship => artifact(&["001-missing-codex-a1b2c3.md"]),
        _ => artifact(&[]),
    };
    let draft = fixture.root.join("draft.md");

    if !matches!(scenario, FailureScenario::MissingDraft) {
        fs::write(&draft, text).expect("failure draft should be written");
    }

    let final_name = "001-test-plan-codex-a1b2c3.md";

    match scenario {
        FailureScenario::ExistingFinal => {
            fs::write(fixture.case.join(final_name), "occupied\n")
                .expect("final path should be occupied");
        }
        FailureScenario::ExistingSidecar => {
            fs::write(
                fixture.case.join(format!("{final_name}.sha256")),
                "occupied\n",
            )
            .expect("sidecar path should be occupied");
        }
        _ => {}
    }

    draft
}

fn run_python_publish(case: &Path, draft: &Path) -> Output {
    let reference = std::env::var_os("AGENTS_WORK_PYTHON_REFERENCE")
        .expect("AGENTS_WORK_PYTHON_REFERENCE must name agents_work.py");
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| OsString::from("python3"));

    Command::new(python)
        .arg(reference)
        .arg("publish")
        .arg(case)
        .arg(draft)
        .output()
        .expect("Python publish should run")
}

fn run_rust_publish(case: &Path, draft: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agents-work"))
        .arg("publish")
        .arg(case)
        .arg(draft)
        .output()
        .expect("Rust publish should run")
}

fn normalize_case(output: &[u8], case: &Path) -> Vec<u8> {
    String::from_utf8(output.to_owned())
        .expect("process output should be UTF-8")
        .replace(&case.display().to_string(), "<CASE>")
        .into_bytes()
}

fn normalize_paths(output: &[u8], case: &Path, draft: &Path) -> Vec<u8> {
    String::from_utf8(output.to_owned())
        .expect("process output should be UTF-8")
        .replace(&case.display().to_string(), "<CASE>")
        .replace(&draft.display().to_string(), "<DRAFT>")
        .into_bytes()
}

fn assert_no_temporary_artifact(case: &Path) {
    assert!(
        fs::read_dir(case)
            .expect("case should be readable")
            .all(|entry| !entry
                .expect("directory entry should be readable")
                .file_name()
                .to_string_lossy()
                .starts_with(".artifact-"))
    );
}

fn directory_bytes(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = fs::read_dir(directory)
        .expect("directory should be readable")
        .map(|entry| {
            let path = entry.expect("directory entry should be readable").path();
            (
                path.file_name()
                    .expect("entry should have a name")
                    .to_string_lossy()
                    .into_owned(),
                fs::read(path).expect("fixture file should be readable"),
            )
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}
