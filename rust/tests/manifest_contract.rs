use agents_work::cli::{PhaseArgument, StatusArgument};
use agents_work::manifest::{ManifestDocument, ManifestSchema, Phase, Status};
use toml::Table;

fn validate(text: &str) -> (agents_work::manifest::ManifestDocument, Vec<String>) {
    let table = text.parse::<Table>().expect("test manifest should be TOML");
    ManifestDocument::validate(table, "work.toml").into_parts()
}

#[test]
fn valid_schema_two_manifest_retains_unknown_fields() {
    let (manifest, errors) = validate(
        r#"
schema_version = 2
phase = "planning"
status = "drafting"
next_agent = "codex"
custom_note = "keep me"
"#,
    );

    assert!(errors.is_empty());
    assert_eq!(manifest.schema(), Some(ManifestSchema::V2));
    assert_eq!(manifest.table()["custom_note"].as_str(), Some("keep me"));
}

#[test]
fn malformed_coordination_fields_accumulate() {
    let (_, errors) = validate(
        r#"
schema_version = 2
phase = "unknown"
status = "unknown"
next_agent = 7
"#,
    );

    assert_eq!(
        errors,
        [
            "work.toml: phase must be one of complete, implementation, planning, pr_review",
            "work.toml: status must be one of awaiting_decision, awaiting_review, complete, deferred, drafting, ready_for_implementation, revision_requested",
            "work.toml: next_agent must be a string",
        ]
    );
}

#[test]
fn active_status_requires_an_owner() {
    let (_, errors) = validate(
        r#"
schema_version = 2
phase = "planning"
status = "awaiting_review"
next_agent = ""
"#,
    );

    assert_eq!(errors, ["work.toml: active status requires next_agent"]);
}

#[test]
fn complete_status_requires_an_empty_owner_and_complete_phase() {
    let (_, errors) = validate(
        r#"
schema_version = 2
phase = "planning"
status = "complete"
next_agent = "codex"
"#,
    );

    assert_eq!(
        errors,
        [
            "work.toml: complete status requires empty next_agent",
            "work.toml: complete status requires complete phase",
        ]
    );
}

#[test]
fn complete_phase_requires_complete_status() {
    let (_, errors) = validate(
        r#"
schema_version = 2
phase = "complete"
status = "deferred"
next_agent = ""
"#,
    );

    assert_eq!(
        errors,
        ["work.toml: complete phase requires complete status"]
    );
}

#[test]
fn schema_comparison_preserves_python_numeric_equality() {
    for (value, expected) in [
        ("true", ManifestSchema::V1),
        ("1.0", ManifestSchema::V1),
        ("2.0", ManifestSchema::V2),
    ] {
        let text = format!(
            r#"
schema_version = {value}
phase = "planning"
status = "deferred"
next_agent = ""
"#
        );
        let (manifest, errors) = validate(&text);

        assert!(errors.is_empty());
        assert_eq!(manifest.schema(), Some(expected));
    }
}

#[test]
fn unsupported_schema_is_retained_without_a_supported_version() {
    let (manifest, errors) = validate(
        r#"
schema_version = 3
phase = "planning"
status = "deferred"
next_agent = ""
"#,
    );

    assert_eq!(manifest.schema(), None);
    assert_eq!(errors, ["work.toml: schema_version must be 1 or 2"]);
}

#[test]
fn schema_two_rejects_legacy_index_fields_in_sorted_order() {
    let (_, errors) = validate(
        r#"
schema_version = 2
phase = "planning"
status = "deferred"
next_agent = ""
latest_sequence = 4
latest_artifacts = []
artifacts = []
"#,
    );

    assert_eq!(
        errors,
        [
            "work.toml: schema 2 contains legacy fields: artifacts, latest_artifacts, latest_sequence"
        ]
    );
}

#[test]
fn schema_one_retains_legacy_records_for_inventory_comparison() {
    let (manifest, errors) = validate(
        r#"
schema_version = 1
phase = "planning"
status = "deferred"
next_agent = ""

[[artifacts]]
path = "001-test-plan-codex-a1b2c3.md"
sha256 = "abc123"
"#,
    );

    assert!(errors.is_empty());
    assert_eq!(manifest.schema(), Some(ManifestSchema::V1));
    assert_eq!(
        manifest.table()["artifacts"].as_array().map(Vec::len),
        Some(1)
    );
}

#[test]
fn command_line_values_convert_to_domain_values() {
    assert_eq!(Phase::from(PhaseArgument::Complete), Phase::Complete);
    assert_eq!(
        Phase::from(PhaseArgument::Implementation),
        Phase::Implementation
    );
    assert_eq!(Phase::from(PhaseArgument::Planning), Phase::Planning);
    assert_eq!(Phase::from(PhaseArgument::PrReview), Phase::PrReview);

    assert_eq!(
        Status::from(StatusArgument::AwaitingReview),
        Status::AwaitingReview
    );
    assert_eq!(Status::from(StatusArgument::Complete), Status::Complete);
    assert_eq!(Status::from(StatusArgument::Deferred), Status::Deferred);
    assert_eq!(Status::from(StatusArgument::Drafting), Status::Drafting);
    assert_eq!(
        Status::from(StatusArgument::ReadyForImplementation),
        Status::ReadyForImplementation
    );
    assert_eq!(
        Status::from(StatusArgument::RevisionRequested),
        Status::RevisionRequested
    );
}
