use std::path::Path;

use agents_work::artifact::{
    ArtifactId, ArtifactKind, ArtifactMetadata, ArtifactName, Sequence, Slug, parse_front_matter,
};
use agents_work::cli::KindArgument;

const VALID_FRONT_MATTER: &str = r#"+++
artifact_schema_version = 1
artifact_id = "a1b2c3"
sequence = 1
kind = "plan"
topic = "test-plan"
author = "codex"
created_at = 2026-07-27T19:00:00+09:00
responds_to = ["000-earlier-plan-claude-010203.md"]
supersedes = []
source_branch = "feature/test"
source_commit = "abc123"
source_path = "draft.md"
subject_repository = "example"
subject_path = "/tmp/example"
subject_commit = "def456"
+++
# Test artifact
"#;

fn parse_metadata(text: &str, source: &str) -> ArtifactMetadata {
    let source = Path::new(source);
    let table = parse_front_matter(text.as_bytes(), source).expect("front matter should parse");
    ArtifactMetadata::parse(&table, source).expect("metadata should parse")
}

#[test]
fn valid_front_matter_becomes_domain_types() {
    let metadata = parse_metadata(VALID_FRONT_MATTER, "001-test-plan-codex-a1b2c3.md");

    assert_eq!(metadata.artifact_id.to_string(), "a1b2c3");
    assert_eq!(metadata.sequence.get(), 1);
    assert_eq!(metadata.kind, ArtifactKind::Plan);
    assert_eq!(metadata.topic.as_str(), "test-plan");
    assert_eq!(metadata.author.as_str(), "codex");
    assert_eq!(
        metadata.created_at.as_datetime().to_string(),
        "2026-07-27T19:00:00+09:00"
    );
    assert_eq!(metadata.responds_to[0].filename_sequence(), 0);
    assert_eq!(metadata.responds_to[0].artifact_id().to_string(), "010203");
    assert!(metadata.supersedes.is_empty());
    assert_eq!(metadata.source_branch, "feature/test");
    assert_eq!(metadata.subject_commit, "def456");
}

#[test]
fn newtypes_accept_only_the_python_protocol_shapes() {
    assert_eq!(
        "a1b2c3"
            .parse::<ArtifactId>()
            .expect("valid id")
            .to_string(),
        "a1b2c3"
    );
    assert!("A1B2C3".parse::<ArtifactId>().is_err());
    assert!("a1b2c".parse::<ArtifactId>().is_err());

    assert_eq!(
        "test-plan".parse::<Slug>().expect("valid slug").as_str(),
        "test-plan"
    );
    assert!("Test-plan".parse::<Slug>().is_err());
    assert!("-test-plan".parse::<Slug>().is_err());
    assert!("tést-plan".parse::<Slug>().is_err());

    assert_eq!(Sequence::try_from(1).expect("positive sequence").get(), 1);
    assert!(Sequence::try_from(0).is_err());
    assert!(Sequence::try_from(-1).is_err());
}

#[test]
fn published_names_preserve_the_discovery_grammar() {
    let name = "000-test-plan-codex-a1b2c3.md"
        .parse::<ArtifactName>()
        .expect("Python discovery accepts sequence zero");

    assert_eq!(name.filename_sequence(), 0);
    assert_eq!(name.artifact_id().to_string(), "a1b2c3");
    assert_eq!(name.as_str(), "000-test-plan-codex-a1b2c3.md");
    assert!(
        "00-test-plan-codex-a1b2c3.md"
            .parse::<ArtifactName>()
            .is_err()
    );
    assert!(
        "1000-test-plan-codex-a1b2c3.md"
            .parse::<ArtifactName>()
            .is_err()
    );
    assert!(
        "../000-test-plan-codex-a1b2c3.md"
            .parse::<ArtifactName>()
            .is_err()
    );
}

#[test]
fn exact_front_matter_delimiters_are_required() {
    let source = Path::new("draft.md");

    assert_eq!(
        parse_front_matter(b"artifact_id = \"a1b2c3\"\n", source)
            .expect_err("opening delimiter is required")
            .to_string(),
        "draft.md: missing TOML front matter"
    );
    assert_eq!(
        parse_front_matter(b"+++\r\nartifact_id = \"a1b2c3\"\r\n+++\r\n", source)
            .expect_err("CRLF does not match Python's opening delimiter")
            .to_string(),
        "draft.md: missing TOML front matter"
    );
    assert_eq!(
        parse_front_matter(b"+++\nartifact_id = \"a1b2c3\"\n", source)
            .expect_err("closing delimiter is required")
            .to_string(),
        "draft.md: unterminated TOML front matter"
    );

    let invalid_utf8 = b"+++\nartifact_id = \"\xff\"\n+++\n";
    assert!(
        parse_front_matter(invalid_utf8, source)
            .expect_err("front matter must be UTF-8")
            .to_string()
            .starts_with("draft.md: invalid TOML front matter:")
    );
    assert!(
        parse_front_matter(b"+++\nnot = [valid TOML\n+++\n", source)
            .expect_err("front matter must be TOML")
            .to_string()
            .starts_with("draft.md: invalid TOML front matter:")
    );
}

#[test]
fn missing_fields_are_sorted_and_stop_further_validation() {
    let source = Path::new("draft.md");
    let table = parse_front_matter(b"+++\nartifact_id = 12\n+++\n", source)
        .expect("the TOML itself is valid");
    let errors = ArtifactMetadata::parse(&table, source)
        .expect_err("required fields are absent")
        .into_vec();

    assert_eq!(
        errors,
        [
            "draft.md: missing fields: artifact_schema_version, author, created_at, kind, responds_to, sequence, supersedes, topic"
        ]
    );
}

#[test]
fn independent_field_errors_accumulate() {
    let text = r#"+++
artifact_schema_version = 2
artifact_id = "A1B2C3"
sequence = 0
kind = "memo"
topic = "Bad-topic"
author = "-codex"
created_at = 2026-07-27T19:00:00
responds_to = ["../001-test-plan-codex-a1b2c3.md"]
supersedes = "001-test-plan-codex-a1b2c3.md"
source_branch = 7
subject_commit = false
+++
"#;
    let source = Path::new("draft.md");
    let table = parse_front_matter(text.as_bytes(), source).expect("the TOML itself is valid");
    let errors = ArtifactMetadata::parse(&table, source)
        .expect_err("every deliberately invalid field should be reported")
        .into_vec();

    assert_eq!(
        errors,
        [
            "draft.md: unsupported artifact schema",
            "draft.md: artifact_id must be 6 lowercase hex",
            "draft.md: sequence must be a positive integer",
            "draft.md: kind must be one of decision, plan, proposal, response, review",
            "draft.md: topic must be a lowercase slug",
            "draft.md: author must be a lowercase slug",
            "draft.md: created_at must include a UTC offset",
            "draft.md: responds_to must contain case-relative artifact names",
            "draft.md: supersedes must contain case-relative artifact names",
            "draft.md: source_branch must be a string",
            "draft.md: subject_commit must be a string",
        ]
    );
}

#[test]
fn optional_fields_default_empty_and_unknown_fields_are_ignored() {
    let text = VALID_FRONT_MATTER
        .replace("source_branch = \"feature/test\"\n", "")
        .replace("source_commit = \"abc123\"\n", "")
        .replace("source_path = \"draft.md\"\n", "")
        .replace("subject_repository = \"example\"\n", "")
        .replace("subject_path = \"/tmp/example\"\n", "")
        .replace("subject_commit = \"def456\"\n", "future_field = 42\n");
    let metadata = parse_metadata(&text, "draft.md");

    assert_eq!(metadata.source_branch, "");
    assert_eq!(metadata.source_commit, "");
    assert_eq!(metadata.source_path, "");
    assert_eq!(metadata.subject_repository, "");
    assert_eq!(metadata.subject_path, "");
    assert_eq!(metadata.subject_commit, "");
}

#[test]
fn current_schema_preserves_python_numeric_equality() {
    for accepted in ["true", "1.0"] {
        let text = VALID_FRONT_MATTER.replace(
            "artifact_schema_version = 1",
            &format!("artifact_schema_version = {accepted}"),
        );

        parse_metadata(&text, "draft.md");
    }
}

#[test]
fn filename_rendering_and_validation_match_front_matter() {
    let metadata = parse_metadata(VALID_FRONT_MATTER, "draft.md");

    assert_eq!(metadata.filename(), "001-test-plan-codex-a1b2c3.md");
    metadata
        .validate_filename(Path::new("001-test-plan-codex-a1b2c3.md"))
        .expect("matching filename should pass");
    assert_eq!(
        metadata
            .validate_filename(Path::new("draft.md"))
            .expect_err("prepared draft has not been published under its expected name")
            .to_string(),
        "draft.md: filename does not match front matter: 001-test-plan-codex-a1b2c3.md"
    );

    let four_digit = VALID_FRONT_MATTER.replace("sequence = 1", "sequence = 1000");
    let metadata = parse_metadata(&four_digit, "draft.md");
    assert_eq!(metadata.filename(), "1000-test-plan-codex-a1b2c3.md");
    assert!(metadata.filename().parse::<ArtifactName>().is_err());
}

#[test]
fn command_line_kind_converts_at_the_domain_boundary() {
    assert_eq!(
        ArtifactKind::from(KindArgument::Decision),
        ArtifactKind::Decision
    );
    assert_eq!(ArtifactKind::from(KindArgument::Plan), ArtifactKind::Plan);
    assert_eq!(
        ArtifactKind::from(KindArgument::Proposal),
        ArtifactKind::Proposal
    );
    assert_eq!(
        ArtifactKind::from(KindArgument::Response),
        ArtifactKind::Response
    );
    assert_eq!(
        ArtifactKind::from(KindArgument::Review),
        ArtifactKind::Review
    );
}
