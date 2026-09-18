//! No-clobber artifact publication with Python-compatible rollback.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use toml::Table;

use crate::artifact::{
    ArtifactId, ArtifactKind, ArtifactMetadata, Slug, TITLE_PLACEHOLDER, front_matter_body,
    generated_draft_id, has_title_placeholder, parse_front_matter,
};
use crate::case::{
    create_new_file, discovered_artifacts, path_error, remove_if_exists, resolve_path,
    set_file_mode, sync_directory,
};
use crate::validate::sha256_bytes;

/// A publication validation, randomness, or filesystem failure.
#[derive(Debug)]
pub enum PublishError {
    Validation(String),
    Io(io::Error),
    Random(getrandom::Error),
}

impl PublishError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }
}

impl Display for PublishError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(message) => formatter.write_str(message),
            Self::Io(error) => error.fmt(formatter),
            Self::Random(error) => write!(formatter, "system randomness unavailable: {error}"),
        }
    }
}

impl Error for PublishError {}

impl From<io::Error> for PublishError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Publishes a prepared Markdown artifact without replacing existing work.
///
/// A draft that `draft` generated for this artifact is removed once the
/// artifact is durable; failing to remove it is a warning, not an error.
///
/// # Errors
///
/// Returns an error when the case or draft is missing, prepared metadata is
/// invalid, a relationship target is absent, publication would clobber an
/// existing path, or a durability operation fails.
pub fn publish(
    case: &Path,
    draft: &Path,
    standard_output: &mut impl Write,
    standard_error: &mut impl Write,
) -> Result<PathBuf, PublishError> {
    publish_with(
        case,
        draft,
        standard_output,
        standard_error,
        |temporary, final_path| fs::hard_link(temporary, final_path),
        sync_directory,
    )
}

fn publish_with(
    case: &Path,
    draft: &Path,
    standard_output: &mut impl Write,
    standard_error: &mut impl Write,
    linker: impl FnOnce(&Path, &Path) -> io::Result<()>,
    mut directory_sync: impl FnMut(&Path) -> io::Result<()>,
) -> Result<PathBuf, PublishError> {
    let case = resolve_path(case)?;
    let draft = resolve_path(draft)?;

    if !case.join("work.toml").is_file() {
        return Err(PublishError::validation(format!(
            "{}: missing work.toml",
            case.display()
        )));
    }

    if !draft.is_file() {
        return Err(PublishError::validation(format!(
            "{}: draft does not exist",
            draft.display()
        )));
    }

    let metadata = validate_prepared_artifact(&draft, &case)?;
    let filename = metadata.filename();
    let final_path = case.join(&filename);
    let sidecar_path = case.join(format!("{filename}.sha256"));

    if final_path.exists() || sidecar_path.exists() {
        return Err(PublishError::validation(format!(
            "{filename}: final artifact or sidecar already exists; use a new ID"
        )));
    }

    // Python deliberately rereads after validation. Preserve that concurrency
    // behavior rather than silently turning validation into a snapshot lock.
    let data = read_bytes(&draft)?;
    let digest = sha256_bytes(&data);
    let sidecar_data = format!("{digest}  {filename}\n").into_bytes();
    let (temporary_path, temporary) = create_temporary(&case)?;
    let publication = install_temporary(
        Publication {
            case: &case,
            temporary_path: &temporary_path,
            temporary,
            data: &data,
            sidecar_path: &sidecar_path,
            sidecar_data: &sidecar_data,
            final_path: &final_path,
        },
        linker,
        &mut directory_sync,
    );
    let cleanup = remove_if_exists(&temporary_path);

    if let Err(error) = cleanup {
        return Err(error.into());
    }

    publication?;

    if is_generated_draft(&draft, &case, metadata.artifact_id) {
        remove_generated_draft(&draft, &case, directory_sync, standard_error);
    }

    writeln!(standard_output, "{}", final_path.display())?;
    Ok(final_path)
}

fn is_generated_draft(draft: &Path, case: &Path, artifact_id: ArtifactId) -> bool {
    draft.parent() == Some(case)
        && draft
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(generated_draft_id)
            == Some(artifact_id)
}

// The artifact is already durable, so a failure here is a warning rather than
// an error: reporting failure would invite a retry that can only hit
// no-clobber.
fn remove_generated_draft(
    draft: &Path,
    case: &Path,
    directory_sync: impl FnOnce(&Path) -> io::Result<()>,
    standard_error: &mut impl Write,
) {
    let removal = match fs::remove_file(draft) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => directory_sync(case),
    };

    if let Err(error) = removal {
        let _ = writeln!(
            standard_error,
            "warning: {}: published, but the draft was not removed: {error}",
            draft.display()
        );
    }
}

fn validate_prepared_artifact(draft: &Path, case: &Path) -> Result<ArtifactMetadata, PublishError> {
    let data = read_bytes(draft)?;
    let table = parse_front_matter(&data, draft).map_err(|error| {
        PublishError::validation(format!("{error}\n{}", draft_hint(case, None)))
    })?;
    let metadata = ArtifactMetadata::parse(&table, draft).map_err(|errors| {
        PublishError::validation(format!("{errors}\n{}", draft_hint(case, Some(&table))))
    })?;
    let discovered = discovered_artifacts(case)?
        .iter()
        .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let mut errors = Vec::new();

    for (relationship, targets) in [
        ("responds_to", &metadata.responds_to),
        ("supersedes", &metadata.supersedes),
    ] {
        for target in targets {
            if !discovered.contains(target.as_str()) {
                errors.push(format!(
                    "{}: {relationship} target missing: {target}",
                    file_name(draft)
                ));
            }
        }
    }

    // Front matter parsed above, so the delimiters are known to be present.
    if front_matter_body(&data).is_some_and(has_title_placeholder) {
        errors.push(format!(
            "{}: body still contains the draft placeholder line \
             '{TITLE_PLACEHOLDER}'; replace it with the artifact's title",
            file_name(draft)
        ));
    }

    if !errors.is_empty() {
        return Err(PublishError::validation(errors.join("\n")));
    }

    Ok(metadata)
}

fn draft_hint(case: &Path, metadata: Option<&Table>) -> String {
    let kind = metadata
        .and_then(|table| table.get("kind"))
        .and_then(toml::Value::as_str)
        .and_then(|value| value.parse::<ArtifactKind>().ok())
        .map_or_else(|| "KIND".to_owned(), |value| value.to_string());
    let author = metadata
        .and_then(|table| table.get("author"))
        .and_then(toml::Value::as_str)
        .and_then(|value| value.parse::<Slug>().ok())
        .map_or_else(|| "AUTHOR".to_owned(), |value| value.to_string());

    format!(
        "hint: agents-work draft {} --kind {kind} --author {author} generates valid front matter",
        shell_quote(&case.display().to_string())
    )
}

struct Publication<'a> {
    case: &'a Path,
    temporary_path: &'a Path,
    temporary: File,
    data: &'a [u8],
    sidecar_path: &'a Path,
    sidecar_data: &'a [u8],
    final_path: &'a Path,
}

fn install_temporary(
    publication: Publication<'_>,
    linker: impl FnOnce(&Path, &Path) -> io::Result<()>,
    directory_sync: &mut impl FnMut(&Path) -> io::Result<()>,
) -> Result<(), PublishError> {
    let Publication {
        case,
        temporary_path,
        mut temporary,
        data,
        sidecar_path,
        sidecar_data,
        final_path,
    } = publication;
    set_file_mode(&temporary, temporary_path, 0o644)?;
    temporary
        .write_all(data)
        .map_err(|error| path_error(temporary_path, &error))?;
    temporary
        .flush()
        .map_err(|error| path_error(temporary_path, &error))?;
    temporary
        .sync_all()
        .map_err(|error| path_error(temporary_path, &error))?;
    drop(temporary);

    // Python enters its sidecar rollback region only after O_EXCL succeeds.
    // A competing writer's path therefore never belongs to this transaction.
    let mut sidecar = create_new_file(sidecar_path, 0o644)?;
    let publication = (|| {
        sidecar
            .write_all(sidecar_data)
            .map_err(|error| path_error(sidecar_path, &error))?;
        sidecar
            .flush()
            .map_err(|error| path_error(sidecar_path, &error))?;
        sidecar
            .sync_all()
            .map_err(|error| path_error(sidecar_path, &error))?;
        drop(sidecar);

        if let Err(error) = linker(temporary_path, final_path) {
            let link_error = PublishError::Io(path_error(final_path, &error));
            remove_if_exists(sidecar_path)?;
            return Err(link_error);
        }

        directory_sync(case).map_err(|error| path_error(case, &error))?;
        Ok(())
    })();

    match publication {
        Err(error) if !final_path.exists() => {
            remove_if_exists(sidecar_path)?;
            Err(error)
        }
        result => result,
    }
}

fn create_temporary(case: &Path) -> Result<(PathBuf, File), PublishError> {
    loop {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).map_err(PublishError::Random)?;
        let path = case.join(format!(".artifact-{:016x}.tmp", u64::from_be_bytes(random)));

        match create_new_file(&path, 0o600) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn read_bytes(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path).map_err(|error| path_error(path, &error))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty() && value.bytes().all(is_shell_safe) {
        return value.to_owned();
    }

    let mut quoted = String::from("'");

    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\"'\"'");
        } else {
            quoted.push(character);
        }
    }

    quoted.push('\'');
    quoted
}

fn is_shell_safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'
        )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{publish_with, shell_quote};

    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
    const GENERATED_DRAFT: &str = ".draft-001-test-plan-codex-a1b2c3.md";

    struct Fixture {
        root: PathBuf,
        case: PathBuf,
        draft: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "agents-work-publish-unit-{}-{label}-{unique}",
                std::process::id()
            ));
            let case = root.join("case");
            let draft = root.join("draft.md");
            fs::create_dir_all(&case).expect("temporary case should be created");
            fs::write(case.join("work.toml"), manifest()).expect("manifest should be written");
            fs::write(&draft, artifact()).expect("draft should be written");
            Self { root, case, draft }
        }

        fn generated_draft(&self) -> PathBuf {
            let draft = self.case.join(GENERATED_DRAFT);
            fs::write(&draft, artifact()).expect("generated draft should be written");
            draft
        }

        fn final_path(&self) -> PathBuf {
            self.resolved_case().join("001-test-plan-codex-a1b2c3.md")
        }

        fn sidecar_path(&self) -> PathBuf {
            self.resolved_case()
                .join("001-test-plan-codex-a1b2c3.md.sha256")
        }

        fn resolved_case(&self) -> PathBuf {
            fs::canonicalize(&self.case).expect("case should resolve")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn link_failure_removes_sidecar_and_temporary_file() {
        let fixture = Fixture::new("link-failure");
        let mut standard_output = Vec::new();

        let error = publish_with(
            &fixture.case,
            &fixture.draft,
            &mut standard_output,
            &mut Vec::new(),
            |_temporary, _final_path| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected link failure",
                ))
            },
            |_case| Ok(()),
        )
        .expect_err("injected link failure should escape");

        assert_eq!(
            error.to_string(),
            format!("{}: injected link failure", fixture.final_path().display())
        );
        assert!(!fixture.final_path().exists());
        assert!(!fixture.sidecar_path().exists());
        assert_no_temporary_artifact(&fixture.case);
        assert!(fixture.draft.is_file());
        assert!(standard_output.is_empty());
    }

    #[test]
    fn directory_sync_failure_retains_committed_artifact_and_sidecar() {
        let fixture = Fixture::new("directory-sync-failure");
        let mut standard_output = Vec::new();

        let error = publish_with(
            &fixture.case,
            &fixture.draft,
            &mut standard_output,
            &mut Vec::new(),
            |temporary, final_path| fs::hard_link(temporary, final_path),
            |_case| Err(io::Error::other("injected directory sync failure")),
        )
        .expect_err("injected directory sync failure should escape");

        assert_eq!(
            error.to_string(),
            format!(
                "{}: injected directory sync failure",
                fixture.resolved_case().display()
            )
        );
        assert_eq!(
            fs::read(fixture.final_path()).expect("committed artifact should remain"),
            artifact().as_bytes()
        );
        assert!(fixture.sidecar_path().is_file());
        assert_no_temporary_artifact(&fixture.case);
        assert!(fixture.draft.is_file());
        assert!(standard_output.is_empty());
    }

    #[test]
    fn directory_sync_failure_keeps_a_generated_draft() {
        let fixture = Fixture::new("directory-sync-failure-generated");
        let draft = fixture.generated_draft();

        publish_with(
            &fixture.case,
            &draft,
            &mut Vec::new(),
            &mut Vec::new(),
            |temporary, final_path| fs::hard_link(temporary, final_path),
            |_case| Err(io::Error::other("injected directory sync failure")),
        )
        .expect_err("injected directory sync failure should escape");

        assert!(fixture.final_path().is_file());
        assert!(draft.is_file());
    }

    #[test]
    fn draft_removal_failure_is_a_warning_after_publishing() {
        let fixture = Fixture::new("draft-removal-failure");
        let draft = fixture.generated_draft();
        let mut standard_output = Vec::new();
        let mut standard_error = Vec::new();
        let mut syncs = 0;

        let published = publish_with(
            &fixture.case,
            &draft,
            &mut standard_output,
            &mut standard_error,
            |temporary, final_path| fs::hard_link(temporary, final_path),
            |_case| {
                syncs += 1;

                if syncs == 1 {
                    return Ok(());
                }

                Err(io::Error::other("injected draft sync failure"))
            },
        )
        .expect("publication should succeed despite the draft warning");

        assert_eq!(published, fixture.final_path());
        assert_eq!(
            String::from_utf8(standard_error).expect("warning should be UTF-8"),
            format!(
                "warning: {}: published, but the draft was not removed: \
                 injected draft sync failure\n",
                fixture.resolved_case().join(GENERATED_DRAFT).display()
            )
        );
        assert_eq!(
            String::from_utf8(standard_output).expect("output should be UTF-8"),
            format!("{}\n", fixture.final_path().display())
        );
    }

    #[test]
    fn draft_hint_shell_quoting_matches_python_shlex() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("/tmp/safe-case"), "/tmp/safe-case");
        assert_eq!(
            shell_quote("/tmp/case with'quote"),
            "'/tmp/case with'\"'\"'quote'"
        );
    }

    fn assert_no_temporary_artifact(case: &Path) {
        assert!(
            fs::read_dir(case)
                .expect("case should be readable")
                .all(|entry| !entry
                    .expect("directory entry should be readable")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".artifact-"))
        );
    }

    fn manifest() -> &'static str {
        r#"schema_version = 2
id = "test-case"
phase = "planning"
status = "drafting"
next_agent = "codex"
"#
    }

    fn artifact() -> &'static str {
        r#"+++
artifact_schema_version = 1
artifact_id = "a1b2c3"
sequence = 1
kind = "plan"
topic = "test-plan"
author = "codex"
created_at = 2026-07-27T19:00:00+09:00
responds_to = []
supersedes = []
source_branch = ""
source_commit = ""
source_path = ""
subject_repository = ""
subject_path = ""
subject_commit = ""
+++
# Test artifact
"#
    }
}
