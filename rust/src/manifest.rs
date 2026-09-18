//! Parsed coordination vocabulary and `work.toml` validation.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use toml::{Table, Value};

const PHASES: &str = "complete, implementation, planning, pr_review";
const STATUSES: &str = concat!(
    "awaiting_decision, awaiting_review, complete, deferred, drafting, ",
    "ready_for_implementation, revision_requested"
);
const LEGACY_FIELDS: [&str; 3] = ["latest_sequence", "latest_artifacts", "artifacts"];

/// A supported `work.toml` schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestSchema {
    V1,
    V2,
}

/// A coordination phase accepted by the Python implementation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Phase {
    Complete,
    Implementation,
    Planning,
    PrReview,
}

impl FromStr for Phase {
    type Err = InvalidPhase;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "complete" => Ok(Self::Complete),
            "implementation" => Ok(Self::Implementation),
            "planning" => Ok(Self::Planning),
            "pr_review" => Ok(Self::PrReview),
            _ => Err(InvalidPhase),
        }
    }
}

impl Display for Phase {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Complete => "complete",
            Self::Implementation => "implementation",
            Self::Planning => "planning",
            Self::PrReview => "pr_review",
        })
    }
}

/// A string was not one of the protocol's coordination phases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPhase;

impl Display for InvalidPhase {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "phase must be one of {PHASES}")
    }
}

impl Error for InvalidPhase {}

/// A coordination status accepted by the Python implementation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Status {
    AwaitingDecision,
    AwaitingReview,
    Complete,
    Deferred,
    Drafting,
    ReadyForImplementation,
    RevisionRequested,
}

impl Status {
    /// Returns whether the status requires a non-empty next agent.
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::AwaitingReview | Self::Drafting | Self::RevisionRequested
        )
    }

    /// Returns whether the status is a gate rather than active work.
    #[must_use]
    pub fn is_resting(self) -> bool {
        matches!(
            self,
            Self::AwaitingDecision | Self::Complete | Self::Deferred | Self::ReadyForImplementation
        )
    }
}

impl FromStr for Status {
    type Err = InvalidStatus;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "awaiting_decision" => Ok(Self::AwaitingDecision),
            "awaiting_review" => Ok(Self::AwaitingReview),
            "complete" => Ok(Self::Complete),
            "deferred" => Ok(Self::Deferred),
            "drafting" => Ok(Self::Drafting),
            "ready_for_implementation" => Ok(Self::ReadyForImplementation),
            "revision_requested" => Ok(Self::RevisionRequested),
            _ => Err(InvalidStatus),
        }
    }
}

impl Display for Status {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AwaitingDecision => "awaiting_decision",
            Self::AwaitingReview => "awaiting_review",
            Self::Complete => "complete",
            Self::Deferred => "deferred",
            Self::Drafting => "drafting",
            Self::ReadyForImplementation => "ready_for_implementation",
            Self::RevisionRequested => "revision_requested",
        })
    }
}

/// A string was not one of the protocol's coordination statuses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidStatus;

impl Display for InvalidStatus {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "status must be one of {STATUSES}")
    }
}

impl Error for InvalidStatus {}

/// A syntactically parsed manifest, including fields unknown to this version.
#[derive(Clone, Debug, PartialEq)]
pub struct ManifestDocument {
    table: Table,
    schema: Option<ManifestSchema>,
}

impl ManifestDocument {
    /// Validates a parsed TOML table while retaining it for legacy inspection.
    #[must_use]
    pub fn validate(table: Table, label: &str) -> ManifestValidation {
        let schema = table.get("schema_version").and_then(parse_schema);
        let mut errors = Vec::new();

        if schema.is_none() {
            errors.push(format!("{label}: schema_version must be 1 or 2"));
        }

        errors.extend(coordination_errors(&table, label));

        if schema == Some(ManifestSchema::V2) {
            let mut present = LEGACY_FIELDS
                .iter()
                .filter(|field| table.contains_key(**field))
                .copied()
                .collect::<Vec<_>>();
            present.sort_unstable();

            if !present.is_empty() {
                errors.push(format!(
                    "{label}: schema 2 contains legacy fields: {}",
                    present.join(", ")
                ));
            }
        }

        ManifestValidation {
            document: Self { table, schema },
            errors,
        }
    }

    /// Returns the supported schema, or `None` after an invalid schema value.
    #[must_use]
    pub fn schema(&self) -> Option<ManifestSchema> {
        self.schema
    }

    /// Returns the complete parsed table, including unknown and legacy fields.
    #[must_use]
    pub fn table(&self) -> &Table {
        &self.table
    }
}

/// A retained manifest plus every independent structural diagnostic.
#[derive(Clone, Debug, PartialEq)]
pub struct ManifestValidation {
    document: ManifestDocument,
    errors: Vec<String>,
}

impl ManifestValidation {
    /// Separates the retained manifest from its diagnostics.
    #[must_use]
    pub fn into_parts(self) -> (ManifestDocument, Vec<String>) {
        (self.document, self.errors)
    }
}

/// Validates the phase, status, and ownership invariants shared with `cursor`.
#[must_use]
pub fn coordination_errors(table: &Table, label: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let phase = table
        .get("phase")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Phase>().ok());
    let status = table
        .get("status")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Status>().ok());

    if phase.is_none() {
        errors.push(format!("{label}: phase must be one of {PHASES}"));
    }

    if status.is_none() {
        errors.push(format!("{label}: status must be one of {STATUSES}"));
    }

    match table.get("next_agent").and_then(Value::as_str) {
        None => errors.push(format!("{label}: next_agent must be a string")),
        Some(next_agent) if status.is_some_and(Status::is_active) && next_agent.is_empty() => {
            errors.push(format!("{label}: active status requires next_agent"));
        }
        Some(next_agent) if status == Some(Status::Complete) && !next_agent.is_empty() => {
            errors.push(format!(
                "{label}: complete status requires empty next_agent"
            ));
        }
        Some(_) => {}
    }

    if phase == Some(Phase::Complete) && status != Some(Status::Complete) {
        errors.push(format!("{label}: complete phase requires complete status"));
    }

    if status == Some(Status::Complete) && phase != Some(Phase::Complete) {
        errors.push(format!("{label}: complete status requires complete phase"));
    }

    errors
}

// Exact equality preserves Python's numeric schema comparison.
#[allow(clippy::float_cmp)]
fn parse_schema(value: &Value) -> Option<ManifestSchema> {
    match value {
        Value::Integer(1) | Value::Boolean(true) => Some(ManifestSchema::V1),
        Value::Integer(2) => Some(ManifestSchema::V2),
        Value::Float(value) if *value == 1.0 => Some(ManifestSchema::V1),
        Value::Float(value) if *value == 2.0 => Some(ManifestSchema::V2),
        _ => None,
    }
}
