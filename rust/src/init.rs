//! Creation of inert schema-two shared-work cases.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::case::{create_new_file, path_error, resolve_path, set_file_mode, sync_directory};
use crate::cursor::{CursorError, current_local_timestamp, render_manifest};

/// Borrowed inputs for one case initialization.
#[derive(Clone, Copy, Debug)]
pub struct InitRequest<'a> {
    pub case: &'a Path,
    pub repository: &'a Path,
    pub title: &'a str,
}

/// A validation, environment, or filesystem failure during initialization.
#[derive(Debug)]
pub enum InitError {
    Validation(String),
    Io(io::Error),
    Cursor(CursorError),
}

impl Display for InitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(message) => formatter.write_str(message),
            Self::Io(error) => error.fmt(formatter),
            Self::Cursor(error) => error.fmt(formatter),
        }
    }
}

impl Error for InitError {}

impl From<io::Error> for InitError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<CursorError> for InitError {
    fn from(error: CursorError) -> Self {
        Self::Cursor(error)
    }
}

/// Creates a new inert schema-two case without replacing an existing manifest.
///
/// # Errors
///
/// Returns an error for an invalid path, case ID, title, local clock, manifest
/// rendering, existing manifest, filesystem operation, or output failure.
pub fn init(
    request: InitRequest<'_>,
    standard_output: &mut impl io::Write,
) -> Result<PathBuf, InitError> {
    let case = resolve_path(request.case)?;
    let repository = resolve_path(request.repository)?;

    if !repository.is_dir() {
        return Err(InitError::Validation(format!(
            "{}: repository is not a directory",
            repository.display()
        )));
    }

    let case_id = case
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| is_slug(name))
        .ok_or_else(|| {
            InitError::Validation(format!(
                "{}: case name must be a lowercase slug",
                case.display()
            ))
        })?;

    if request.title.is_empty() {
        return Err(InitError::Validation("title must not be empty".to_owned()));
    }

    let repository_name = repository
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            InitError::Validation(format!(
                "{}: repository name must be valid UTF-8",
                repository.display()
            ))
        })?;
    let repository_path = repository.to_str().ok_or_else(|| {
        InitError::Validation(format!(
            "{}: repository path must be valid UTF-8",
            repository.display()
        ))
    })?;

    fs::create_dir_all(&case).map_err(|error| path_error(&case, &error))?;
    let manifest_path = case.join("work.toml");
    let timestamp = current_local_timestamp()?;
    let mut manifest = Table::new();

    for (field, value) in [
        ("id", case_id),
        ("title", request.title),
        ("repository_name", repository_name),
        ("repository_path", repository_path),
        ("phase", "planning"),
        ("status", "deferred"),
        ("next_agent", ""),
        ("requested_action", ""),
        ("implementation_branch", ""),
        ("pull_request_system", ""),
        ("pull_request_id", ""),
        ("reviewed_commit", ""),
    ] {
        manifest.insert(field.to_owned(), Value::String(value.to_owned()));
    }
    manifest.insert("schema_version".to_owned(), Value::Integer(2));
    manifest.insert("created_at".to_owned(), Value::String(timestamp.clone()));
    manifest.insert("updated_at".to_owned(), Value::String(timestamp));

    let body = render_manifest(&manifest)?;
    let mut output = match create_new_file(&manifest_path, 0o644) {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(InitError::Validation(format!(
                "{}: already exists",
                manifest_path.display()
            )));
        }
        Err(error) => return Err(error.into()),
    };
    set_file_mode(&output, &manifest_path, 0o644)?;
    output
        .write_all(body.as_bytes())
        .map_err(|error| path_error(&manifest_path, &error))?;
    output
        .flush()
        .map_err(|error| path_error(&manifest_path, &error))?;
    output
        .sync_all()
        .map_err(|error| path_error(&manifest_path, &error))?;
    drop(output);
    sync_directory(&case).map_err(|error| path_error(&case, &error))?;

    writeln!(standard_output, "{}", manifest_path.display())?;
    Ok(manifest_path)
}

fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
}
