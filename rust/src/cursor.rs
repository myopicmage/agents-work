//! Typed cursor transitions over a deliberately raw existing manifest.

use std::error::Error;
use std::fmt::{self, Display, Formatter, Write as _};
use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::case::{
    create_new_file, normalize_newlines, path_error, remove_if_exists, resolve_path, set_file_mode,
    sync_directory,
};
use crate::manifest::{Phase, Status, coordination_errors};

/// Borrowed cursor inputs before the case path is resolved.
#[derive(Clone, Copy, Debug)]
pub struct CursorRequest<'a> {
    pub case: &'a Path,
    pub updates: CursorUpdates<'a>,
}

/// Coordination fields supplied by one cursor invocation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CursorUpdates<'a> {
    pub phase: Option<Phase>,
    pub status: Option<Status>,
    pub next_agent: Option<&'a str>,
    pub requested_action: Option<&'a str>,
    pub implementation_branch: Option<&'a str>,
    pub pull_request_system: Option<&'a str>,
    pub pull_request_id: Option<&'a str>,
    pub reviewed_commit: Option<&'a str>,
}

impl CursorUpdates<'_> {
    /// Returns whether the invocation supplied no writable fields.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.phase.is_none()
            && self.status.is_none()
            && self.next_agent.is_none()
            && self.requested_action.is_none()
            && self.implementation_branch.is_none()
            && self.pull_request_system.is_none()
            && self.pull_request_id.is_none()
            && self.reviewed_commit.is_none()
    }

    fn apply(self, manifest: &mut Table) {
        if let Some(phase) = self.phase {
            manifest.insert("phase".to_owned(), Value::String(phase.to_string()));
        }

        if let Some(status) = self.status {
            manifest.insert("status".to_owned(), Value::String(status.to_string()));
        }

        for (field, value) in [
            ("next_agent", self.next_agent),
            ("requested_action", self.requested_action),
            ("implementation_branch", self.implementation_branch),
            ("pull_request_system", self.pull_request_system),
            ("pull_request_id", self.pull_request_id),
            ("reviewed_commit", self.reviewed_commit),
        ] {
            if let Some(value) = value {
                manifest.insert(field.to_owned(), Value::String(value.to_owned()));
            }
        }

        let resting = manifest
            .get("status")
            .and_then(Value::as_str)
            .and_then(|status| status.parse::<Status>().ok())
            .is_some_and(Status::is_resting);

        if resting && self.next_agent.is_none() {
            manifest.insert("next_agent".to_owned(), Value::String(String::new()));
        }
    }
}

/// Exact manifest state and bytes after a legal cursor transition.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorDocument {
    manifest: Table,
    body: String,
}

impl CursorDocument {
    /// Applies typed updates to a raw schema-two manifest and renders it.
    ///
    /// Keeping the existing table raw is deliberate: a cursor update must be
    /// able to repair invalid coordination values already on disk.
    ///
    /// # Errors
    ///
    /// Returns a Python-compatible validation error for an empty update,
    /// legacy schema, illegal merged cursor, unsupported manifest value, or a
    /// renderer that does not parse back to the merged table.
    pub fn build(
        manifest: Table,
        updates: CursorUpdates<'_>,
        updated_at: &str,
        label: &str,
    ) -> Result<Self, CursorError> {
        Self::build_with_clock(manifest, updates, label, || Ok(updated_at.to_owned()))
    }

    fn build_with_clock(
        mut manifest: Table,
        updates: CursorUpdates<'_>,
        label: &str,
        clock: impl FnOnce() -> Result<String, CursorError>,
    ) -> Result<Self, CursorError> {
        if updates.is_empty() {
            return Err(CursorError::validation(
                "cursor requires at least one field to set",
            ));
        }

        require_schema_two(&manifest, label)?;
        updates.apply(&mut manifest);
        manifest.insert("updated_at".to_owned(), Value::String(clock()?));

        let errors = coordination_errors(&manifest, label);

        if !errors.is_empty() {
            return Err(CursorError::validation(errors.join("\n")));
        }

        let body = render_manifest(&manifest)?;
        let round_trip = body.parse::<Table>().map_err(|_| {
            CursorError::validation(format!("{label}: rendered manifest does not round-trip"))
        })?;

        if round_trip != manifest {
            return Err(CursorError::validation(format!(
                "{label}: rendered manifest does not round-trip"
            )));
        }

        Ok(Self { manifest, body })
    }

    /// Returns the merged manifest values.
    #[must_use]
    pub fn manifest(&self) -> &Table {
        &self.manifest
    }

    /// Returns the exact UTF-8 text written to `work.toml`.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

/// A cursor validation, environment, or filesystem failure.
#[derive(Debug)]
pub enum CursorError {
    Validation(String),
    Io(io::Error),
    Random(getrandom::Error),
    LocalOffset(time::error::IndeterminateOffset),
}

impl CursorError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }
}

impl Display for CursorError {
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

impl Error for CursorError {}

impl From<io::Error> for CursorError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Moves `work.toml` using the local clock and atomic filesystem replacement.
///
/// # Errors
///
/// Returns an error for missing or invalid input, an illegal merged cursor,
/// unsupported retained manifest values, unavailable clock or randomness, a
/// failed durability operation, or output failure.
pub fn cursor(
    request: CursorRequest<'_>,
    standard_output: &mut impl io::Write,
) -> Result<PathBuf, CursorError> {
    cursor_with(
        request,
        current_local_timestamp,
        |temporary, manifest| fs::rename(temporary, manifest),
        sync_directory,
        standard_output,
    )
}

fn cursor_with(
    request: CursorRequest<'_>,
    clock: impl FnOnce() -> Result<String, CursorError>,
    replacer: impl FnOnce(&Path, &Path) -> io::Result<()>,
    directory_sync: impl FnOnce(&Path) -> io::Result<()>,
    standard_output: &mut impl io::Write,
) -> Result<PathBuf, CursorError> {
    let case = resolve_path(request.case)?;
    let manifest_path = case.join("work.toml");

    if !manifest_path.is_file() {
        return Err(CursorError::validation(format!(
            "{}: missing work.toml",
            case.display()
        )));
    }

    if request.updates.is_empty() {
        return Err(CursorError::validation(
            "cursor requires at least one field to set",
        ));
    }

    let manifest = read_cursor_manifest(&manifest_path)?;
    let label = manifest_path.display().to_string();
    let document = CursorDocument::build_with_clock(manifest, request.updates, &label, clock)?;
    let (temporary_path, temporary) = create_temporary(&case)?;
    let replacement = replace_manifest(
        ManifestReplacement {
            case: &case,
            temporary_path: &temporary_path,
            temporary,
            manifest_path: &manifest_path,
            body: document.body.as_bytes(),
        },
        replacer,
        directory_sync,
    );
    let cleanup = remove_if_exists(&temporary_path);

    if let Err(error) = cleanup {
        return Err(error.into());
    }

    replacement?;
    let phase = manifest_string(&document.manifest, "phase");
    let status = manifest_string(&document.manifest, "status");
    let next_agent = manifest_string(&document.manifest, "next_agent");
    let agent = if next_agent.is_empty() {
        "(none)"
    } else {
        next_agent
    };
    writeln!(
        standard_output,
        "{}: phase={phase} status={status} next_agent={agent}",
        manifest_path.display()
    )?;
    Ok(manifest_path)
}

struct ManifestReplacement<'a> {
    case: &'a Path,
    temporary_path: &'a Path,
    temporary: File,
    manifest_path: &'a Path,
    body: &'a [u8],
}

fn replace_manifest(
    replacement: ManifestReplacement<'_>,
    replacer: impl FnOnce(&Path, &Path) -> io::Result<()>,
    directory_sync: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), CursorError> {
    let ManifestReplacement {
        case,
        temporary_path,
        mut temporary,
        manifest_path,
        body,
    } = replacement;
    set_file_mode(&temporary, temporary_path, 0o644)?;
    temporary
        .write_all(body)
        .map_err(|error| path_error(temporary_path, &error))?;
    temporary
        .flush()
        .map_err(|error| path_error(temporary_path, &error))?;
    temporary
        .sync_all()
        .map_err(|error| path_error(temporary_path, &error))?;
    drop(temporary);
    replacer(temporary_path, manifest_path).map_err(|error| path_error(manifest_path, &error))?;
    directory_sync(case).map_err(|error| path_error(case, &error))?;
    Ok(())
}

fn read_cursor_manifest(path: &Path) -> Result<Table, CursorError> {
    let data = fs::read(path).map_err(|error| invalid_manifest(path, &error))?;
    let text = String::from_utf8(data).map_err(|error| {
        CursorError::validation(format!("{}: invalid work.toml: {error}", path.display()))
    })?;
    normalize_newlines(text).parse::<Table>().map_err(|error| {
        CursorError::validation(format!("{}: invalid work.toml: {error}", path.display()))
    })
}

fn invalid_manifest(path: &Path, error: &io::Error) -> CursorError {
    CursorError::validation(format!("{}: invalid work.toml: {error}", path.display()))
}

fn current_local_timestamp() -> Result<String, CursorError> {
    let current = time::OffsetDateTime::now_local().map_err(CursorError::LocalOffset)?;
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
    Ok(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{offset}",
        current.year(),
        u8::from(current.month()),
        current.day(),
        current.hour(),
        current.minute(),
        current.second()
    ))
}

fn create_temporary(case: &Path) -> Result<(PathBuf, File), CursorError> {
    loop {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).map_err(CursorError::Random)?;
        let path = case.join(format!(".work-{:016x}.tmp", u64::from_be_bytes(random)));

        match create_new_file(&path, 0o600) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn manifest_string<'a>(manifest: &'a Table, field: &str) -> &'a str {
    manifest.get(field).and_then(Value::as_str).unwrap_or("")
}

fn require_schema_two(manifest: &Table, label: &str) -> Result<(), CursorError> {
    if manifest.get("schema_version").is_some_and(is_python_two) {
        return Ok(());
    }

    Err(CursorError::validation(format!(
        "{label}: cursor requires schema_version 2; migrate a legacy manifest by hand first"
    )))
}

// Python compares values with `!= 2`, so TOML float 2.0 is also accepted.
#[allow(clippy::float_cmp)]
fn is_python_two(value: &Value) -> bool {
    matches!(value, Value::Integer(2)) || matches!(value, Value::Float(value) if *value == 2.0)
}

fn render_manifest(manifest: &Table) -> Result<String, CursorError> {
    let field_groups: [&[&str]; 5] = [
        &["schema_version", "id", "title"],
        &["repository_name", "repository_path"],
        &["phase", "status", "next_agent", "requested_action"],
        &[
            "implementation_branch",
            "pull_request_system",
            "pull_request_id",
            "reviewed_commit",
        ],
        &["created_at", "updated_at"],
    ];
    let known = field_groups
        .iter()
        .flat_map(|group| group.iter())
        .copied()
        .collect::<Vec<_>>();
    let mut groups = Vec::new();

    for fields in field_groups {
        let mut lines = Vec::new();

        for &field in fields {
            if let Some(value) = manifest.get(field) {
                lines.push(format!("{field} = {}", render_toml_value(field, value)?));
            }
        }

        if !lines.is_empty() {
            groups.push(lines.join("\n"));
        }
    }

    let mut unknown = manifest
        .keys()
        .filter(|field| !known.contains(&field.as_str()))
        .collect::<Vec<_>>();
    unknown.sort_unstable();

    if !unknown.is_empty() {
        let mut lines = Vec::new();

        for field in unknown {
            lines.push(format!(
                "{field} = {}",
                render_toml_value(field, &manifest[field])?
            ));
        }

        groups.push(lines.join("\n"));
    }

    Ok(format!("{}\n", groups.join("\n\n")))
}

fn render_toml_value(field: &str, value: &Value) -> Result<String, CursorError> {
    match value {
        Value::Boolean(value) => Ok(value.to_string()),
        Value::Integer(value) => Ok(value.to_string()),
        Value::String(value) => Ok(quote_toml_string(value)),
        unsupported => Err(CursorError::validation(format!(
            "cannot render manifest field {field} of type {}",
            python_type_name(unsupported)
        ))),
    }
}

fn python_type_name(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "str",
        Value::Integer(_) => "int",
        Value::Float(_) => "float",
        Value::Boolean(_) => "bool",
        Value::Datetime(value) if value.date.is_some() && value.time.is_some() => "datetime",
        Value::Datetime(value) if value.date.is_some() => "date",
        Value::Datetime(_) => "time",
        Value::Array(_) => "list",
        Value::Table(_) => "dict",
    }
}

fn quote_toml_string(value: &str) -> String {
    let mut quoted = String::from("\"");

    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\u{08}' => quoted.push_str("\\b"),
            '\u{0c}' => quoted.push_str("\\f"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{00}'..='\u{1f}' | '\u{7f}' => {
                write!(quoted, "\\u{:04x}", u32::from(character))
                    .expect("writing to a String cannot fail");
            }
            character if !character.is_ascii() && character.len_utf16() == 1 => {
                write!(quoted, "\\u{:04x}", u32::from(character))
                    .expect("writing to a String cannot fail");
            }
            character => quoted.push(character),
        }
    }

    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{CursorError, CursorRequest, CursorUpdates, cursor_with};
    use crate::manifest::Status;

    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        case: PathBuf,
    }

    impl Fixture {
        fn new(label: &str, schema_version: i64) -> Self {
            let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "agents-work-cursor-unit-{}-{label}-{unique}",
                std::process::id()
            ));
            let case = root.join("case");
            fs::create_dir_all(&case).expect("temporary case should be created");
            fs::write(
                case.join("work.toml"),
                format!(
                    r#"schema_version = {schema_version}
phase = "planning"
status = "drafting"
next_agent = "codex"
updated_at = "old"
"#
                ),
            )
            .expect("manifest should be written");
            Self { root, case }
        }

        fn resolved_case(&self) -> PathBuf {
            fs::canonicalize(&self.case).expect("case should resolve")
        }

        fn manifest_path(&self) -> PathBuf {
            self.resolved_case().join("work.toml")
        }

        fn request(&self) -> CursorRequest<'_> {
            CursorRequest {
                case: &self.case,
                updates: CursorUpdates {
                    status: Some(Status::Deferred),
                    ..CursorUpdates::default()
                },
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn replacement_failure_retains_the_original_manifest() {
        let fixture = Fixture::new("replace-failure", 2);
        let before = fs::read(fixture.manifest_path()).expect("manifest should be readable");
        let mut standard_output = Vec::new();

        let error = cursor_with(
            fixture.request(),
            || Ok(timestamp().to_owned()),
            |_temporary, _manifest| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected replace failure",
                ))
            },
            |_case| Ok(()),
            &mut standard_output,
        )
        .expect_err("injected replacement failure should escape");

        assert_eq!(
            error.to_string(),
            format!(
                "{}: injected replace failure",
                fixture.manifest_path().display()
            )
        );
        assert_eq!(
            fs::read(fixture.manifest_path()).expect("original manifest should remain"),
            before
        );
        assert_no_temporary_work_file(&fixture.case);
        assert!(standard_output.is_empty());
    }

    #[test]
    fn directory_sync_failure_retains_the_replaced_manifest() {
        let fixture = Fixture::new("sync-failure", 2);
        let mut standard_output = Vec::new();

        let error = cursor_with(
            fixture.request(),
            || Ok(timestamp().to_owned()),
            |temporary, manifest| fs::rename(temporary, manifest),
            |_case| Err(io::Error::other("injected directory sync failure")),
            &mut standard_output,
        )
        .expect_err("injected directory sync failure should escape");
        let table = fs::read_to_string(fixture.manifest_path())
            .expect("replacement should remain readable")
            .parse::<toml::Table>()
            .expect("replacement should remain TOML");

        assert_eq!(
            error.to_string(),
            format!(
                "{}: injected directory sync failure",
                fixture.resolved_case().display()
            )
        );
        assert_eq!(table["status"].as_str(), Some("deferred"));
        assert_eq!(table["next_agent"].as_str(), Some(""));
        assert_eq!(table["updated_at"].as_str(), Some(timestamp()));
        assert_no_temporary_work_file(&fixture.case);
        assert!(standard_output.is_empty());
    }

    #[test]
    fn schema_failure_precedes_the_clock_and_all_filesystem_writes() {
        let fixture = Fixture::new("schema-before-clock", 1);
        let before = fs::read(fixture.manifest_path()).expect("manifest should be readable");
        let clock_called = Cell::new(false);

        let error = cursor_with(
            fixture.request(),
            || {
                clock_called.set(true);
                Ok(timestamp().to_owned())
            },
            |_temporary, _manifest| Ok(()),
            |_case| Ok(()),
            &mut Vec::new(),
        )
        .expect_err("legacy schema should fail before the clock");

        assert_eq!(
            error.to_string(),
            format!(
                "{}: cursor requires schema_version 2; migrate a legacy manifest by hand first",
                fixture.manifest_path().display()
            )
        );
        assert!(!clock_called.get());
        assert_eq!(
            fs::read(fixture.manifest_path()).expect("legacy manifest should remain"),
            before
        );
        assert_no_temporary_work_file(&fixture.case);
    }

    #[test]
    fn stdout_failure_happens_after_the_replacement_commit_point() {
        struct BrokenOutput;

        impl io::Write for BrokenOutput {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected stdout failure",
                ))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let fixture = Fixture::new("stdout-failure", 2);

        let error = cursor_with(
            fixture.request(),
            || Ok(timestamp().to_owned()),
            |temporary, manifest| fs::rename(temporary, manifest),
            |_case| Ok(()),
            &mut BrokenOutput,
        )
        .expect_err("injected stdout failure should escape");
        let table = fs::read_to_string(fixture.manifest_path())
            .expect("committed manifest should remain readable")
            .parse::<toml::Table>()
            .expect("committed manifest should remain TOML");

        assert!(matches!(
            error,
            CursorError::Io(ref error) if error.kind() == io::ErrorKind::BrokenPipe
        ));
        assert_eq!(table["status"].as_str(), Some("deferred"));
        assert_eq!(table["updated_at"].as_str(), Some(timestamp()));
        assert_no_temporary_work_file(&fixture.case);
    }

    fn timestamp() -> &'static str {
        "2026-08-10T09:15:30+09:00"
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
}
