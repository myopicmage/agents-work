use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use agents_work::artifact::OffsetDateTime;
use agents_work::cursor::{CursorDocument, CursorRequest, CursorUpdates, cursor};
use agents_work::manifest::{Phase, Status};
use agents_work::validate::validate_case;
use toml::{Table, Value};

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

struct TemporaryCase {
    root: PathBuf,
    case: PathBuf,
}

impl TemporaryCase {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "agents-work-cursor-{}-{label}-{unique}",
            std::process::id()
        ));
        let case = root.join("case");
        fs::create_dir_all(&case).expect("temporary case should be created");
        Self { root, case }
    }

    fn write_manifest(&self, status: &str, next_agent: &str) {
        fs::write(
            self.case.join("work.toml"),
            manifest_text(status, next_agent),
        )
        .expect("manifest should be written");
    }

    fn read_manifest(&self) -> Table {
        fs::read_to_string(self.case.join("work.toml"))
            .expect("manifest should be readable")
            .parse()
            .expect("manifest should remain TOML")
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

fn manifest(status: &str, next_agent: &str) -> Table {
    manifest_text(status, next_agent)
        .parse()
        .expect("fixture manifest should parse")
}

fn manifest_text(status: &str, next_agent: &str) -> String {
    format!(
        r#"schema_version = 2
id = "test-case"
title = "Test case"
repository_name = "repo"
repository_path = "/tmp/repo"
phase = "planning"
status = "{status}"
next_agent = "{next_agent}"
requested_action = "Old action."
implementation_branch = ""
pull_request_system = ""
pull_request_id = ""
reviewed_commit = ""
created_at = "2026-07-27T19:00:00+09:00"
updated_at = "2026-07-27T19:00:00+09:00"
"#
    )
}

fn timestamp() -> &'static str {
    "2026-08-10T09:15:30+09:00"
}

#[test]
fn typed_updates_render_exact_python_manifest_bytes() {
    let mut table = manifest("drafting", "codex");
    table.insert("zeta".to_owned(), Value::Integer(7));
    table.insert("enabled".to_owned(), Value::Boolean(true));
    table.insert(
        "custom_note".to_owned(),
        Value::String("café\n\u{7f}".to_owned()),
    );
    let document = CursorDocument::build(
        table,
        CursorUpdates {
            phase: Some(Phase::PrReview),
            status: Some(Status::AwaitingReview),
            next_agent: Some("claude"),
            requested_action: Some("Review plan 030."),
            implementation_branch: Some("feature/cursor"),
            pull_request_system: Some("github"),
            pull_request_id: Some("42"),
            reviewed_commit: Some("abc123"),
        },
        timestamp(),
        "work.toml",
    )
    .expect("legal update should render");

    assert_eq!(
        document.body(),
        concat!(
            "schema_version = 2\n",
            "id = \"test-case\"\n",
            "title = \"Test case\"\n",
            "\n",
            "repository_name = \"repo\"\n",
            "repository_path = \"/tmp/repo\"\n",
            "\n",
            "phase = \"pr_review\"\n",
            "status = \"awaiting_review\"\n",
            "next_agent = \"claude\"\n",
            "requested_action = \"Review plan 030.\"\n",
            "\n",
            "implementation_branch = \"feature/cursor\"\n",
            "pull_request_system = \"github\"\n",
            "pull_request_id = \"42\"\n",
            "reviewed_commit = \"abc123\"\n",
            "\n",
            "created_at = \"2026-07-27T19:00:00+09:00\"\n",
            "updated_at = \"2026-08-10T09:15:30+09:00\"\n",
            "\n",
            "custom_note = \"caf\\u00e9\\n\\u007f\"\n",
            "enabled = true\n",
            "zeta = 7\n",
        )
    );
    assert_eq!(document.manifest()["phase"].as_str(), Some("pr_review"));
    assert_eq!(
        document.manifest()["custom_note"].as_str(),
        Some("café\n\u{7f}")
    );
}

#[test]
fn non_bmp_strings_render_as_toml_scalars() {
    let mut table = manifest("drafting", "codex");
    table.insert("custom_note".to_owned(), Value::String("😀".to_owned()));

    let document = CursorDocument::build(
        table,
        CursorUpdates {
            requested_action: Some("Ship it 🚀"),
            ..CursorUpdates::default()
        },
        timestamp(),
        "work.toml",
    )
    .expect("non-BMP Unicode scalars should round-trip through TOML");

    assert!(
        document
            .body()
            .contains("requested_action = \"Ship it 🚀\"\n")
    );
    assert!(document.body().contains("custom_note = \"😀\"\n"));
    assert_eq!(
        document.manifest()["requested_action"].as_str(),
        Some("Ship it 🚀")
    );
    assert_eq!(document.manifest()["custom_note"].as_str(), Some("😀"));
}

#[test]
fn typed_update_repairs_an_invalid_existing_status() {
    let table = manifest("in_progress", "codex");

    let document = CursorDocument::build(
        table,
        CursorUpdates {
            status: Some(Status::RevisionRequested),
            ..CursorUpdates::default()
        },
        timestamp(),
        "work.toml",
    )
    .expect("valid typed update should repair invalid old status");

    assert_eq!(
        document.manifest()["status"].as_str(),
        Some("revision_requested")
    );
    assert_eq!(document.manifest()["next_agent"].as_str(), Some("codex"));
}

#[test]
fn resting_status_without_an_explicit_owner_clears_stale_ownership() {
    for updates in [
        CursorUpdates {
            status: Some(Status::Deferred),
            ..CursorUpdates::default()
        },
        CursorUpdates {
            requested_action: Some("Park this."),
            ..CursorUpdates::default()
        },
    ] {
        let starting_status = if updates.status.is_some() {
            "drafting"
        } else {
            "deferred"
        };
        let document = CursorDocument::build(
            manifest(starting_status, "codex"),
            updates,
            timestamp(),
            "work.toml",
        )
        .expect("resting cursor should be legal");

        assert_eq!(document.manifest()["next_agent"].as_str(), Some(""));
    }
}

#[test]
fn resting_status_keeps_an_explicit_resumption_owner() {
    let document = CursorDocument::build(
        manifest("drafting", "codex"),
        CursorUpdates {
            status: Some(Status::ReadyForImplementation),
            next_agent: Some("claude"),
            ..CursorUpdates::default()
        },
        timestamp(),
        "work.toml",
    )
    .expect("resting cursor may name an explicit owner");

    assert_eq!(document.manifest()["next_agent"].as_str(), Some("claude"));
}

#[test]
fn transition_validation_matches_python_error_order() {
    let empty = CursorDocument::build(
        manifest("drafting", "codex"),
        CursorUpdates::default(),
        timestamp(),
        "work.toml",
    )
    .expect_err("empty updates should fail");
    assert_eq!(
        empty.to_string(),
        "cursor requires at least one field to set"
    );

    let legacy = CursorDocument::build(
        "schema_version = 1\n"
            .parse()
            .expect("legacy fixture should parse"),
        CursorUpdates {
            status: Some(Status::Drafting),
            ..CursorUpdates::default()
        },
        timestamp(),
        "work.toml",
    )
    .expect_err("legacy manifest should fail");
    assert_eq!(
        legacy.to_string(),
        "work.toml: cursor requires schema_version 2; migrate a legacy manifest by hand first"
    );

    let illegal = CursorDocument::build(
        manifest("drafting", "codex"),
        CursorUpdates {
            phase: Some(Phase::Complete),
            status: Some(Status::Complete),
            next_agent: Some("codex"),
            ..CursorUpdates::default()
        },
        timestamp(),
        "work.toml",
    )
    .expect_err("complete status cannot retain an owner");
    assert_eq!(
        illegal.to_string(),
        "work.toml: complete status requires empty next_agent"
    );
}

#[test]
fn unsupported_manifest_values_use_python_type_names() {
    for (value, type_name) in [
        (Value::Float(2.5), "float"),
        (Value::Array(Vec::new()), "list"),
        (Value::Table(Table::new()), "dict"),
        (
            Value::Datetime("2026-08-10".parse().expect("date should parse")),
            "date",
        ),
        (
            Value::Datetime("09:15:30".parse().expect("time should parse")),
            "time",
        ),
        (
            Value::Datetime(
                "2026-08-10T09:15:30+09:00"
                    .parse()
                    .expect("datetime should parse"),
            ),
            "datetime",
        ),
    ] {
        let mut table = manifest("drafting", "codex");
        table.insert("custom".to_owned(), value);

        let error = CursorDocument::build(
            table,
            CursorUpdates {
                requested_action: Some("Keep moving."),
                ..CursorUpdates::default()
            },
            timestamp(),
            "work.toml",
        )
        .expect_err("unsupported value should not be silently discarded");

        assert_eq!(
            error.to_string(),
            format!("cannot render manifest field custom of type {type_name}")
        );
    }
}

#[test]
fn numeric_schema_equality_precedes_renderer_type_rejection() {
    let mut table = manifest("drafting", "codex");
    table.insert("schema_version".to_owned(), Value::Float(2.0));

    let error = CursorDocument::build(
        table,
        CursorUpdates {
            requested_action: Some("Keep moving."),
            ..CursorUpdates::default()
        },
        timestamp(),
        "work.toml",
    )
    .expect_err("Python accepts 2.0 as schema two, then cannot render it");

    assert_eq!(
        error.to_string(),
        "cannot render manifest field schema_version of type float"
    );
}

#[test]
fn cursor_atomically_rewrites_a_valid_manifest_and_reports_the_new_state() {
    let fixture = TemporaryCase::new("success");
    fixture.write_manifest("drafting", "codex");
    fs::write(fixture.case.join("extra.txt"), "not part of the cursor\n")
        .expect("unrelated file should be written");
    let mut standard_output = Vec::new();

    let manifest_path = cursor(
        CursorRequest {
            case: &fixture.case,
            updates: CursorUpdates {
                status: Some(Status::AwaitingReview),
                next_agent: Some("claude"),
                requested_action: Some("Review plan 030."),
                ..CursorUpdates::default()
            },
        },
        &mut standard_output,
    )
    .expect("legal cursor should be written");
    let expected_path = fixture.resolved().join("work.toml");
    let table = fixture.read_manifest();
    let updated_at = table["updated_at"]
        .as_str()
        .expect("updated_at should remain a string");

    assert_eq!(manifest_path, expected_path);
    assert_eq!(
        String::from_utf8(standard_output).expect("stdout should be UTF-8"),
        format!(
            "{}: phase=planning status=awaiting_review next_agent=claude\n",
            expected_path.display()
        )
    );
    assert_eq!(table["status"].as_str(), Some("awaiting_review"));
    assert_eq!(table["next_agent"].as_str(), Some("claude"));
    assert_eq!(table["id"].as_str(), Some("test-case"));
    assert_eq!(table["title"].as_str(), Some("Test case"));
    assert_eq!(
        table["created_at"].as_str(),
        Some("2026-07-27T19:00:00+09:00")
    );
    assert_eq!(table["requested_action"].as_str(), Some("Review plan 030."));
    assert_ne!(updated_at, "2026-07-27T19:00:00+09:00");
    updated_at
        .parse::<OffsetDateTime>()
        .expect("updated_at should include a UTC offset");
    assert_eq!(
        fs::read_to_string(fixture.case.join("extra.txt")).expect("unrelated file should remain"),
        "not part of the cursor\n"
    );
    assert_no_temporary_work_file(&fixture.case);
    assert!(
        validate_case(&fixture.case)
            .expect("updated case should be traversable")
            .is_valid()
    );

    #[cfg(unix)]
    assert_eq!(
        fs::metadata(manifest_path)
            .expect("manifest metadata should be readable")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}

#[test]
fn missing_manifest_precedes_the_empty_update_error() {
    let fixture = TemporaryCase::new("missing-manifest");
    let request = CursorRequest {
        case: &fixture.case,
        updates: CursorUpdates::default(),
    };

    let missing = cursor(request, &mut Vec::new()).expect_err("manifest check should run first");
    assert_eq!(
        missing.to_string(),
        format!("{}: missing work.toml", fixture.resolved().display())
    );

    fixture.write_manifest("drafting", "codex");
    let empty = cursor(request, &mut Vec::new()).expect_err("empty update should then fail");
    assert_eq!(
        empty.to_string(),
        "cursor requires at least one field to set"
    );
}

#[test]
fn production_cursor_repairs_an_illegal_existing_status() {
    let fixture = TemporaryCase::new("repair");
    fixture.write_manifest("in_progress", "codex");

    cursor(
        CursorRequest {
            case: &fixture.case,
            updates: CursorUpdates {
                status: Some(Status::RevisionRequested),
                ..CursorUpdates::default()
            },
        },
        &mut Vec::new(),
    )
    .expect("cursor should read past the illegal old status");

    assert_eq!(
        fixture.read_manifest()["status"].as_str(),
        Some("revision_requested")
    );
    assert!(
        validate_case(&fixture.case)
            .expect("repaired case should be traversable")
            .is_valid()
    );
}

#[test]
fn cursor_reads_universal_newlines_and_rewrites_canonical_lf() {
    let fixture = TemporaryCase::new("crlf");
    let text = manifest_text("drafting", "codex").replace('\n', "\r\n");
    fs::write(fixture.case.join("work.toml"), text).expect("CRLF manifest should be written");

    cursor(
        CursorRequest {
            case: &fixture.case,
            updates: CursorUpdates {
                requested_action: Some("Normalize this."),
                ..CursorUpdates::default()
            },
        },
        &mut Vec::new(),
    )
    .expect("Python text-mode newline behavior should be preserved");

    assert!(
        !fs::read(fixture.case.join("work.toml"))
            .expect("rewritten manifest should be readable")
            .contains(&b'\r')
    );
}

#[test]
fn malformed_toml_does_not_modify_the_manifest() {
    let fixture = TemporaryCase::new("malformed-toml");
    let manifest_path = fixture.case.join("work.toml");
    let before = b"not = [toml\n";
    fs::write(&manifest_path, before).expect("malformed manifest should be written");

    let error = cursor(
        CursorRequest {
            case: &fixture.case,
            updates: CursorUpdates {
                requested_action: Some("Cannot land."),
                ..CursorUpdates::default()
            },
        },
        &mut Vec::new(),
    )
    .expect_err("malformed TOML should fail before mutation");

    assert!(error.to_string().starts_with(&format!(
        "{}: invalid work.toml:",
        fixture.resolved().join("work.toml").display()
    )));
    assert_eq!(
        fs::read(manifest_path).expect("malformed manifest should remain"),
        before
    );
    assert_no_temporary_work_file(&fixture.case);
}

#[test]
fn rejected_transition_does_not_modify_the_manifest() {
    let fixture = TemporaryCase::new("rejected-transition");
    fixture.write_manifest("drafting", "codex");
    let before = fs::read(fixture.case.join("work.toml")).expect("manifest should be readable");

    let error = cursor(
        CursorRequest {
            case: &fixture.case,
            updates: CursorUpdates {
                phase: Some(Phase::Complete),
                status: Some(Status::Complete),
                next_agent: Some("codex"),
                ..CursorUpdates::default()
            },
        },
        &mut Vec::new(),
    )
    .expect_err("illegal complete cursor should fail");

    assert_eq!(
        error.to_string(),
        format!(
            "{}: complete status requires empty next_agent",
            fixture.resolved().join("work.toml").display()
        )
    );
    assert_eq!(
        fs::read(fixture.case.join("work.toml")).expect("manifest should remain readable"),
        before
    );
    assert_no_temporary_work_file(&fixture.case);
}

#[cfg(unix)]
#[test]
fn cursor_replaces_the_manifest_symlink_instead_of_its_target() {
    use std::os::unix::fs::symlink;

    let fixture = TemporaryCase::new("manifest-symlink");
    let external = fixture.root.join("external.toml");
    let original = manifest_text("drafting", "codex");
    fs::write(&external, &original).expect("external manifest should be written");
    symlink(&external, fixture.case.join("work.toml")).expect("manifest symlink should be created");

    cursor(
        CursorRequest {
            case: &fixture.case,
            updates: CursorUpdates {
                status: Some(Status::Deferred),
                ..CursorUpdates::default()
            },
        },
        &mut Vec::new(),
    )
    .expect("cursor should replace the symlink entry");

    assert!(
        !fs::symlink_metadata(fixture.case.join("work.toml"))
            .expect("replacement metadata should exist")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(external).expect("external target should remain"),
        original
    );
    assert_eq!(fixture.read_manifest()["status"].as_str(), Some("deferred"));
}

#[test]
fn cursor_command_routes_all_writable_fields() {
    let fixture = TemporaryCase::new("cli");
    fixture.write_manifest("drafting", "codex");
    let process = Command::new(env!("CARGO_BIN_EXE_agents-work"))
        .args([
            OsStr::new("cursor"),
            fixture.case.as_os_str(),
            OsStr::new("--phase"),
            OsStr::new("pr_review"),
            OsStr::new("--status"),
            OsStr::new("awaiting_review"),
            OsStr::new("--next-agent"),
            OsStr::new("claude"),
            OsStr::new("--action"),
            OsStr::new("Review this. 🚀"),
            OsStr::new("--implementation-branch"),
            OsStr::new("feature/cursor"),
            OsStr::new("--pr-system"),
            OsStr::new("github"),
            OsStr::new("--pr-id"),
            OsStr::new("42"),
            OsStr::new("--reviewed-commit"),
            OsStr::new("abc123"),
        ])
        .output()
        .expect("cursor process should run");
    let table = fixture.read_manifest();

    assert_eq!(process.status.code(), Some(0));
    assert!(process.stderr.is_empty());
    assert_eq!(table["phase"].as_str(), Some("pr_review"));
    assert_eq!(table["status"].as_str(), Some("awaiting_review"));
    assert_eq!(table["next_agent"].as_str(), Some("claude"));
    assert_eq!(table["requested_action"].as_str(), Some("Review this. 🚀"));
    assert_eq!(
        table["implementation_branch"].as_str(),
        Some("feature/cursor")
    );
    assert_eq!(table["pull_request_system"].as_str(), Some("github"));
    assert_eq!(table["pull_request_id"].as_str(), Some("42"));
    assert_eq!(table["reviewed_commit"].as_str(), Some("abc123"));
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn cursor_cli_matches_python_except_documented_non_bmp_divergence() {
    assert_cursor_success_parity(
        "all-fields",
        "drafting",
        "codex",
        &[
            OsStr::new("--phase"),
            OsStr::new("pr_review"),
            OsStr::new("--status"),
            OsStr::new("awaiting_review"),
            OsStr::new("--next-agent"),
            OsStr::new("claude"),
            OsStr::new("--action"),
            OsStr::new("Review café."),
            OsStr::new("--implementation-branch"),
            OsStr::new("feature/cursor"),
            OsStr::new("--pr-system"),
            OsStr::new("github"),
            OsStr::new("--pr-id"),
            OsStr::new("42"),
            OsStr::new("--reviewed-commit"),
            OsStr::new("abc123"),
        ],
    );
    assert_cursor_success_parity(
        "repair",
        "in_progress",
        "codex",
        &[OsStr::new("--status"), OsStr::new("revision_requested")],
    );
    assert_cursor_success_parity(
        "resting-clear",
        "drafting",
        "codex",
        &[OsStr::new("--status"), OsStr::new("deferred")],
    );
    assert_non_bmp_cursor_divergence();
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn cursor_failure_diagnostics_and_no_write_state_match_python() {
    for scenario in [
        FailureScenario::MissingManifest,
        FailureScenario::EmptyUpdate,
        FailureScenario::LegacySchema,
        FailureScenario::ActiveWithoutOwner,
        FailureScenario::CompleteWithOwner,
        FailureScenario::InvalidExistingStatus,
        FailureScenario::UnsupportedList,
        FailureScenario::NumericSchema,
    ] {
        assert_cursor_failure_parity(scenario);
    }
}

#[derive(Clone, Copy)]
enum FailureScenario {
    MissingManifest,
    EmptyUpdate,
    LegacySchema,
    ActiveWithoutOwner,
    CompleteWithOwner,
    InvalidExistingStatus,
    UnsupportedList,
    NumericSchema,
}

fn assert_cursor_success_parity(label: &str, status: &str, next_agent: &str, extra: &[&OsStr]) {
    let python_fixture = TemporaryCase::new(&format!("oracle-python-{label}"));
    let rust_fixture = TemporaryCase::new(&format!("oracle-rust-{label}"));
    write_oracle_manifest(&python_fixture, status, next_agent);
    write_oracle_manifest(&rust_fixture, status, next_agent);

    let python = run_python_cursor(&python_fixture.case, extra);
    let rust = run_rust_cursor(&rust_fixture.case, extra);

    assert_eq!(rust.status.code(), python.status.code());
    assert_eq!(rust.stderr, python.stderr);
    assert_eq!(
        normalize_case(&rust.stdout, &rust_fixture.resolved()),
        normalize_case(&python.stdout, &python_fixture.resolved())
    );
    assert_eq!(
        normalized_manifest(&rust_fixture),
        normalized_manifest(&python_fixture)
    );
    assert_no_temporary_work_file(&rust_fixture.case);
    assert_no_temporary_work_file(&python_fixture.case);

    #[cfg(unix)]
    assert_eq!(
        fs::metadata(rust_fixture.case.join("work.toml"))
            .expect("Rust manifest metadata should be readable")
            .permissions()
            .mode()
            & 0o777,
        fs::metadata(python_fixture.case.join("work.toml"))
            .expect("Python manifest metadata should be readable")
            .permissions()
            .mode()
            & 0o777
    );
}

fn assert_non_bmp_cursor_divergence() {
    let python_fixture = TemporaryCase::new("oracle-python-non-bmp");
    let rust_fixture = TemporaryCase::new("oracle-rust-non-bmp");
    write_oracle_manifest(&python_fixture, "drafting", "codex");
    write_oracle_manifest(&rust_fixture, "drafting", "codex");
    let python_before = directory_bytes(&python_fixture.case);
    let extra = [OsStr::new("--action"), OsStr::new("Ship it 🚀")];

    let python = run_python_cursor(&python_fixture.case, &extra);
    let rust = run_rust_cursor(&rust_fixture.case, &extra);

    assert_eq!(python.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&python.stderr).contains("TOMLDecodeError"));
    assert_eq!(directory_bytes(&python_fixture.case), python_before);
    assert_eq!(rust.status.code(), Some(0));
    assert!(rust.stderr.is_empty());
    assert_eq!(
        rust_fixture.read_manifest()["requested_action"].as_str(),
        Some("Ship it 🚀")
    );
    assert_no_temporary_work_file(&python_fixture.case);
    assert_no_temporary_work_file(&rust_fixture.case);
}

fn write_oracle_manifest(fixture: &TemporaryCase, status: &str, next_agent: &str) {
    let mut text = manifest_text(status, next_agent);
    text.push_str("custom_note = \"café\"\ncustom_number = 7\n");
    fs::write(fixture.case.join("work.toml"), text).expect("oracle manifest should be written");
}

fn assert_cursor_failure_parity(scenario: FailureScenario) {
    let python_fixture = TemporaryCase::new("oracle-python-failure");
    let rust_fixture = TemporaryCase::new("oracle-rust-failure");
    let extra = prepare_failure(&python_fixture, scenario);
    prepare_failure(&rust_fixture, scenario);
    let python_before = directory_bytes(&python_fixture.case);
    let rust_before = directory_bytes(&rust_fixture.case);

    let python = run_python_cursor(&python_fixture.case, &extra);
    let rust = run_rust_cursor(&rust_fixture.case, &extra);

    assert_eq!(rust.status.code(), python.status.code());
    assert_eq!(rust.stdout, python.stdout);
    assert_eq!(
        normalize_case(&rust.stderr, &rust_fixture.resolved()),
        normalize_case(&python.stderr, &python_fixture.resolved())
    );
    assert_eq!(directory_bytes(&rust_fixture.case), rust_before);
    assert_eq!(directory_bytes(&python_fixture.case), python_before);
    assert_no_temporary_work_file(&rust_fixture.case);
    assert_no_temporary_work_file(&python_fixture.case);
}

fn prepare_failure(fixture: &TemporaryCase, scenario: FailureScenario) -> Vec<&'static OsStr> {
    let mut text = manifest_text("drafting", "codex");

    match scenario {
        FailureScenario::MissingManifest => {
            return vec![OsStr::new("--status"), OsStr::new("drafting")];
        }
        FailureScenario::LegacySchema => {
            text = text.replacen("schema_version = 2", "schema_version = 1", 1);
        }
        FailureScenario::InvalidExistingStatus => {
            text = text.replacen("status = \"drafting\"", "status = \"in_progress\"", 1);
        }
        FailureScenario::UnsupportedList => text.push_str("custom = []\n"),
        FailureScenario::NumericSchema => {
            text = text.replacen("schema_version = 2", "schema_version = 2.0", 1);
        }
        FailureScenario::EmptyUpdate
        | FailureScenario::ActiveWithoutOwner
        | FailureScenario::CompleteWithOwner => {}
    }

    fs::write(fixture.case.join("work.toml"), text).expect("failure manifest should be written");

    match scenario {
        FailureScenario::EmptyUpdate => Vec::new(),
        FailureScenario::ActiveWithoutOwner => vec![
            OsStr::new("--status"),
            OsStr::new("awaiting_review"),
            OsStr::new("--next-agent"),
            OsStr::new(""),
        ],
        FailureScenario::CompleteWithOwner => vec![
            OsStr::new("--phase"),
            OsStr::new("complete"),
            OsStr::new("--status"),
            OsStr::new("complete"),
            OsStr::new("--next-agent"),
            OsStr::new("codex"),
        ],
        FailureScenario::InvalidExistingStatus
        | FailureScenario::UnsupportedList
        | FailureScenario::NumericSchema => {
            vec![OsStr::new("--action"), OsStr::new("Keep moving.")]
        }
        FailureScenario::MissingManifest | FailureScenario::LegacySchema => {
            vec![OsStr::new("--status"), OsStr::new("drafting")]
        }
    }
}

fn run_python_cursor(case: &Path, extra: &[&OsStr]) -> Output {
    let reference = std::env::var_os("AGENTS_WORK_PYTHON_REFERENCE")
        .expect("AGENTS_WORK_PYTHON_REFERENCE must name agents_work.py");
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| OsString::from("python3"));

    Command::new(python)
        .arg(reference)
        .arg("cursor")
        .arg(case)
        .args(extra)
        .output()
        .expect("Python cursor should run")
}

fn run_rust_cursor(case: &Path, extra: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agents-work"))
        .arg("cursor")
        .arg(case)
        .args(extra)
        .output()
        .expect("Rust cursor should run")
}

fn normalize_case(output: &[u8], case: &Path) -> Vec<u8> {
    String::from_utf8(output.to_owned())
        .expect("process output should be UTF-8")
        .replace(&case.display().to_string(), "<CASE>")
        .into_bytes()
}

fn normalized_manifest(fixture: &TemporaryCase) -> String {
    let text = fs::read_to_string(fixture.case.join("work.toml"))
        .expect("oracle manifest should be readable");
    let mut lines = text
        .lines()
        .map(|line| {
            if line.starts_with("updated_at = ") {
                "updated_at = \"<TIME>\"".to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    lines.push('\n');
    lines
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

fn assert_no_temporary_work_file(case: &Path) {
    assert!(
        fs::read_dir(case)
            .expect("case should be readable")
            .all(|entry| !entry
                .expect("directory entry should be readable")
                .file_name()
                .to_string_lossy()
                .starts_with(".work-"))
    );
}
