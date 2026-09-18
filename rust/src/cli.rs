//! Typed command-line grammar for `agents-work`.

use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

use crate::artifact::ArtifactKind;
use crate::manifest::{Phase, Status};

/// Parsed `agents-work` invocation.
#[derive(Debug, Parser)]
#[command(
    name = "agents-work",
    about = "Validate and publish shared agent-work artifacts.",
    disable_help_subcommand = true,
    infer_long_args = true
)]
pub struct Cli {
    /// Operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Supported `agents-work` operations.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new inert shared-work case.
    Init {
        /// Case directory to create.
        case: PathBuf,

        /// Canonical repository directory associated with the case.
        #[arg(long)]
        repository: PathBuf,

        /// Human-readable case title.
        #[arg(long)]
        title: String,
    },

    /// Validate one or more shared-work cases.
    Validate {
        /// Case directories to validate.
        #[arg(required = true, num_args = 1..)]
        case: Vec<PathBuf>,
    },

    /// Write a publishable skeleton with front matter already filled in.
    Draft {
        /// Case directory that owns the draft.
        case: PathBuf,

        /// Artifact kind.
        #[arg(long, value_enum)]
        kind: KindArgument,

        /// Artifact author slug.
        #[arg(long)]
        author: String,

        /// Artifact topic slug, defaulting to the one topic the referenced
        /// artifacts share, otherwise the work.toml id.
        #[arg(long)]
        topic: Option<String>,

        /// Artifact filenames or bare sequence numbers this draft responds to.
        #[arg(
            long,
            num_args = 0..,
            action = ArgAction::Append,
            value_name = "REF"
        )]
        responds_to: Vec<String>,

        /// Artifact filenames or bare sequence numbers this draft supersedes.
        #[arg(
            long,
            num_args = 0..,
            action = ArgAction::Append,
            value_name = "REF"
        )]
        supersedes: Vec<String>,

        /// Output path, defaulting to a hidden draft beside the case.
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Publish a prepared artifact without replacing existing work.
    Publish {
        /// Case directory that will own the artifact.
        case: PathBuf,

        /// Prepared Markdown draft.
        draft: PathBuf,
    },

    /// Move work.toml's coordination fields, refusing an illegal cursor.
    Cursor {
        /// Case directory whose cursor will move.
        case: PathBuf,

        /// Coordination phase.
        #[arg(long, value_enum)]
        phase: Option<PhaseArgument>,

        /// Coordination status.
        #[arg(long, value_enum)]
        status: Option<StatusArgument>,

        /// Agent expected to act next, or an empty string to clear it.
        #[arg(long)]
        next_agent: Option<String>,

        /// Work requested from the next agent.
        #[arg(long = "action", value_name = "REQUESTED_ACTION")]
        requested_action: Option<String>,

        /// Implementation branch associated with the case.
        #[arg(long)]
        implementation_branch: Option<String>,

        /// Pull request provider associated with the case.
        #[arg(long = "pr-system")]
        pull_request_system: Option<String>,

        /// Pull request identifier associated with the case.
        #[arg(long = "pr-id")]
        pull_request_id: Option<String>,

        /// Exact commit most recently reviewed.
        #[arg(long)]
        reviewed_commit: Option<String>,
    },
}

/// Artifact kind accepted by the current Python CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum KindArgument {
    Decision,
    Plan,
    Proposal,
    Response,
    Review,
}

impl From<KindArgument> for ArtifactKind {
    fn from(value: KindArgument) -> Self {
        match value {
            KindArgument::Decision => Self::Decision,
            KindArgument::Plan => Self::Plan,
            KindArgument::Proposal => Self::Proposal,
            KindArgument::Response => Self::Response,
            KindArgument::Review => Self::Review,
        }
    }
}

/// Coordination phase accepted by the current Python CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum PhaseArgument {
    Complete,
    Implementation,
    Planning,
    #[value(name = "pr_review")]
    PrReview,
}

impl From<PhaseArgument> for Phase {
    fn from(value: PhaseArgument) -> Self {
        match value {
            PhaseArgument::Complete => Self::Complete,
            PhaseArgument::Implementation => Self::Implementation,
            PhaseArgument::Planning => Self::Planning,
            PhaseArgument::PrReview => Self::PrReview,
        }
    }
}

/// Coordination status accepted by the current Python CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum StatusArgument {
    #[value(name = "awaiting_decision")]
    AwaitingDecision,
    #[value(name = "awaiting_review")]
    AwaitingReview,
    Complete,
    Deferred,
    Drafting,
    #[value(name = "ready_for_implementation")]
    ReadyForImplementation,
    #[value(name = "revision_requested")]
    RevisionRequested,
}

impl From<StatusArgument> for Status {
    fn from(value: StatusArgument) -> Self {
        match value {
            StatusArgument::AwaitingDecision => Self::AwaitingDecision,
            StatusArgument::AwaitingReview => Self::AwaitingReview,
            StatusArgument::Complete => Self::Complete,
            StatusArgument::Deferred => Self::Deferred,
            StatusArgument::Drafting => Self::Drafting,
            StatusArgument::ReadyForImplementation => Self::ReadyForImplementation,
            StatusArgument::RevisionRequested => Self::RevisionRequested,
        }
    }
}
