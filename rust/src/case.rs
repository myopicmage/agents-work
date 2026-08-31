//! Shared filesystem boundary for case discovery and manifest reads.

use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use toml::Table;

use crate::artifact::ArtifactName;
use crate::manifest::ManifestDocument;

pub(crate) fn resolve_path(path: &Path) -> io::Result<PathBuf> {
    let expanded = expand_current_user(path);
    let absolute = std::path::absolute(expanded).map_err(|error| path_error(path, &error))?;
    canonicalize_allow_missing(&absolute)
}

pub(crate) fn read_manifest(case: &Path) -> (Option<ManifestDocument>, Vec<String>) {
    let path = case.join("work.toml");

    if !path.is_file() {
        return (None, vec![format!("{}: missing work.toml", path.display())]);
    }

    let data = match fs::read(&path) {
        Ok(data) => data,
        Err(error) => {
            return (
                None,
                vec![format!("{}: invalid work.toml: {error}", path.display())],
            );
        }
    };
    let text = match String::from_utf8(data) {
        Ok(text) => normalize_newlines(text),
        Err(error) => {
            return (
                None,
                vec![format!("{}: invalid work.toml: {error}", path.display())],
            );
        }
    };
    let table = match text.parse::<Table>() {
        Ok(table) => table,
        Err(error) => {
            return (
                None,
                vec![format!("{}: invalid work.toml: {error}", path.display())],
            );
        }
    };

    let validation = ManifestDocument::validate(table, &path.display().to_string());
    let (manifest, errors) = validation.into_parts();
    (Some(manifest), errors)
}

pub(crate) fn discovered_artifacts(case: &Path) -> io::Result<Vec<PathBuf>> {
    let mut artifacts = Vec::new();

    for entry in fs::read_dir(case).map_err(|error| path_error(case, &error))? {
        let path = entry.map_err(|error| path_error(case, &error))?.path();

        if path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.parse::<ArtifactName>().is_ok())
        {
            artifacts.push(path);
        }
    }

    artifacts.sort();
    Ok(artifacts)
}

pub(crate) fn normalize_newlines(text: String) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text
    }
}

pub(crate) fn path_error(path: &Path, error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

pub(crate) fn create_new_file(path: &Path, mode: u32) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    options.mode(mode);

    #[cfg(not(unix))]
    let _ = mode;

    options.open(path).map_err(|error| path_error(path, &error))
}

#[cfg(unix)]
pub(crate) fn set_file_mode(file: &File, path: &Path, mode: u32) -> io::Result<()> {
    let mut permissions = file
        .metadata()
        .map_err(|error| path_error(path, &error))?
        .permissions();
    permissions.set_mode(mode);
    file.set_permissions(permissions)
        .map_err(|error| path_error(path, &error))
}

#[cfg(not(unix))]
pub(crate) fn set_file_mode(_file: &File, _path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    let directory = File::open(path)?;
    directory.sync_all()
}

pub(crate) fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(path_error(path, &error)),
    }
}

fn expand_current_user(path: &Path) -> PathBuf {
    let Ok(remainder) = path.strip_prefix("~") else {
        return path.to_owned();
    };
    let Some(home) = env::var_os("HOME") else {
        return path.to_owned();
    };

    PathBuf::from(home).join(remainder)
}

fn canonicalize_allow_missing(path: &Path) -> io::Result<PathBuf> {
    let mut probe = path;
    let mut missing = Vec::<OsString>::new();

    loop {
        match fs::canonicalize(probe) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }

                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(name) = probe.file_name() else {
                    return Ok(path.to_owned());
                };
                let Some(parent) = probe.parent() else {
                    return Ok(path.to_owned());
                };

                missing.push(name.to_owned());
                probe = parent;
            }
            Err(error) => return Err(path_error(probe, &error)),
        }
    }
}
