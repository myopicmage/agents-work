use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use agents_work::cli::{Cli, Command};
use agents_work::cursor::{CursorRequest, CursorUpdates, cursor};
use agents_work::draft::{DraftRequest, draft};
use agents_work::init::{InitRequest, init};
use agents_work::publish::publish;
use agents_work::validate::validate_cases;
use clap::Parser;

fn main() -> ExitCode {
    let invocation = Cli::parse();

    match invocation.command {
        Command::Init {
            case,
            repository,
            title,
        } => run_init(&case, &repository, &title),
        Command::Validate { case } => {
            let mut standard_output = io::stdout().lock();
            let mut standard_error = io::stderr().lock();

            match validate_cases(&case, &mut standard_output, &mut standard_error) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(error) => {
                    let _ = writeln!(standard_error, "error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Draft {
            case,
            kind,
            author,
            topic,
            responds_to,
            supersedes,
            output,
        } => {
            let mut standard_output = io::stdout().lock();
            let mut standard_error = io::stderr().lock();
            let request = DraftRequest {
                case: &case,
                kind: kind.into(),
                author: &author,
                topic: topic.as_deref(),
                responds_to: &responds_to,
                supersedes: &supersedes,
                output: output.as_deref(),
            };

            match draft(request, &mut standard_output) {
                Ok(_) => ExitCode::SUCCESS,
                Err(error) => {
                    let _ = writeln!(standard_error, "error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Publish {
            case,
            draft: draft_path,
        } => run_publish(&case, &draft_path),
        Command::Cursor {
            case,
            phase,
            status,
            next_agent,
            requested_action,
            implementation_branch,
            pull_request_system,
            pull_request_id,
            reviewed_commit,
        } => {
            let mut standard_output = io::stdout().lock();
            let mut standard_error = io::stderr().lock();
            let request = CursorRequest {
                case: &case,
                updates: CursorUpdates {
                    phase: phase.map(Into::into),
                    status: status.map(Into::into),
                    next_agent: next_agent.as_deref(),
                    requested_action: requested_action.as_deref(),
                    implementation_branch: implementation_branch.as_deref(),
                    pull_request_system: pull_request_system.as_deref(),
                    pull_request_id: pull_request_id.as_deref(),
                    reviewed_commit: reviewed_commit.as_deref(),
                },
            };

            match cursor(request, &mut standard_output) {
                Ok(_) => ExitCode::SUCCESS,
                Err(error) => {
                    let _ = writeln!(standard_error, "error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn run_publish(case: &Path, draft_path: &Path) -> ExitCode {
    let mut standard_output = io::stdout().lock();
    let mut standard_error = io::stderr().lock();

    match publish(case, draft_path, &mut standard_output, &mut standard_error) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(standard_error, "error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_init(case: &Path, repository: &Path, title: &str) -> ExitCode {
    let mut standard_output = io::stdout().lock();
    let mut standard_error = io::stderr().lock();
    let request = InitRequest {
        case,
        repository,
        title,
    };

    match init(request, &mut standard_output) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(standard_error, "error: {error}");
            ExitCode::FAILURE
        }
    }
}
