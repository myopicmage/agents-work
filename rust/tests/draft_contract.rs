use std::cell::Cell;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use agents_work::artifact::{
    ArtifactId, ArtifactKind, ArtifactMetadata, ArtifactName, OffsetDateTime, parse_front_matter,
};
use agents_work::draft::{DraftError, DraftRequest, draft_with, resolve_reference};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

struct TemporaryCase {
    root: PathBuf,
    case: PathBuf,
}

impl TemporaryCase {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "agents-work-draft-{}-{label}-{unique}",
            std::process::id()
        ));
        let case = root.join("case");
        fs::create_dir_all(&case).expect("temporary case should be created");
        Self { root, case }
    }

    fn write_manifest(&self, text: &str) {
        fs::write(self.case.join("work.toml"), text).expect("manifest should be written");
    }

    fn write_artifact(&self, name: &str) {
        fs::write(self.case.join(name), "fixture\n").expect("artifact should be written");
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

fn manifest(id: &str) -> String {
    format!(
        r#"schema_version = 2
id = "{id}"
phase = "planning"
status = "drafting"
next_agent = "codex"
"#
    )
}

fn timestamp() -> OffsetDateTime {
    "2026-08-09T18:30:45+09:00"
        .parse()
        .expect("test timestamp should parse")
}

fn artifact_id(value: &str) -> ArtifactId {
    value.parse().expect("test artifact ID should parse")
}

fn request<'a>(
    case: &'a Path,
    author: &'a str,
    topic: Option<&'a str>,
    responds_to: &'a [String],
    supersedes: &'a [String],
    output: Option<&'a Path>,
) -> DraftRequest<'a> {
    DraftRequest {
        case,
        kind: ArtifactKind::Review,
        author,
        topic,
        responds_to,
        supersedes,
        output,
    }
}

#[test]
fn explicit_time_and_id_render_exact_python_bytes() {
    let fixture = TemporaryCase::new("exact-bytes");
    fixture.write_manifest(&manifest("test-case"));
    let earlier = "001-earlier-plan-codex-010203.md";
    fixture.write_artifact(earlier);
    let responds_to = vec!["1".to_owned(), earlier.to_owned()];
    let supersedes = Vec::new();
    let mut standard_output = Vec::new();

    let drafted = draft_with(
        request(
            &fixture.case,
            "claude",
            None,
            &responds_to,
            &supersedes,
            None,
        ),
        || Ok(timestamp()),
        || Ok(artifact_id("a1b2c3")),
        &mut standard_output,
    )
    .expect("deterministic draft should be written");
    let expected_name = ".draft-002-test-case-claude-a1b2c3.md";
    let expected_path = fixture.resolved().join(expected_name);

    assert_eq!(drafted, expected_path);
    assert_eq!(
        String::from_utf8(standard_output).expect("stdout should be UTF-8"),
        format!("{}\n", expected_path.display())
    );
    assert_eq!(
        fs::read_to_string(drafted).expect("draft should be readable"),
        format!(
            r#"+++
artifact_schema_version = 1
artifact_id = "a1b2c3"
sequence = 2
kind = "review"
topic = "test-case"
author = "claude"
created_at = 2026-08-09T18:30:45+09:00
responds_to = [
  "{earlier}",
  "{earlier}",
]
supersedes = []
source_branch = ""
source_commit = ""
source_path = ""
subject_repository = ""
subject_path = ""
subject_commit = ""
+++

# TITLE
"#
        )
    );
}

#[test]
fn occupied_ids_retry_and_sequence_follows_the_highest_filename() {
    let fixture = TemporaryCase::new("collision");
    fixture.write_manifest(&manifest("test-case"));
    fixture.write_artifact("001-first-plan-codex-a1b2c3.md");
    fixture.write_artifact("999-later-review-claude-d4e5f6.md");
    let references = Vec::new();
    let mut ids = [artifact_id("a1b2c3"), artifact_id("ffffff")].into_iter();
    let calls = Cell::new(0_u8);

    let drafted = draft_with(
        request(
            &fixture.case,
            "claude",
            None,
            &references,
            &references,
            None,
        ),
        || Ok(timestamp()),
        || {
            calls.set(calls.get() + 1);
            Ok(ids.next().expect("a free deterministic ID should remain"))
        },
        &mut Vec::new(),
    )
    .expect("collision should retry");

    assert_eq!(calls.get(), 2);
    assert_eq!(
        drafted.file_name().and_then(OsStr::to_str),
        Some(".draft-1000-test-case-claude-ffffff.md")
    );
}

#[test]
fn full_decimal_and_unicode_decimal_references_match_python() {
    let first = "001-alpha-plan-claude-a1b2c3.md"
        .parse::<ArtifactName>()
        .expect("first name should parse");
    let second = "001-zeta-review-codex-d4e5f6.md"
        .parse::<ArtifactName>()
        .expect("second name should parse");
    let inventory = [second.clone(), first.clone()];

    assert_eq!(
        resolve_reference(first.as_str(), &inventory)
            .expect("full name should win")
            .as_str(),
        first.as_str()
    );

    for reference in ["0001", "１", "١", "௧"] {
        assert_eq!(
            resolve_reference(reference, std::slice::from_ref(&first))
                .expect("decimal sequence should resolve")
                .as_str(),
            first.as_str()
        );
    }

    for reference in [
        "\u{10D41}",
        "\u{116D1}",
        "\u{116DB}",
        "\u{11BF1}",
        "\u{16131}",
        "\u{16D71}",
        "\u{1CCF1}",
        "\u{1E5F2}",
    ] {
        assert_eq!(
            resolve_reference(reference, std::slice::from_ref(&first))
                .expect("Unicode 16 decimal sequence should resolve")
                .as_str(),
            first.as_str()
        );
    }

    assert_eq!(
        resolve_reference("\u{116D0}", &inventory)
            .expect_err("decimal zero should resolve to an absent sequence")
            .to_string(),
        "no artifact with sequence 0"
    );

    assert_eq!(
        resolve_reference("1", &inventory)
            .expect_err("duplicate sequence should be ambiguous")
            .to_string(),
        format!(
            "sequence 1 is ambiguous, name one of: {}, {}",
            first.as_str(),
            second.as_str()
        )
    );
    assert_eq!(
        resolve_reference("7", &inventory)
            .expect_err("absent sequence should fail")
            .to_string(),
        "no artifact with sequence 7"
    );
    assert_eq!(
        resolve_reference("not-a-reference", &inventory)
            .expect_err("non-decimal shorthand should fail")
            .to_string(),
        "unknown artifact reference: not-a-reference"
    );
}

#[test]
fn manifest_may_be_structurally_invalid_when_its_id_is_usable() {
    let fixture = TemporaryCase::new("invalid-manifest-structure");
    fixture.write_manifest(
        r#"schema_version = 99
id = "usable-id"
phase = "unknown"
status = "unknown"
next_agent = 42
"#,
    );
    let references = Vec::new();

    let drafted = draft_with(
        request(
            &fixture.case,
            "claude",
            None,
            &references,
            &references,
            None,
        ),
        || Ok(timestamp()),
        || Ok(artifact_id("a1b2c3")),
        &mut Vec::new(),
    )
    .expect("Python ignores structural manifest errors during draft");

    assert_eq!(
        drafted.file_name().and_then(OsStr::to_str),
        Some(".draft-001-usable-id-claude-a1b2c3.md")
    );
}

#[test]
fn topic_override_bypasses_an_unusable_manifest_id() {
    let fixture = TemporaryCase::new("topic-override");
    fixture.write_manifest(&manifest("Bad-ID"));
    let references = Vec::new();

    let drafted = draft_with(
        request(
            &fixture.case,
            "claude",
            Some("explicit-topic"),
            &references,
            &references,
            None,
        ),
        || Ok(timestamp()),
        || Ok(artifact_id("a1b2c3")),
        &mut Vec::new(),
    )
    .expect("explicit valid topic should win");

    assert_eq!(
        drafted.file_name().and_then(OsStr::to_str),
        Some(".draft-001-explicit-topic-claude-a1b2c3.md")
    );
}

#[test]
fn invalid_topic_stops_before_entropy_or_filesystem_output() {
    let fixture = TemporaryCase::new("invalid-topic");
    fixture.write_manifest(&manifest("Bad-ID"));
    let references = Vec::new();
    let entropy_called = Cell::new(false);

    let error = draft_with(
        request(
            &fixture.case,
            "claude",
            None,
            &references,
            &references,
            None,
        ),
        || Ok(timestamp()),
        || {
            entropy_called.set(true);
            Ok(artifact_id("a1b2c3"))
        },
        &mut Vec::new(),
    )
    .expect_err("invalid topic should fail");

    assert_eq!(
        error.to_string(),
        "topic must be a lowercase slug; work.toml has no usable id, so pass --topic"
    );
    assert!(!entropy_called.get());
    assert_eq!(
        fs::read_dir(&fixture.case)
            .expect("case should list")
            .count(),
        1
    );
}

#[test]
fn invalid_author_uses_the_generated_filename_diagnostic() {
    let fixture = TemporaryCase::new("invalid-author");
    fixture.write_manifest(&manifest("test-case"));
    let references = Vec::new();

    let error = draft_with(
        request(
            &fixture.case,
            "Bad-Author",
            None,
            &references,
            &references,
            None,
        ),
        || Ok(timestamp()),
        || Ok(artifact_id("a1b2c3")),
        &mut Vec::new(),
    )
    .expect_err("invalid author should fail generated metadata validation");

    assert_eq!(
        error.to_string(),
        "001-test-case-Bad-Author-a1b2c3.md: author must be a lowercase slug"
    );
    assert_eq!(
        fs::read_dir(&fixture.case)
            .expect("case should list")
            .count(),
        1
    );
}

#[test]
fn missing_or_unreadable_manifest_uses_the_draft_diagnostic() {
    for (label, contents) in [("missing", None), ("invalid", Some("not = [toml\n"))] {
        let fixture = TemporaryCase::new(label);

        if let Some(contents) = contents {
            fixture.write_manifest(contents);
        }

        let references = Vec::new();
        let error = draft_with(
            request(
                &fixture.case,
                "claude",
                None,
                &references,
                &references,
                None,
            ),
            || Ok(timestamp()),
            || Ok(artifact_id("a1b2c3")),
            &mut Vec::new(),
        )
        .expect_err("missing or unreadable manifest should fail");

        assert_eq!(
            error.to_string(),
            format!(
                "{}: missing or unreadable work.toml",
                fixture.resolved().display()
            )
        );
    }
}

#[test]
fn explicit_output_is_resolved_and_never_clobbered() {
    let fixture = TemporaryCase::new("explicit-output");
    fixture.write_manifest(&manifest("test-case"));
    let references = Vec::new();
    let output = fixture.root.join("prepared.md");
    fs::write(&output, "keep me\n").expect("existing output should be created");

    let error = draft_with(
        request(
            &fixture.case,
            "claude",
            None,
            &references,
            &references,
            Some(&output),
        ),
        || Ok(timestamp()),
        || Ok(artifact_id("a1b2c3")),
        &mut Vec::new(),
    )
    .expect_err("exclusive creation should reject an existing output");

    assert!(matches!(error, DraftError::Io(_)));
    assert_eq!(
        fs::read_to_string(output).expect("existing output should remain"),
        "keep me\n"
    );
}

#[test]
fn production_cli_supplies_real_local_time_and_entropy() {
    let fixture = TemporaryCase::new("production-cli");
    fixture.write_manifest(&manifest("test-case"));
    let output = fixture.root.join("cli-draft.md");
    let process = Command::new(env!("CARGO_BIN_EXE_agents-work"))
        .args([
            OsStr::new("draft"),
            fixture.case.as_os_str(),
            OsStr::new("--kind"),
            OsStr::new("review"),
            OsStr::new("--author"),
            OsStr::new("claude"),
            OsStr::new("--output"),
            output.as_os_str(),
        ])
        .output()
        .expect("draft process should run");

    assert_eq!(process.status.code(), Some(0));
    assert!(process.stderr.is_empty());
    let resolved_output = fs::canonicalize(&output).expect("CLI output path should resolve");
    assert_eq!(
        String::from_utf8(process.stdout).expect("stdout should be UTF-8"),
        format!("{}\n", resolved_output.display())
    );

    let data = fs::read(&output).expect("CLI draft should exist");
    let table = parse_front_matter(&data, &output).expect("CLI front matter should parse");
    let metadata = ArtifactMetadata::parse(&table, &output).expect("CLI metadata should validate");

    assert_eq!(metadata.sequence.get(), 1);
    assert_eq!(metadata.topic.as_str(), "test-case");
    assert_eq!(metadata.author.as_str(), "claude");
    assert_eq!(metadata.artifact_id.to_string().len(), 6);
    assert!(data.ends_with(b"+++\n\n# TITLE\n"));
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn draft_cli_matches_normalized_python_output() {
    let reference = std::env::var_os("AGENTS_WORK_PYTHON_REFERENCE")
        .expect("AGENTS_WORK_PYTHON_REFERENCE must name agents_work.py");
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| OsStr::new("python3").to_owned());
    let fixture = TemporaryCase::new("oracle");
    fixture.write_manifest(&manifest("test-case"));
    let earlier = "001-earlier-plan-codex-010203.md";
    fixture.write_artifact(earlier);
    let python_output = fixture.root.join("python-draft.md");
    let rust_output = fixture.root.join("rust-draft.md");
    let common = [
        OsStr::new("draft"),
        fixture.case.as_os_str(),
        OsStr::new("--kind"),
        OsStr::new("review"),
        OsStr::new("--author"),
        OsStr::new("claude"),
        OsStr::new("--responds-to"),
        OsStr::new("１"),
        OsStr::new("--responds-to"),
        OsStr::new("\u{16D71}"),
        OsStr::new("--supersedes"),
        OsStr::new(earlier),
    ];
    let python_process = Command::new(python)
        .arg(reference)
        .args(common)
        .arg("--output")
        .arg(&python_output)
        .output()
        .expect("Python draft should run");
    let rust_process = Command::new(env!("CARGO_BIN_EXE_agents-work"))
        .args(common)
        .arg("--output")
        .arg(&rust_output)
        .output()
        .expect("Rust draft should run");

    assert_eq!(rust_process.status.code(), python_process.status.code());
    assert_eq!(rust_process.stderr, python_process.stderr);
    assert_eq!(
        normalize_generated(&fs::read(rust_output).expect("Rust draft should exist")),
        normalize_generated(&fs::read(python_output).expect("Python draft should exist"))
    );
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn draft_failure_diagnostics_match_the_python_reference() {
    let reference = std::env::var_os("AGENTS_WORK_PYTHON_REFERENCE")
        .expect("AGENTS_WORK_PYTHON_REFERENCE must name agents_work.py");
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| OsStr::new("python3").to_owned());

    let missing = TemporaryCase::new("oracle-missing-manifest");
    assert_draft_failure_parity(
        python.as_os_str(),
        reference.as_os_str(),
        &missing.case,
        &[],
    );

    let unreadable = TemporaryCase::new("oracle-unreadable-manifest");
    unreadable.write_manifest("not = [toml\n");
    assert_draft_failure_parity(
        python.as_os_str(),
        reference.as_os_str(),
        &unreadable.case,
        &[],
    );

    let invalid_topic = TemporaryCase::new("oracle-invalid-topic");
    invalid_topic.write_manifest(&manifest("Bad-ID"));
    assert_draft_failure_parity(
        python.as_os_str(),
        reference.as_os_str(),
        &invalid_topic.case,
        &[],
    );

    let missing_reference = TemporaryCase::new("oracle-missing-reference");
    missing_reference.write_manifest(&manifest("test-case"));
    assert_draft_failure_parity(
        python.as_os_str(),
        reference.as_os_str(),
        &missing_reference.case,
        &[OsStr::new("--responds-to"), OsStr::new("7")],
    );

    let unicode_zero = TemporaryCase::new("oracle-unicode-zero");
    unicode_zero.write_manifest(&manifest("test-case"));
    assert_draft_failure_parity(
        python.as_os_str(),
        reference.as_os_str(),
        &unicode_zero.case,
        &[OsStr::new("--responds-to"), OsStr::new("\u{116D0}")],
    );

    let ambiguous = TemporaryCase::new("oracle-ambiguous-reference");
    ambiguous.write_manifest(&manifest("test-case"));
    ambiguous.write_artifact("001-alpha-plan-claude-a1b2c3.md");
    ambiguous.write_artifact("001-zeta-review-codex-d4e5f6.md");
    assert_draft_failure_parity(
        python.as_os_str(),
        reference.as_os_str(),
        &ambiguous.case,
        &[OsStr::new("--responds-to"), OsStr::new("1")],
    );
}

fn assert_draft_failure_parity(python: &OsStr, reference: &OsStr, case: &Path, extra: &[&OsStr]) {
    let common = [
        OsStr::new("draft"),
        case.as_os_str(),
        OsStr::new("--kind"),
        OsStr::new("review"),
        OsStr::new("--author"),
        OsStr::new("claude"),
    ];
    let python_process = Command::new(python)
        .arg(reference)
        .args(common)
        .args(extra)
        .output()
        .expect("Python failure case should run");
    let rust_process = Command::new(env!("CARGO_BIN_EXE_agents-work"))
        .args(common)
        .args(extra)
        .output()
        .expect("Rust failure case should run");

    assert_eq!(rust_process.status.code(), python_process.status.code());
    assert_eq!(rust_process.stdout, python_process.stdout);
    assert_eq!(rust_process.stderr, python_process.stderr);
}

fn normalize_generated(data: &[u8]) -> String {
    let source = Path::new("draft.md");
    let table = parse_front_matter(data, source).expect("generated front matter should parse");
    let artifact_id = table["artifact_id"]
        .as_str()
        .expect("generated ID should be a string");
    let created_at = table["created_at"]
        .as_datetime()
        .expect("generated timestamp should be a TOML datetime")
        .to_string();
    String::from_utf8(data.to_owned())
        .expect("generated draft should be UTF-8")
        .replace(
            &format!("artifact_id = \"{artifact_id}\""),
            "artifact_id = \"<ID>\"",
        )
        .replace(&format!("created_at = {created_at}"), "created_at = <TIME>")
}
