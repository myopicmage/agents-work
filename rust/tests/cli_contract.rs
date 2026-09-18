use std::ffi::OsString;
use std::process::Command as ProcessCommand;

use agents_work::cli::{Cli, Command, KindArgument, PhaseArgument, StatusArgument};
use clap::{CommandFactory, Parser, ValueEnum};

fn value_names<T: ValueEnum + 'static>() -> Vec<String> {
    T::value_variants()
        .iter()
        .map(|value| {
            value
                .to_possible_value()
                .expect("CLI value should be visible")
                .get_name()
                .to_owned()
        })
        .collect()
}

#[test]
fn command_and_value_vocabularies_match_python() {
    let commands = Cli::command()
        .get_subcommands()
        .map(|command| command.get_name().to_owned())
        .collect::<Vec<_>>();

    assert_eq!(commands, ["init", "validate", "draft", "publish", "cursor"]);
    assert_eq!(
        value_names::<KindArgument>(),
        ["decision", "plan", "proposal", "response", "review"]
    );
    assert_eq!(
        value_names::<PhaseArgument>(),
        ["complete", "implementation", "planning", "pr_review"]
    );
    assert_eq!(
        value_names::<StatusArgument>(),
        [
            "awaiting_decision",
            "awaiting_review",
            "complete",
            "deferred",
            "drafting",
            "ready_for_implementation",
            "revision_requested",
        ]
    );
}

#[test]
fn validate_accepts_more_than_one_case() {
    let cli = Cli::try_parse_from(["agents-work", "validate", "one", "two"])
        .expect("validate arguments should parse");

    match cli.command {
        Command::Validate { case } => {
            assert_eq!(case.len(), 2);
            assert_eq!(case[0].to_string_lossy(), "one");
            assert_eq!(case[1].to_string_lossy(), "two");
        }
        command => panic!("expected validate, got {command:?}"),
    }
}

#[test]
fn repeated_relationship_flags_accumulate() {
    let cli = Cli::try_parse_from([
        "agents-work",
        "draft",
        "case",
        "--kind",
        "review",
        "--author",
        "claude",
        "--responds-to",
        "025",
        "026",
        "--responds-to",
        "027",
    ])
    .expect("draft arguments should parse");

    match cli.command {
        Command::Draft { responds_to, .. } => {
            assert_eq!(responds_to, ["025", "026", "027"]);
        }
        command => panic!("expected draft, got {command:?}"),
    }
}

#[test]
fn cursor_preserves_underscore_vocabulary_and_empty_agent() {
    let cli = Cli::try_parse_from([
        "agents-work",
        "cursor",
        "case",
        "--phase",
        "pr_review",
        "--status",
        "revision_requested",
        "--next-agent",
        "",
    ])
    .expect("cursor arguments should parse");

    match cli.command {
        Command::Cursor {
            phase,
            status,
            next_agent,
            ..
        } => {
            assert_eq!(phase, Some(PhaseArgument::PrReview));
            assert_eq!(status, Some(StatusArgument::RevisionRequested));
            assert_eq!(next_agent.as_deref(), Some(""));
        }
        command => panic!("expected cursor, got {command:?}"),
    }
}

#[test]
fn unambiguous_long_option_prefixes_match_argparse() {
    let cli = Cli::try_parse_from([
        "agents-work",
        "cursor",
        "case",
        "--stat",
        "deferred",
        "--next-a",
        "claude",
        "--implementation",
        "feature/test",
    ])
    .expect("unambiguous long-option prefixes should parse");

    match cli.command {
        Command::Cursor {
            status,
            next_agent,
            implementation_branch,
            ..
        } => {
            assert_eq!(status, Some(StatusArgument::Deferred));
            assert_eq!(next_agent.as_deref(), Some("claude"));
            assert_eq!(implementation_branch.as_deref(), Some("feature/test"));
        }
        command => panic!("expected cursor, got {command:?}"),
    }

    let cli = Cli::try_parse_from(["agents-work", "cursor", "case", "--s", "deferred"])
        .expect("a one-letter unambiguous prefix should parse");

    assert!(matches!(
        cli.command,
        Command::Cursor {
            status: Some(StatusArgument::Deferred),
            ..
        }
    ));
}

#[test]
fn ambiguous_long_option_prefixes_remain_parser_errors() {
    for option in ["--p", "--pr"] {
        let error = Cli::try_parse_from(["agents-work", "cursor", "case", option, "value"])
            .expect_err("ambiguous prefixes must not choose an option");

        assert_eq!(error.exit_code(), 2);
    }
}

#[test]
fn cursor_rejects_identity_fields_and_unknown_status_at_the_parse_boundary() {
    for arguments in [
        ["agents-work", "cursor", "case", "--id", "other-case"],
        ["agents-work", "cursor", "case", "--status", "in_progress"],
    ] {
        let error = Cli::try_parse_from(arguments)
            .expect_err("cursor grammar should reject an impossible update");

        assert_eq!(error.exit_code(), 2);
    }
}

#[test]
fn help_exits_successfully() {
    let error = Cli::try_parse_from(["agents-work", "--help"])
        .expect_err("help should stop normal argument parsing");

    assert_eq!(error.exit_code(), 0);
}

#[test]
fn invalid_arguments_use_the_parser_exit_class() {
    let error = Cli::try_parse_from(["agents-work", "draft", "case"])
        .expect_err("missing required draft arguments should fail");

    assert_eq!(error.exit_code(), 2);
}

#[test]
fn implicit_help_subcommand_is_not_part_of_the_grammar() {
    let error = Cli::try_parse_from(["agents-work", "help"])
        .expect_err("the Python CLI has no help subcommand");

    assert_eq!(error.exit_code(), 2);
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn abbreviated_long_option_reaches_the_same_python_command_boundary() {
    let reference = std::env::var_os("AGENTS_WORK_PYTHON_REFERENCE")
        .expect("AGENTS_WORK_PYTHON_REFERENCE must name agents_work.py");
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| OsString::from("python3"));
    let case = std::env::temp_dir().join(format!(
        "agents-work-cli-prefix-missing-case-{}",
        std::process::id()
    ));
    assert!(!case.exists(), "the parser fixture must remain absent");

    let python_output = ProcessCommand::new(&python)
        .arg(&reference)
        .arg("cursor")
        .arg(&case)
        .args(["--stat", "deferred"])
        .output()
        .expect("Python parser should run");
    let rust_output = ProcessCommand::new(env!("CARGO_BIN_EXE_agents-work"))
        .arg("cursor")
        .arg(&case)
        .args(["--stat", "deferred"])
        .output()
        .expect("Rust parser should run");

    assert_eq!(rust_output.status.code(), python_output.status.code());
    assert_eq!(rust_output.status.code(), Some(1));
    assert_eq!(rust_output.stdout, python_output.stdout);
    assert_eq!(rust_output.stderr, python_output.stderr);

    let python_ambiguous = ProcessCommand::new(&python)
        .arg(&reference)
        .arg("cursor")
        .arg(&case)
        .args(["--pr", "value"])
        .output()
        .expect("Python parser should reject an ambiguous prefix");
    let rust_ambiguous = ProcessCommand::new(env!("CARGO_BIN_EXE_agents-work"))
        .arg("cursor")
        .arg(&case)
        .args(["--pr", "value"])
        .output()
        .expect("Rust parser should reject an ambiguous prefix");

    assert_eq!(rust_ambiguous.status.code(), python_ambiguous.status.code());
    assert_eq!(rust_ambiguous.status.code(), Some(2));
    assert!(rust_ambiguous.stdout.is_empty());
    assert!(python_ambiguous.stdout.is_empty());
    assert!(!rust_ambiguous.stderr.is_empty());
    assert!(!python_ambiguous.stderr.is_empty());
    assert!(!case.exists(), "parser parity must not create the case");
}
