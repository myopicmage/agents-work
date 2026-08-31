//! Deterministic draft construction and its thin filesystem shell.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::artifact::{
    ArtifactId, ArtifactKind, ArtifactMetadata, ArtifactName, OffsetDateTime, Sequence, Slug,
    parse_front_matter,
};
use crate::case::{create_new_file, discovered_artifacts, path_error, read_manifest, resolve_path};

// Python 3.14.6 uses Unicode 16.0 decimal digits for successful `int` parsing.
// Other `str.isdigit` characters make the reference implementation raise a
// ValueError before it can produce a protocol diagnostic.
const DECIMAL_ZEROES: [u32; 76] = [
    0x0030, 0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6,
    0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80,
    0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0,
    0xFF10, 0x104A0, 0x10D30, 0x10D40, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450,
    0x114D0, 0x11650, 0x116C0, 0x116D0, 0x116DA, 0x11730, 0x118E0, 0x11950, 0x11BF0, 0x11C50,
    0x11D50, 0x11DA0, 0x11F50, 0x16130, 0x16A60, 0x16AC0, 0x16B50, 0x16D70, 0x1CCF0, 0x1D7CE,
    0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E4F0, 0x1E5F1, 0x1E950, 0x1FBF0,
];

/// Borrowed command inputs before filesystem values are resolved.
#[derive(Clone, Copy, Debug)]
pub struct DraftRequest<'a> {
    pub case: &'a Path,
    pub kind: ArtifactKind,
    pub author: &'a str,
    pub topic: Option<&'a str>,
    pub responds_to: &'a [String],
    pub supersedes: &'a [String],
    pub output: Option<&'a Path>,
}

/// Explicit values from which a draft document is rendered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftInputs {
    pub artifact_id: ArtifactId,
    pub sequence: Sequence,
    pub kind: ArtifactKind,
    pub topic: Slug,
    pub author: String,
    pub created_at: OffsetDateTime,
    pub responds_to: Vec<ArtifactName>,
    pub supersedes: Vec<ArtifactName>,
}

/// Exact filename and bytes for one generated draft.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftDocument {
    filename: String,
    body: String,
}

impl DraftDocument {
    /// Constructs and round-trips a publishable draft document.
    ///
    /// # Errors
    ///
    /// Returns a Python-compatible field diagnostic if a loose input is not
    /// accepted or if the generated document does not validate.
    pub fn build(inputs: DraftInputs) -> Result<Self, DraftError> {
        let prospective_filename = artifact_filename(
            inputs.sequence,
            &inputs.topic,
            &inputs.author,
            inputs.artifact_id,
        );
        let author = inputs.author.parse::<Slug>().map_err(|_| {
            DraftError::validation(format!(
                "{prospective_filename}: author must be a lowercase slug"
            ))
        })?;
        let metadata = ArtifactMetadata {
            artifact_id: inputs.artifact_id,
            sequence: inputs.sequence,
            kind: inputs.kind,
            topic: inputs.topic,
            author,
            created_at: inputs.created_at,
            responds_to: inputs.responds_to,
            supersedes: inputs.supersedes,
            source_branch: String::new(),
            source_commit: String::new(),
            source_path: String::new(),
            subject_repository: String::new(),
            subject_path: String::new(),
            subject_commit: String::new(),
        };
        let filename = metadata.filename();
        let body = format!("{}\n# TITLE\n", render_front_matter(&metadata));
        prove_generated_document(&filename, &body, &metadata)?;

        Ok(Self { filename, body })
    }

    /// Returns the eventual published filename encoded by the metadata.
    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Returns the exact UTF-8 document text.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

/// A draft validation, environment, or filesystem failure.
#[derive(Debug)]
pub enum DraftError {
    Validation(String),
    Io(io::Error),
    Random(getrandom::Error),
    LocalOffset(time::error::IndeterminateOffset),
}

impl DraftError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }
}

impl Display for DraftError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(message) => formatter.write_str(message),
            Self::Io(error) => error.fmt(formatter),
            Self::Random(error) => write!(formatter, "system randomness unavailable: {error}"),
            Self::LocalOffset(error) => {
                write!(
                    formatter,
                    "could not determine the local UTC offset: {error}"
                )
            }
        }
    }
}

impl Error for DraftError {}

impl From<io::Error> for DraftError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Creates a draft using the system clock and operating-system entropy.
///
/// # Errors
///
/// Returns an error for invalid protocol input, unavailable environment
/// values, filesystem failures, or output failures.
pub fn draft(
    request: DraftRequest<'_>,
    standard_output: &mut impl Write,
) -> Result<PathBuf, DraftError> {
    draft_with(
        request,
        current_local_timestamp,
        random_artifact_id,
        standard_output,
    )
}

/// Creates a draft with explicit clock and artifact-ID sources.
///
/// The closures make the two nondeterministic inputs controllable without
/// changing the command's filesystem behavior.
///
/// # Errors
///
/// Returns an error for invalid protocol input, unavailable injected values,
/// filesystem failures, or output failures.
pub fn draft_with(
    request: DraftRequest<'_>,
    clock: impl FnOnce() -> Result<OffsetDateTime, DraftError>,
    mut next_artifact_id: impl FnMut() -> Result<ArtifactId, DraftError>,
    standard_output: &mut impl Write,
) -> Result<PathBuf, DraftError> {
    let case = resolve_path(request.case)?;
    let (manifest, _) = read_manifest(&case);
    let Some(manifest) = manifest else {
        return Err(DraftError::validation(format!(
            "{}: missing or unreadable work.toml",
            case.display()
        )));
    };
    let topic = resolve_topic(request.topic, manifest.table())?;
    let artifact_id = unused_artifact_id(&case, &mut next_artifact_id)?;
    let sequence = next_sequence(&artifact_names(&case)?)
        .map_err(|error| DraftError::validation(error.to_string()))?;
    let created_at = clock()?;
    let responds_to = resolve_case_references(&case, request.responds_to)?;
    let supersedes = resolve_case_references(&case, request.supersedes)?;
    let document = DraftDocument::build(DraftInputs {
        artifact_id,
        sequence,
        kind: request.kind,
        topic,
        author: request.author.to_owned(),
        created_at,
        responds_to,
        supersedes,
    })?;
    let draft_path = match request.output {
        Some(output) => resolve_path(output)?,
        None => case.join(format!(".draft-{}", document.filename())),
    };
    let mut file = create_new(&draft_path)?;
    file.write_all(document.body().as_bytes())
        .map_err(|error| path_error(&draft_path, &error))?;
    drop(file);
    writeln!(standard_output, "{}", draft_path.display())?;

    Ok(draft_path)
}

/// Returns the next sequence after the highest discovered filename sequence.
///
/// # Errors
///
/// Returns an error only if the resulting positive sequence cannot be
/// represented.
pub fn next_sequence(
    inventory: &[ArtifactName],
) -> Result<Sequence, crate::artifact::InvalidSequence> {
    let highest = inventory
        .iter()
        .map(ArtifactName::filename_sequence)
        .max()
        .unwrap_or(0);
    Sequence::try_from(i64::from(highest) + 1)
}

/// Resolves a full artifact name or Python-compatible decimal sequence.
///
/// # Errors
///
/// Returns the Python diagnostic for unknown, absent, or ambiguous references.
pub fn resolve_reference(
    reference: &str,
    inventory: &[ArtifactName],
) -> Result<ArtifactName, DraftError> {
    if let Some(name) = inventory.iter().find(|name| name.as_str() == reference) {
        return Ok(name.clone());
    }

    let Some(wanted) = normalize_decimal(reference) else {
        return Err(DraftError::validation(format!(
            "unknown artifact reference: {reference}"
        )));
    };
    let mut matches = inventory
        .iter()
        .filter(|name| name.filename_sequence().to_string() == wanted)
        .cloned()
        .collect::<Vec<_>>();
    matches.sort();

    match matches.as_slice() {
        [] => Err(DraftError::validation(format!(
            "no artifact with sequence {wanted}"
        ))),
        [name] => Ok(name.clone()),
        _ => Err(DraftError::validation(format!(
            "sequence {wanted} is ambiguous, name one of: {}",
            matches
                .iter()
                .map(ArtifactName::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn resolve_topic(topic: Option<&str>, manifest: &toml::Table) -> Result<Slug, DraftError> {
    let candidate = topic.or_else(|| manifest.get("id").and_then(toml::Value::as_str));
    candidate
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| {
            DraftError::validation(
                "topic must be a lowercase slug; work.toml has no usable id, so pass --topic",
            )
        })
}

fn unused_artifact_id(
    case: &Path,
    next_artifact_id: &mut impl FnMut() -> Result<ArtifactId, DraftError>,
) -> Result<ArtifactId, DraftError> {
    let taken = artifact_names(case)?
        .into_iter()
        .map(|name| name.artifact_id())
        .collect::<BTreeSet<_>>();

    loop {
        let candidate = next_artifact_id()?;

        if !taken.contains(&candidate) {
            return Ok(candidate);
        }
    }
}

fn resolve_case_references(
    case: &Path,
    references: &[String],
) -> Result<Vec<ArtifactName>, DraftError> {
    references
        .iter()
        .map(|reference| resolve_reference(reference, &artifact_names(case)?))
        .collect()
}

fn artifact_names(case: &Path) -> Result<Vec<ArtifactName>, DraftError> {
    discovered_artifacts(case)?
        .iter()
        .filter_map(|path| path.file_name()?.to_str())
        .map(|name| {
            name.parse::<ArtifactName>().map_err(|error| {
                DraftError::validation(format!("invalid discovered artifact name: {error}"))
            })
        })
        .collect()
}

fn random_artifact_id() -> Result<ArtifactId, DraftError> {
    let mut bytes = [0_u8; 3];
    getrandom::fill(&mut bytes).map_err(DraftError::Random)?;
    Ok(bytes.into())
}

fn current_local_timestamp() -> Result<OffsetDateTime, DraftError> {
    let current = time::OffsetDateTime::now_local().map_err(DraftError::LocalOffset)?;
    let offset = current.offset();
    let (offset_hours, offset_minutes, offset_seconds) = offset.as_hms();
    let sign = if offset.is_negative() { '-' } else { '+' };
    let offset = if offset_seconds == 0 {
        format!(
            "{sign}{:02}:{:02}",
            offset_hours.unsigned_abs(),
            offset_minutes.unsigned_abs()
        )
    } else {
        format!(
            "{sign}{:02}:{:02}:{:02}",
            offset_hours.unsigned_abs(),
            offset_minutes.unsigned_abs(),
            offset_seconds.unsigned_abs()
        )
    };
    let timestamp = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{offset}",
        current.year(),
        u8::from(current.month()),
        current.day(),
        current.hour(),
        current.minute(),
        current.second()
    );
    timestamp.parse().map_err(|error| {
        DraftError::validation(format!(
            "current local timestamp is not TOML-compatible: {error}"
        ))
    })
}

fn create_new(path: &Path) -> Result<File, DraftError> {
    create_new_file(path, 0o644).map_err(Into::into)
}

fn artifact_filename(
    sequence: Sequence,
    topic: &Slug,
    author: &str,
    artifact_id: ArtifactId,
) -> String {
    format!("{sequence:03}-{topic}-{author}-{artifact_id}.md")
}

fn render_front_matter(metadata: &ArtifactMetadata) -> String {
    let mut output = String::new();
    output.push_str("+++\nartifact_schema_version = 1\nartifact_id = \"");
    output.push_str(&metadata.artifact_id.to_string());
    output.push_str("\"\nsequence = ");
    output.push_str(&metadata.sequence.to_string());
    output.push_str("\nkind = \"");
    output.push_str(&metadata.kind.to_string());
    output.push_str("\"\ntopic = \"");
    output.push_str(metadata.topic.as_str());
    output.push_str("\"\nauthor = \"");
    output.push_str(metadata.author.as_str());
    output.push_str("\"\ncreated_at = ");
    output.push_str(&metadata.created_at.as_datetime().to_string());
    output.push_str("\nresponds_to = ");
    output.push_str(&render_relationships(&metadata.responds_to));
    output.push_str("\nsupersedes = ");
    output.push_str(&render_relationships(&metadata.supersedes));
    output.push_str(
        "\nsource_branch = \"\"\nsource_commit = \"\"\nsource_path = \"\"\nsubject_repository = \"\"\nsubject_path = \"\"\nsubject_commit = \"\"\n+++\n",
    );
    output
}

fn render_relationships(items: &[ArtifactName]) -> String {
    if items.is_empty() {
        return "[]".to_owned();
    }

    let mut output = "[\n".to_owned();

    for item in items {
        output.push_str("  \"");
        output.push_str(item.as_str());
        output.push_str("\",\n");
    }

    output.push(']');
    output
}

fn prove_generated_document(
    filename: &str,
    body: &str,
    expected: &ArtifactMetadata,
) -> Result<(), DraftError> {
    let source = Path::new(filename);
    let table = parse_front_matter(body.as_bytes(), source)
        .map_err(|error| DraftError::validation(error.to_string()))?;
    let parsed = ArtifactMetadata::parse(&table, source)
        .map_err(|error| DraftError::validation(error.to_string()))?;
    parsed
        .validate_filename(source)
        .map_err(|error| DraftError::validation(error.to_string()))?;

    if parsed != *expected {
        return Err(DraftError::validation(format!(
            "{filename}: rendered metadata does not round-trip"
        )));
    }

    Ok(())
}

fn normalize_decimal(value: &str) -> Option<String> {
    let digits = value
        .chars()
        .map(decimal_digit)
        .collect::<Option<Vec<_>>>()?;

    if digits.is_empty() {
        return None;
    }

    let first_nonzero = digits.iter().position(|digit| *digit != 0);
    let Some(first_nonzero) = first_nonzero else {
        return Some("0".to_owned());
    };
    Some(
        digits[first_nonzero..]
            .iter()
            .map(|digit| char::from(b'0' + digit))
            .collect(),
    )
}

fn decimal_digit(character: char) -> Option<u8> {
    let codepoint = u32::from(character);

    DECIMAL_ZEROES.iter().find_map(|zero| {
        let digit = codepoint.checked_sub(*zero)?;
        u8::try_from(digit).ok().filter(|digit| *digit < 10)
    })
}
