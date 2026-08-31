//! Parsed artifact names and TOML front matter.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::num::NonZeroU64;
use std::path::Path;
use std::str::FromStr;
use std::sync::LazyLock;

use regex::Regex;
use toml::Table;
use toml::Value;
use toml::value::Datetime;

const REQUIRED_FIELDS: [&str; 9] = [
    "artifact_schema_version",
    "artifact_id",
    "sequence",
    "kind",
    "topic",
    "author",
    "created_at",
    "responds_to",
    "supersedes",
];

const OPTIONAL_STRING_FIELDS: [&str; 6] = [
    "source_branch",
    "source_commit",
    "source_path",
    "subject_repository",
    "subject_path",
    "subject_commit",
];

const ARTIFACT_KINDS: &str = "decision, plan, proposal, response, review";

static ARTIFACT_NAME_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?<sequence>[0-9]{3})-[a-z0-9][a-z0-9-]*-[a-z0-9][a-z0-9-]*-(?<artifact_id>[0-9a-f]{6})[.]md$",
    )
    .expect("the artifact-name pattern is valid")
});

static ARTIFACT_ID_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9a-f]{6}$").expect("the artifact-id pattern is valid"));

static SLUG_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]*$").expect("the slug pattern is valid"));

/// A six-character lowercase hexadecimal artifact identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactId([u8; 3]);

impl FromStr for ArtifactId {
    type Err = InvalidArtifactId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if !ARTIFACT_ID_PATTERN.is_match(value) {
            return Err(InvalidArtifactId);
        }

        let bytes = value.as_bytes();
        Ok(Self([
            decode_hex_pair(bytes[0], bytes[1]).ok_or(InvalidArtifactId)?,
            decode_hex_pair(bytes[2], bytes[3]).ok_or(InvalidArtifactId)?,
            decode_hex_pair(bytes[4], bytes[5]).ok_or(InvalidArtifactId)?,
        ]))
    }
}

impl From<[u8; 3]> for ArtifactId {
    fn from(value: [u8; 3]) -> Self {
        Self(value)
    }
}

impl Display for ArtifactId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:02x}{:02x}{:02x}",
            self.0[0], self.0[1], self.0[2]
        )
    }
}

/// An artifact identifier was not exactly six lowercase hexadecimal digits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidArtifactId;

impl Display for InvalidArtifactId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("artifact id must be 6 lowercase hex")
    }
}

impl Error for InvalidArtifactId {}

/// A lowercase ASCII slug accepted by the Python implementation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Slug(String);

impl Slug {
    /// Returns the slug text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Slug {
    type Err = InvalidSlug;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if !SLUG_PATTERN.is_match(value) {
            return Err(InvalidSlug);
        }

        Ok(Self(value.to_owned()))
    }
}

impl Display for Slug {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A string was not a lowercase ASCII slug.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidSlug;

impl Display for InvalidSlug {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("value must be a lowercase slug")
    }
}

impl Error for InvalidSlug {}

/// A positive artifact sequence from front matter.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Sequence(NonZeroU64);

impl Sequence {
    /// Returns the numeric sequence.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl TryFrom<i64> for Sequence {
    type Error = InvalidSequence;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        let value = u64::try_from(value).map_err(|_| InvalidSequence)?;
        NonZeroU64::new(value).map_or(Err(InvalidSequence), |value| Ok(Self(value)))
    }
}

impl Display for Sequence {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// A sequence was not a positive TOML integer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidSequence;

impl Display for InvalidSequence {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("sequence must be a positive integer")
    }
}

impl Error for InvalidSequence {}

/// One of the five artifact kinds accepted by the protocol.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ArtifactKind {
    Decision,
    Plan,
    Proposal,
    Response,
    Review,
}

impl FromStr for ArtifactKind {
    type Err = InvalidArtifactKind;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "decision" => Ok(Self::Decision),
            "plan" => Ok(Self::Plan),
            "proposal" => Ok(Self::Proposal),
            "response" => Ok(Self::Response),
            "review" => Ok(Self::Review),
            _ => Err(InvalidArtifactKind),
        }
    }
}

impl Display for ArtifactKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Decision => "decision",
            Self::Plan => "plan",
            Self::Proposal => "proposal",
            Self::Response => "response",
            Self::Review => "review",
        })
    }
}

/// A string was not one of the protocol's artifact kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidArtifactKind;

impl Display for InvalidArtifactKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "kind must be one of {ARTIFACT_KINDS}")
    }
}

impl Error for InvalidArtifactKind {}

/// A case-relative published artifact filename.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArtifactName {
    value: String,
    filename_sequence: u16,
    artifact_id: ArtifactId,
}

impl ArtifactName {
    /// Returns the three-digit sequence encoded in the filename.
    #[must_use]
    pub fn filename_sequence(&self) -> u16 {
        self.filename_sequence
    }

    /// Returns the artifact identifier encoded in the filename.
    #[must_use]
    pub fn artifact_id(&self) -> ArtifactId {
        self.artifact_id
    }

    /// Returns the complete filename.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl FromStr for ArtifactName {
    type Err = InvalidArtifactName;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let captures = ARTIFACT_NAME_PATTERN
            .captures(value)
            .ok_or(InvalidArtifactName)?;
        let filename_sequence = captures["sequence"]
            .parse::<u16>()
            .map_err(|_| InvalidArtifactName)?;
        let artifact_id = captures["artifact_id"]
            .parse()
            .map_err(|_| InvalidArtifactName)?;

        Ok(Self {
            value: value.to_owned(),
            filename_sequence,
            artifact_id,
        })
    }
}

impl Display for ArtifactName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.value)
    }
}

/// A string was not a case-relative published artifact filename.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidArtifactName;

impl Display for InvalidArtifactName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("value must be a case-relative artifact name")
    }
}

impl Error for InvalidArtifactName {}

/// A TOML offset date-time, excluding local date-times and date-only values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OffsetDateTime(Datetime);

impl OffsetDateTime {
    /// Returns the parsed TOML date-time.
    #[must_use]
    pub fn as_datetime(&self) -> &Datetime {
        &self.0
    }
}

impl FromStr for OffsetDateTime {
    type Err = MissingUtcOffset;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let datetime = value.parse::<Datetime>().map_err(|_| MissingUtcOffset)?;
        Self::try_from(datetime)
    }
}

impl TryFrom<Datetime> for OffsetDateTime {
    type Error = MissingUtcOffset;

    fn try_from(value: Datetime) -> Result<Self, Self::Error> {
        if value.date.is_none() || value.time.is_none() || value.offset.is_none() {
            return Err(MissingUtcOffset);
        }

        Ok(Self(value))
    }
}

/// A TOML value was not a date-time carrying an explicit UTC offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MissingUtcOffset;

impl Display for MissingUtcOffset {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("date-time must include a UTC offset")
    }
}

impl Error for MissingUtcOffset {}

/// Validated artifact front matter for the current schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactMetadata {
    pub artifact_id: ArtifactId,
    pub sequence: Sequence,
    pub kind: ArtifactKind,
    pub topic: Slug,
    pub author: Slug,
    pub created_at: OffsetDateTime,
    pub responds_to: Vec<ArtifactName>,
    pub supersedes: Vec<ArtifactName>,
    pub source_branch: String,
    pub source_commit: String,
    pub source_path: String,
    pub subject_repository: String,
    pub subject_path: String,
    pub subject_commit: String,
}

impl ArtifactMetadata {
    /// Parses a TOML table, accumulating independent field errors.
    ///
    /// # Errors
    ///
    /// Returns all field errors found, except that missing required fields are
    /// returned immediately to match the Python implementation.
    pub fn parse(table: &Table, source: &Path) -> Result<Self, ValidationErrors> {
        let source = source_label(source);
        let mut missing = REQUIRED_FIELDS
            .iter()
            .filter(|field| !table.contains_key(**field))
            .copied()
            .collect::<Vec<_>>();

        if !missing.is_empty() {
            missing.sort_unstable();
            return Err(ValidationErrors::one(format!(
                "{source}: missing fields: {}",
                missing.join(", ")
            )));
        }

        let mut errors = Vec::new();

        validate_schema_version(&table["artifact_schema_version"], &source, &mut errors);
        let artifact_id = parse_artifact_id(&table["artifact_id"], &source, &mut errors);
        let sequence = parse_sequence(&table["sequence"], &source, &mut errors);
        let kind = parse_kind(&table["kind"], &source, &mut errors);
        let topic = parse_slug(&table["topic"], "topic", &source, &mut errors);
        let author = parse_slug(&table["author"], "author", &source, &mut errors);
        let created_at = parse_created_at(&table["created_at"], &source, &mut errors);
        let responds_to =
            parse_relationships(&table["responds_to"], "responds_to", &source, &mut errors);
        let supersedes =
            parse_relationships(&table["supersedes"], "supersedes", &source, &mut errors);
        let optional = parse_optional_strings(table, &source, &mut errors);

        if !errors.is_empty() {
            return Err(ValidationErrors(errors));
        }

        let [
            source_branch,
            source_commit,
            source_path,
            subject_repository,
            subject_path,
            subject_commit,
        ] = optional;

        match (
            artifact_id,
            sequence,
            kind,
            topic,
            author,
            created_at,
            responds_to,
            supersedes,
        ) {
            (
                Some(artifact_id),
                Some(sequence),
                Some(kind),
                Some(topic),
                Some(author),
                Some(created_at),
                Some(responds_to),
                Some(supersedes),
            ) => Ok(Self {
                artifact_id,
                sequence,
                kind,
                topic,
                author,
                created_at,
                responds_to,
                supersedes,
                source_branch,
                source_commit,
                source_path,
                subject_repository,
                subject_path,
                subject_commit,
            }),
            _ => Err(ValidationErrors::one(format!(
                "{source}: invalid artifact metadata"
            ))),
        }
    }

    /// Renders the filename implied by this front matter.
    #[must_use]
    pub fn filename(&self) -> String {
        format!(
            "{:03}-{}-{}-{}.md",
            self.sequence, self.topic, self.author, self.artifact_id
        )
    }

    /// Checks that a published file's name agrees with its front matter.
    ///
    /// # Errors
    ///
    /// Returns the Python-compatible mismatch diagnostic.
    pub fn validate_filename(&self, source: &Path) -> Result<(), ValidationErrors> {
        let expected = self.filename();
        let actual = source_label(source);

        if actual != expected {
            return Err(ValidationErrors::one(format!(
                "{actual}: filename does not match front matter: {expected}"
            )));
        }

        Ok(())
    }
}

/// Front-matter syntax failed before field validation could begin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontMatterError(String);

impl Display for FrontMatterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for FrontMatterError {}

/// One or more artifact field validation failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationErrors(Vec<String>);

impl ValidationErrors {
    fn one(error: String) -> Self {
        Self(vec![error])
    }

    /// Returns each independent validation failure in protocol order.
    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    /// Consumes the wrapper and returns the individual diagnostics.
    #[must_use]
    pub fn into_vec(self) -> Vec<String> {
        self.0
    }
}

impl Display for ValidationErrors {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.join("\n"))
    }
}

impl Error for ValidationErrors {}

/// Extracts and parses the leading TOML front matter from an artifact.
///
/// # Errors
///
/// Returns a source-labelled error if the exact delimiters, UTF-8, or TOML are
/// invalid.
pub fn parse_front_matter(data: &[u8], source: &Path) -> Result<Table, FrontMatterError> {
    const OPENING: &[u8] = b"+++\n";
    const CLOSING: &[u8] = b"\n+++\n";

    let source = source_label(source);

    if !data.starts_with(OPENING) {
        return Err(FrontMatterError(format!(
            "{source}: missing TOML front matter"
        )));
    }

    let remainder = &data[OPENING.len()..];
    let Some(end) = remainder
        .windows(CLOSING.len())
        .position(|window| window == CLOSING)
    else {
        return Err(FrontMatterError(format!(
            "{source}: unterminated TOML front matter"
        )));
    };

    let raw = std::str::from_utf8(&remainder[..end]).map_err(|error| {
        FrontMatterError(format!("{source}: invalid TOML front matter: {error}"))
    })?;
    raw.parse::<Table>()
        .map_err(|error| FrontMatterError(format!("{source}: invalid TOML front matter: {error}")))
}

fn decode_hex_pair(high: u8, low: u8) -> Option<u8> {
    Some((decode_hex_digit(high)? << 4) | decode_hex_digit(low)?)
}

fn decode_hex_digit(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

fn source_label(source: &Path) -> String {
    source
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

// Exact equality preserves Python's acceptance of numeric schema version 1.0.
#[allow(clippy::float_cmp)]
fn validate_schema_version(value: &Value, source: &str, errors: &mut Vec<String>) {
    let supported = match value {
        Value::Integer(1) | Value::Boolean(true) => true,
        Value::Float(value) => *value == 1.0,
        _ => false,
    };

    if !supported {
        errors.push(format!("{source}: unsupported artifact schema"));
    }
}

fn parse_artifact_id(value: &Value, source: &str, errors: &mut Vec<String>) -> Option<ArtifactId> {
    let parsed = value.as_str().and_then(|value| value.parse().ok());

    if parsed.is_none() {
        errors.push(format!("{source}: artifact_id must be 6 lowercase hex"));
    }

    parsed
}

fn parse_sequence(value: &Value, source: &str, errors: &mut Vec<String>) -> Option<Sequence> {
    let parsed = value
        .as_integer()
        .and_then(|value| Sequence::try_from(value).ok());

    if parsed.is_none() {
        errors.push(format!("{source}: sequence must be a positive integer"));
    }

    parsed
}

fn parse_kind(value: &Value, source: &str, errors: &mut Vec<String>) -> Option<ArtifactKind> {
    let parsed = value.as_str().and_then(|value| value.parse().ok());

    if parsed.is_none() {
        errors.push(format!("{source}: kind must be one of {ARTIFACT_KINDS}"));
    }

    parsed
}

fn parse_slug(value: &Value, field: &str, source: &str, errors: &mut Vec<String>) -> Option<Slug> {
    let parsed = value.as_str().and_then(|value| value.parse().ok());

    if parsed.is_none() {
        errors.push(format!("{source}: {field} must be a lowercase slug"));
    }

    parsed
}

fn parse_created_at(
    value: &Value,
    source: &str,
    errors: &mut Vec<String>,
) -> Option<OffsetDateTime> {
    let parsed = value
        .as_datetime()
        .copied()
        .and_then(|value| OffsetDateTime::try_from(value).ok());

    if parsed.is_none() {
        errors.push(format!("{source}: created_at must include a UTC offset"));
    }

    parsed
}

fn parse_relationships(
    value: &Value,
    field: &str,
    source: &str,
    errors: &mut Vec<String>,
) -> Option<Vec<ArtifactName>> {
    let parsed = value.as_array().and_then(|values| {
        values
            .iter()
            .map(|value| {
                let value = value.as_str()?;

                if Path::new(value).file_name()?.to_str()? != value {
                    return None;
                }

                value.parse().ok()
            })
            .collect::<Option<Vec<_>>>()
    });

    if parsed.is_none() {
        errors.push(format!(
            "{source}: {field} must contain case-relative artifact names"
        ));
    }

    parsed
}

fn parse_optional_strings(table: &Table, source: &str, errors: &mut Vec<String>) -> [String; 6] {
    OPTIONAL_STRING_FIELDS.map(|field| match table.get(field) {
        None => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(_) => {
            errors.push(format!("{source}: {field} must be a string"));
            String::new()
        }
    })
}
