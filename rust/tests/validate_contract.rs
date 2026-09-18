use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use agents_work::validate::{sha256_bytes, validate_case, validate_cases};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

struct TemporaryCase {
    root: PathBuf,
    case: PathBuf,
}

impl TemporaryCase {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "agents-work-validate-{}-{label}-{unique}",
            std::process::id()
        ));
        let case = root.join("case");
        fs::create_dir_all(&case).expect("temporary case should be created");
        Self { root, case }
    }

    fn write_manifest(&self, text: &str) {
        fs::write(self.case.join("work.toml"), text).expect("manifest should be written");
    }

    fn write_artifact(&self, name: &str, text: &str) -> String {
        fs::write(self.case.join(name), text).expect("artifact should be written");
        sha256_bytes(text.as_bytes())
    }

    fn write_sidecar(&self, name: &str, content: &str) {
        fs::write(self.case.join(format!("{name}.sha256")), content)
            .expect("sidecar should be written");
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

fn manifest(status: &str, next_agent: &str) -> String {
    format!(
        r#"schema_version = 2
phase = "planning"
status = "{status}"
next_agent = "{next_agent}"
"#
    )
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
+++
# Test artifact

Body.
"#
    )
}

fn run(command: &mut Command, arguments: &[&OsStr]) -> Output {
    command
        .args(arguments)
        .output()
        .expect("validator process should run")
}

fn assert_python_parity(cases: &[&Path]) {
    let reference = std::env::var_os("AGENTS_WORK_PYTHON_REFERENCE")
        .expect("AGENTS_WORK_PYTHON_REFERENCE must name agents_work.py");
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| OsStr::new("python3").to_owned());
    let mut rust_command = Command::new(env!("CARGO_BIN_EXE_agents-work"));
    rust_command.arg("validate");
    let mut python_command = Command::new(python);
    python_command.arg(reference).arg("validate");

    for case in cases {
        rust_command.arg(case);
        python_command.arg(case);
    }

    let rust = rust_command.output().expect("Rust validator should run");
    let python = python_command
        .output()
        .expect("Python validator should run");

    assert_eq!(rust.status.code(), python.status.code());
    assert_eq!(rust.stdout, python.stdout);
    assert_eq!(rust.stderr, python.stderr);
}

#[test]
fn sha256_matches_the_standard_known_vector() {
    assert_eq!(
        sha256_bytes(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn empty_schema_two_case_is_valid() {
    let fixture = TemporaryCase::new("empty-valid");
    fixture.write_manifest(&manifest("drafting", "codex"));

    let report = validate_case(&fixture.case).expect("case traversal should succeed");

    assert!(report.is_valid());
    assert_eq!(report.error_count(), 0);
    assert_eq!(report.case(), fixture.resolved());
    assert_eq!(
        report.stdout_text(),
        format!(
            "case: {}\ninventory:\n  (none)\nresult: valid (0 artifacts)\n",
            fixture.resolved().display()
        )
    );
    assert_eq!(report.stderr_text(), "");
}

#[test]
fn valid_artifact_is_hashed_and_rendered_in_inventory() {
    let fixture = TemporaryCase::new("artifact-valid");
    fixture.write_manifest(&manifest("drafting", "codex"));
    let name = "001-test-plan-codex-a1b2c3.md";
    let text = artifact(&[]);
    let digest = fixture.write_artifact(name, &text);
    fixture.write_sidecar(name, &format!("{digest}  {name}\n"));

    let report = validate_case(&fixture.case).expect("case traversal should succeed");

    assert!(report.is_valid());
    assert_eq!(
        report.stdout_text(),
        format!(
            "case: {}\ninventory:\n  {name}  plan  codex  {digest}\nresult: valid (1 artifacts)\n",
            fixture.resolved().display()
        )
    );
}

#[test]
fn invalid_case_retains_inventory_and_accumulates_errors_in_python_order() {
    let fixture = TemporaryCase::new("accumulated-errors");
    fixture.write_manifest(&manifest("awaiting_review", ""));
    let name = "001-test-plan-codex-a1b2c3.md";
    let text = "# No front matter\n";
    let digest = fixture.write_artifact(name, text);
    let orphan = "999-orphan-plan-codex-ffffff.md.sha256";
    fs::write(fixture.case.join(orphan), "orphan\n").expect("orphan should be written");

    let report = validate_case(&fixture.case).expect("case traversal should succeed");
    let manifest_path = fixture.resolved().join("work.toml");

    assert!(!report.is_valid());
    assert_eq!(report.error_count(), 4);
    assert_eq!(
        report.stdout_text(),
        format!(
            "case: {}\ninventory:\n  {name}  INVALID  {digest}\nresult: invalid (4 errors)\n",
            fixture.resolved().display()
        )
    );
    assert_eq!(
        report.stderr_text(),
        format!(
            concat!(
                "error: {}: active status requires next_agent\n",
                "error: {}: orphan sidecar\n",
                "error: {}: missing TOML front matter\n",
                "error: {}: missing sidecar\n"
            ),
            manifest_path.display(),
            orphan,
            name,
            name
        )
    );
}

#[test]
fn missing_relationship_target_invalidates_an_otherwise_valid_artifact() {
    let fixture = TemporaryCase::new("relationship-missing");
    fixture.write_manifest(&manifest("drafting", "codex"));
    let name = "001-test-plan-codex-a1b2c3.md";
    let missing = "002-missing-plan-claude-d4e5f6.md";
    let text = artifact(&[missing]);
    let digest = fixture.write_artifact(name, &text);
    fixture.write_sidecar(name, &format!("{digest}  {name}\n"));

    let report = validate_case(&fixture.case).expect("case traversal should succeed");

    assert_eq!(report.error_count(), 1);
    assert_eq!(
        report.stderr_text(),
        format!("error: {name}: responds_to target missing: {missing}\n")
    );
}

#[test]
fn malformed_relationship_containers_keep_python_second_pass_errors() {
    let fixture = TemporaryCase::new("malformed-relationship-containers");
    fixture.write_manifest(&manifest("drafting", "codex"));
    let name = "001-test-plan-codex-a1b2c3.md";
    let text = artifact(&[])
        .replace("responds_to = []", r#"responds_to = "cab""#)
        .replace("supersedes = []", "supersedes = { zz = 1, aa = 2 }");
    let digest = fixture.write_artifact(name, &text);
    fixture.write_sidecar(name, &format!("{digest}  {name}\n"));

    let report = validate_case(&fixture.case).expect("case traversal should succeed");

    assert_eq!(report.error_count(), 7);
    assert_eq!(
        report.stderr_text(),
        format!(
            concat!(
                "error: {}: responds_to must contain case-relative artifact names\n",
                "error: {}: supersedes must contain case-relative artifact names\n",
                "error: {}: responds_to target missing: c\n",
                "error: {}: responds_to target missing: a\n",
                "error: {}: responds_to target missing: b\n",
                "error: {}: supersedes target missing: zz\n",
                "error: {}: supersedes target missing: aa\n"
            ),
            name, name, name, name, name, name, name
        )
    );
}

#[test]
fn invalid_inventory_values_use_python_string_representations() {
    let composite = TemporaryCase::new("python-inventory-composite");
    composite.write_manifest(&manifest("drafting", "codex"));
    let name = "001-test-plan-codex-a1b2c3.md";
    let composite_text = artifact(&[])
        .replace(
            r#"kind = "plan""#,
            "kind = [1.0, -0.0, 0.00001, 0.0001, 1000000000000000.0, 1e16, inf, nan]",
        )
        .replace(
            r#"author = "codex""#,
            r#"author = { plain = "a", quoted = "don't", when = 2026-07-27T19:00:00.123456789+09:00 }"#,
        );
    let composite_digest = composite.write_artifact(name, &composite_text);
    composite.write_sidecar(name, &format!("{composite_digest}  {name}\n"));

    let composite_report =
        validate_case(&composite.case).expect("composite case traversal should succeed");

    assert!(composite_report.stdout_text().contains(&format!(
        concat!(
            "  {}  [1.0, -0.0, 1e-05, 0.0001, 1000000000000000.0, ",
            "1e+16, inf, nan]  {{'plain': 'a', 'quoted': \"don't\", 'when': ",
            "datetime.datetime(2026, 7, 27, 19, 0, 0, 123456, ",
            "tzinfo=datetime.timezone(datetime.timedelta(seconds=32400)))}}  ",
            "{}\n"
        ),
        name, composite_digest
    )));

    let temporal = TemporaryCase::new("python-inventory-temporal");
    temporal.write_manifest(&manifest("drafting", "codex"));
    let temporal_text = artifact(&[])
        .replace(r#"kind = "plan""#, "kind = 2026-07-27T19:00:00.123456789Z")
        .replace(r#"author = "codex""#, "author = 19:00:00.120000");
    let temporal_digest = temporal.write_artifact(name, &temporal_text);
    temporal.write_sidecar(name, &format!("{temporal_digest}  {name}\n"));

    let temporal_report =
        validate_case(&temporal.case).expect("temporal case traversal should succeed");

    assert!(temporal_report.stdout_text().contains(&format!(
        "  {name}  2026-07-27 19:00:00.123456+00:00  19:00:00.120000  {temporal_digest}\n"
    )));
}

#[test]
fn ascii_sidecar_uses_python_universal_newline_handling() {
    let fixture = TemporaryCase::new("sidecar-crlf");
    fixture.write_manifest(&manifest("drafting", "codex"));
    let name = "001-test-plan-codex-a1b2c3.md";
    let text = artifact(&[]);
    let digest = fixture.write_artifact(name, &text);
    fixture.write_sidecar(name, &format!("{digest}  {name}\r\n"));

    let report = validate_case(&fixture.case).expect("case traversal should succeed");

    assert!(report.is_valid());
}

#[test]
fn unpublished_draft_is_not_discovered() {
    let fixture = TemporaryCase::new("hidden-draft");
    fixture.write_manifest(&manifest("drafting", "codex"));
    fs::write(
        fixture.case.join(".draft-001-test-plan-codex-a1b2c3.md"),
        artifact(&[]),
    )
    .expect("draft should be written");

    let report = validate_case(&fixture.case).expect("case traversal should succeed");

    assert!(report.is_valid());
    assert!(report.stdout_text().contains("  (none)\n"));
}

#[test]
fn schema_one_absent_record_is_informational_only() {
    let fixture = TemporaryCase::new("legacy-info");
    fixture.write_manifest(
        r#"schema_version = 1
phase = "planning"
status = "deferred"
next_agent = ""
artifacts = []
"#,
    );
    let name = "001-test-plan-codex-a1b2c3.md";
    let text = artifact(&[]);
    let digest = fixture.write_artifact(name, &text);
    fixture.write_sidecar(name, &format!("{digest}  {name}\n"));

    let report = validate_case(&fixture.case).expect("case traversal should succeed");

    assert!(report.is_valid());
    assert!(report.stdout_text().contains(&format!(
        "info: discovered artifact absent from legacy manifest: {name}\n"
    )));
}

#[test]
fn schema_one_reports_malformed_missing_and_mismatched_records() {
    let fixture = TemporaryCase::new("legacy-errors");
    let name = "001-test-plan-codex-a1b2c3.md";
    fixture.write_manifest(&format!(
        r#"schema_version = 1
phase = "planning"
status = "deferred"
next_agent = ""
artifacts = [
  "not a table",
  {{ path = "../bad.md", sha256 = "ignored" }},
  {{ path = "missing.md", sha256 = "ignored" }},
  {{ path = "{name}", sha256 = "wrong" }},
]
"#
    ));
    let text = artifact(&[]);
    let digest = fixture.write_artifact(name, &text);
    fixture.write_sidecar(name, &format!("{digest}  {name}\n"));

    let report = validate_case(&fixture.case).expect("case traversal should succeed");
    let label = fixture.resolved().join("work.toml");

    assert_eq!(report.error_count(), 4);
    assert_eq!(
        report.stderr_text(),
        format!(
            concat!(
                "error: {}: malformed artifact record\n",
                "error: {}: invalid legacy artifact path\n",
                "error: {}: listed artifact missing: missing.md\n",
                "error: {}: legacy hash mismatch: {}\n"
            ),
            label.display(),
            label.display(),
            label.display(),
            label.display(),
            name
        )
    );
}

#[test]
fn multiple_cases_continue_after_validation_errors() {
    let valid = TemporaryCase::new("multiple-valid");
    valid.write_manifest(&manifest("drafting", "codex"));
    let invalid = TemporaryCase::new("multiple-invalid");

    let mut standard_output = Vec::new();
    let mut standard_error = Vec::new();
    let all_valid = validate_cases(
        &[valid.case.clone(), invalid.case.clone()],
        &mut standard_output,
        &mut standard_error,
    )
    .expect("both existing directories should be traversed");

    assert!(!all_valid);
    assert_eq!(
        String::from_utf8(standard_output).expect("stdout should be UTF-8"),
        format!(
            concat!(
                "case: {}\ninventory:\n  (none)\nresult: valid (0 artifacts)\n",
                "case: {}\ninventory:\n  (none)\nresult: invalid (1 errors)\n"
            ),
            valid.resolved().display(),
            invalid.resolved().display()
        )
    );
    assert_eq!(
        String::from_utf8(standard_error).expect("stderr should be UTF-8"),
        format!(
            "error: {}: missing work.toml\n",
            invalid.resolved().join("work.toml").display()
        )
    );
}

#[test]
fn validate_command_uses_validation_exit_status() {
    let valid = TemporaryCase::new("binary-valid");
    valid.write_manifest(&manifest("drafting", "codex"));
    let invalid = TemporaryCase::new("binary-invalid");
    let binary = env!("CARGO_BIN_EXE_agents-work");

    let valid_output = run(
        &mut Command::new(binary),
        &[OsStr::new("validate"), valid.case.as_os_str()],
    );
    let invalid_output = run(
        &mut Command::new(binary),
        &[OsStr::new("validate"), invalid.case.as_os_str()],
    );

    assert_eq!(valid_output.status.code(), Some(0));
    assert_eq!(invalid_output.status.code(), Some(1));
    assert!(valid_output.stderr.is_empty());
    assert!(
        String::from_utf8(invalid_output.stderr)
            .expect("stderr should be UTF-8")
            .contains("missing work.toml")
    );
}

fn inventory_oracle_cases(name: &str) -> [TemporaryCase; 4] {
    let invalid_inventory = TemporaryCase::new("oracle-invalid-inventory");
    invalid_inventory.write_manifest(&manifest("drafting", "codex"));
    let invalid_inventory_text = artifact(&[])
        .replace(
            r#"kind = "plan""#,
            "kind = [1.0, -0.0, 0.00001, 0.0001, 1000000000000000.0, 1e16, inf, nan]",
        )
        .replace(
            r#"author = "codex""#,
            r#"author = { plain = "a", quoted = "don't", when = 2026-07-27T19:00:00.123456789+09:00 }"#,
        );
    let invalid_inventory_digest = invalid_inventory.write_artifact(name, &invalid_inventory_text);
    invalid_inventory.write_sidecar(name, &format!("{invalid_inventory_digest}  {name}\n"));

    let temporal_inventory = TemporaryCase::new("oracle-temporal-inventory");
    temporal_inventory.write_manifest(&manifest("drafting", "codex"));
    let temporal_inventory_text = artifact(&[])
        .replace(r#"kind = "plan""#, "kind = 2026-07-27T19:00:00.123456789Z")
        .replace(r#"author = "codex""#, "author = 19:00:00.120000");
    let temporal_inventory_digest =
        temporal_inventory.write_artifact(name, &temporal_inventory_text);
    temporal_inventory.write_sidecar(name, &format!("{temporal_inventory_digest}  {name}\n"));

    let float_inventory = TemporaryCase::new("oracle-float-inventory");
    float_inventory.write_manifest(&manifest("drafting", "codex"));
    let mut bits = 0x4d59_5df4_d0f3_3173_u64;
    let mut float_literals = Vec::new();

    for _ in 0..256 {
        bits = bits
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let value = f64::from_bits(bits);

        if value.is_finite() {
            float_literals.push(format!("{value:?}"));
        }
    }

    let float_inventory_text = artifact(&[]).replace(
        r#"kind = "plan""#,
        &format!("kind = [{}]", float_literals.join(", ")),
    );
    let float_inventory_digest = float_inventory.write_artifact(name, &float_inventory_text);
    float_inventory.write_sidecar(name, &format!("{float_inventory_digest}  {name}\n"));

    let string_inventory = TemporaryCase::new("oracle-string-inventory");
    string_inventory.write_manifest(&manifest("drafting", "codex"));
    let string_inventory_text = artifact(&[]).replace(
        r#"kind = "plan""#,
        r#"kind = ["plain", "don't", "say \"don't\"", "\b\f\t\n\r", "\u00A0\u200B\uE000"]"#,
    );
    let string_inventory_digest = string_inventory.write_artifact(name, &string_inventory_text);
    string_inventory.write_sidecar(name, &format!("{string_inventory_digest}  {name}\n"));

    [
        invalid_inventory,
        temporal_inventory,
        float_inventory,
        string_inventory,
    ]
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn validate_cli_matches_the_python_reference() {
    let valid = TemporaryCase::new("oracle-valid");
    valid.write_manifest(&manifest("drafting", "codex"));
    let name = "001-test-plan-codex-a1b2c3.md";
    let text = artifact(&[]);
    let digest = valid.write_artifact(name, &text);
    valid.write_sidecar(name, &format!("{digest}  {name}\n"));

    let invalid = TemporaryCase::new("oracle-invalid");
    invalid.write_manifest(&manifest("awaiting_review", ""));
    let orphan = "999-orphan-plan-codex-ffffff.md.sha256";
    fs::write(invalid.case.join(orphan), "orphan\n").expect("orphan should be written");

    let missing_manifest = TemporaryCase::new("oracle-missing-manifest");

    let relationship = TemporaryCase::new("oracle-relationship");
    relationship.write_manifest(&manifest("drafting", "codex"));
    let missing = "002-missing-plan-claude-d4e5f6.md";
    relationship.write_artifact(name, &artifact(&[missing]));

    let malformed_relationships = TemporaryCase::new("oracle-malformed-relationships");
    malformed_relationships.write_manifest(&manifest("drafting", "codex"));
    let malformed_relationship_text = artifact(&[])
        .replace("responds_to = []", r#"responds_to = "cab""#)
        .replace("supersedes = []", "supersedes = { zz = 1, aa = 2 }");
    let malformed_relationship_digest =
        malformed_relationships.write_artifact(name, &malformed_relationship_text);
    malformed_relationships
        .write_sidecar(name, &format!("{malformed_relationship_digest}  {name}\n"));

    let [
        invalid_inventory,
        temporal_inventory,
        float_inventory,
        string_inventory,
    ] = inventory_oracle_cases(name);

    let invalid_metadata = TemporaryCase::new("oracle-invalid-metadata");
    invalid_metadata.write_manifest(&manifest("drafting", "codex"));
    let invalid_text = artifact(&[]).replace("sequence = 1", "sequence = 0");
    let invalid_digest = invalid_metadata.write_artifact(name, &invalid_text);
    invalid_metadata.write_sidecar(name, &format!("{invalid_digest}  {name}\n"));

    let filename_mismatch = TemporaryCase::new("oracle-filename-mismatch");
    filename_mismatch.write_manifest(&manifest("drafting", "codex"));
    let wrong_name = "001-other-plan-codex-a1b2c3.md";
    let wrong_name_text = artifact(&[]);
    let wrong_name_digest = filename_mismatch.write_artifact(wrong_name, &wrong_name_text);
    filename_mismatch.write_sidecar(wrong_name, &format!("{wrong_name_digest}  {wrong_name}\n"));

    let hash_mismatch = TemporaryCase::new("oracle-hash-mismatch");
    hash_mismatch.write_manifest(&manifest("drafting", "codex"));
    hash_mismatch.write_artifact(name, &artifact(&[]));
    hash_mismatch.write_sidecar(name, &format!("{}  {name}\n", "0".repeat(64)));

    let unreadable_sidecar = TemporaryCase::new("oracle-unreadable-sidecar");
    unreadable_sidecar.write_manifest(&manifest("drafting", "codex"));
    unreadable_sidecar.write_artifact(name, &artifact(&[]));
    fs::write(
        unreadable_sidecar.case.join(format!("{name}.sha256")),
        [0xff],
    )
    .expect("non-ASCII sidecar should be written");

    let schema_two_legacy = TemporaryCase::new("oracle-schema-two-legacy");
    schema_two_legacy.write_manifest(&format!("{}artifacts = []\n", manifest("deferred", "")));

    let schema_one_info = TemporaryCase::new("oracle-schema-one-info");
    schema_one_info.write_manifest(
        r#"schema_version = 1
phase = "planning"
status = "deferred"
next_agent = ""
artifacts = []
"#,
    );
    let schema_one_text = artifact(&[]);
    let schema_one_digest = schema_one_info.write_artifact(name, &schema_one_text);
    schema_one_info.write_sidecar(name, &format!("{schema_one_digest}  {name}\n"));

    assert_python_parity(&[
        &valid.case,
        &invalid.case,
        &missing_manifest.case,
        &relationship.case,
        &malformed_relationships.case,
        &invalid_inventory.case,
        &temporal_inventory.case,
        &float_inventory.case,
        &string_inventory.case,
        &invalid_metadata.case,
        &filename_mismatch.case,
        &hash_mismatch.case,
        &unreadable_sidecar.case,
        &schema_two_legacy.case,
        &schema_one_info.case,
    ]);
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn coordination_matrix_matches_the_python_reference() {
    let phases = ["planning", "implementation", "pr_review", "complete"];
    let statuses = [
        "drafting",
        "awaiting_review",
        "revision_requested",
        "ready_for_implementation",
        "awaiting_decision",
        "deferred",
        "complete",
    ];
    let next_agents = ["", "codex"];
    let mut fixtures = Vec::new();

    for (phase_index, phase) in phases.iter().enumerate() {
        for (status_index, status) in statuses.iter().enumerate() {
            for (agent_index, next_agent) in next_agents.iter().enumerate() {
                let fixture = TemporaryCase::new(&format!(
                    "oracle-coordination-{phase_index}-{status_index}-{agent_index}"
                ));
                fixture.write_manifest(&format!(
                    r#"schema_version = 2
phase = "{phase}"
status = "{status}"
next_agent = "{next_agent}"
"#
                ));
                fixtures.push(fixture);
            }
        }
    }

    let cases = fixtures
        .iter()
        .map(|fixture| fixture.case.as_path())
        .collect::<Vec<_>>();
    assert_python_parity(&cases);
}

#[test]
fn validation_does_not_modify_case_files() {
    let fixture = TemporaryCase::new("read-only");
    fixture.write_manifest(&manifest("drafting", "codex"));
    let name = "001-test-plan-codex-a1b2c3.md";
    let text = artifact(&[]);
    let digest = fixture.write_artifact(name, &text);
    fixture.write_sidecar(name, &format!("{digest}  {name}\n"));
    let before = directory_bytes(&fixture.case);

    validate_case(&fixture.case).expect("case traversal should succeed");

    assert_eq!(directory_bytes(&fixture.case), before);
}

fn directory_bytes(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = fs::read_dir(directory)
        .expect("directory should be readable")
        .map(|entry| {
            let path = entry.expect("entry should be readable").path();
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
