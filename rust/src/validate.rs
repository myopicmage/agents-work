//! Read-only case validation and report rendering.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use toml::value::{Datetime, Offset, Time};
use toml::{Table, Value};

use crate::artifact::{ArtifactMetadata, parse_front_matter};
use crate::case::{
    discovered_artifacts, normalize_newlines, path_error, read_manifest, resolve_path,
};
use crate::manifest::ManifestSchema;

/// A complete read-only validation result for one case directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationReport {
    case: PathBuf,
    inventory: Vec<InventoryEntry>,
    infos: Vec<String>,
    errors: Vec<String>,
}

impl ValidationReport {
    /// Returns whether the case has no validation errors.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns the resolved case directory.
    #[must_use]
    pub fn case(&self) -> &Path {
        &self.case
    }

    /// Returns the number shown in an invalid result line.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    /// Renders everything the Python command writes to standard output.
    #[must_use]
    pub fn stdout_text(&self) -> String {
        let mut output = self.stdout_prelude();
        output.push_str(&self.result_line());
        output
    }

    /// Renders everything the Python command writes to standard error.
    #[must_use]
    pub fn stderr_text(&self) -> String {
        let mut output = String::new();

        for error in &self.errors {
            output.push_str("error: ");
            output.push_str(error);
            output.push('\n');
        }

        output
    }

    fn stdout_prelude(&self) -> String {
        let mut output = format!("case: {}\ninventory:\n", self.case.display());

        if self.inventory.is_empty() {
            output.push_str("  (none)\n");
        }

        for artifact in &self.inventory {
            output.push_str("  ");
            output.push_str(&artifact.name);
            output.push_str("  ");

            match &artifact.identity {
                None => output.push_str("INVALID"),
                Some(identity) => {
                    output.push_str(&identity.kind);
                    output.push_str("  ");
                    output.push_str(&identity.author);
                }
            }

            output.push_str("  ");
            output.push_str(&artifact.digest);
            output.push('\n');
        }

        for info in &self.infos {
            output.push_str("info: ");
            output.push_str(info);
            output.push('\n');
        }

        output
    }

    fn result_line(&self) -> String {
        if self.is_valid() {
            format!("result: valid ({} artifacts)\n", self.inventory.len())
        } else {
            format!("result: invalid ({} errors)\n", self.errors.len())
        }
    }

    fn write_to(
        &self,
        standard_output: &mut impl Write,
        standard_error: &mut impl Write,
    ) -> io::Result<()> {
        standard_output.write_all(self.stdout_prelude().as_bytes())?;
        standard_output.flush()?;
        standard_error.write_all(self.stderr_text().as_bytes())?;
        standard_error.flush()?;
        standard_output.write_all(self.result_line().as_bytes())?;
        standard_output.flush()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InventoryEntry {
    name: String,
    identity: Option<InventoryIdentity>,
    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InventoryIdentity {
    kind: String,
    author: String,
}

/// Validates every requested case, continuing after ordinary validation errors.
///
/// # Errors
///
/// Returns an I/O error when traversal itself cannot continue or report output
/// cannot be written. A malformed case is an `Ok(false)` result.
pub fn validate_cases(
    cases: &[PathBuf],
    standard_output: &mut impl Write,
    standard_error: &mut impl Write,
) -> io::Result<bool> {
    let mut all_valid = true;

    for case in cases {
        let report = validate_case(case)?;
        all_valid &= report.is_valid();
        report.write_to(standard_output, standard_error)?;
    }

    Ok(all_valid)
}

/// Validates one case without changing it.
///
/// # Errors
///
/// Returns an I/O error if the case cannot be resolved, enumerated, or an
/// artifact cannot be read. Manifest and sidecar read failures are validation
/// diagnostics, matching the Python implementation.
pub fn validate_case(case: &Path) -> io::Result<ValidationReport> {
    let case = resolve_path(case)?;
    let mut errors = Vec::new();
    let mut infos = Vec::new();
    let mut inventory = Vec::new();

    let (manifest, manifest_errors) = read_manifest(&case);
    errors.extend(manifest_errors);

    let discovered = discovered_artifacts(&case)?;
    let discovered_names = discovered
        .iter()
        .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();

    for sidecar in discovered_sidecars(&case)? {
        let sidecar_name = file_name(&sidecar);
        let artifact_name = sidecar_name
            .strip_suffix(".sha256")
            .unwrap_or(&sidecar_name);

        if !case.join(artifact_name).is_file() {
            errors.push(format!("{sidecar_name}: orphan sidecar"));
        }
    }

    let mut metadata_by_name = Vec::new();
    let mut hash_by_name = BTreeMap::new();

    for artifact in discovered {
        let name = file_name(&artifact);
        let data = read_required(&artifact)?;
        let digest = sha256_bytes(&data);
        hash_by_name.insert(name.clone(), digest.clone());

        let identity = match parse_front_matter(&data, &artifact) {
            Err(error) => {
                errors.push(error.to_string());
                None
            }
            Ok(table) => {
                let identity = InventoryIdentity {
                    kind: inventory_value(table.get("kind")),
                    author: inventory_value(table.get("author")),
                };

                match ArtifactMetadata::parse(&table, &artifact) {
                    Ok(metadata) => {
                        if let Err(filename_errors) = metadata.validate_filename(&artifact) {
                            errors.extend(filename_errors.into_vec());
                        }
                    }
                    Err(field_errors) => errors.extend(field_errors.into_vec()),
                }

                metadata_by_name.push((name.clone(), table));
                Some(identity)
            }
        };

        let sidecar = artifact.with_file_name(format!("{name}.sha256"));
        if sidecar.is_file() {
            let expected = format!("{digest}  {name}\n");

            match read_ascii_text(&sidecar) {
                Err(error) => errors.push(format!(
                    "{}: unreadable sidecar: {error}",
                    file_name(&sidecar)
                )),
                Ok(actual) if actual != expected => {
                    errors.push(format!("{}: hash or format mismatch", file_name(&sidecar)));
                }
                Ok(_) => {}
            }
        } else {
            errors.push(format!("{name}: missing sidecar"));
        }

        inventory.push(InventoryEntry {
            name,
            identity,
            digest,
        });
    }

    validate_relationships(&metadata_by_name, &discovered_names, &mut errors);

    if let Some(manifest) = manifest.as_ref()
        && manifest.schema() == Some(ManifestSchema::V1)
    {
        validate_legacy_inventory(
            &case,
            manifest.table(),
            &discovered_names,
            &hash_by_name,
            &mut infos,
            &mut errors,
        );
    }

    Ok(ValidationReport {
        case,
        inventory,
        infos,
        errors,
    })
}

/// Returns a lowercase SHA-256 digest for exact artifact bytes.
#[must_use]
pub fn sha256_bytes(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut encoded = String::with_capacity(64);

    for byte in digest {
        encoded.push(lower_hex_digit(byte >> 4));
        encoded.push(lower_hex_digit(byte & 0x0f));
    }

    encoded
}

fn lower_hex_digit(value: u8) -> char {
    char::from(if value < 10 {
        b'0' + value
    } else {
        b'a' + value - 10
    })
}

fn discovered_sidecars(case: &Path) -> io::Result<Vec<PathBuf>> {
    let mut sidecars = Vec::new();

    for entry in fs::read_dir(case).map_err(|error| path_error(case, &error))? {
        let path = entry.map_err(|error| path_error(case, &error))?.path();

        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".md.sha256"))
        {
            sidecars.push(path);
        }
    }

    sidecars.sort();
    Ok(sidecars)
}

fn validate_relationships(
    metadata_by_name: &[(String, Table)],
    discovered_names: &BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    for (name, table) in metadata_by_name {
        for field in ["responds_to", "supersedes"] {
            match table.get(field) {
                Some(Value::Array(targets)) => {
                    for target in targets.iter().filter_map(Value::as_str) {
                        push_missing_relationship(name, field, target, discovered_names, errors);
                    }
                }
                Some(Value::String(targets)) => {
                    for target in targets.chars() {
                        push_missing_relationship(
                            name,
                            field,
                            &target.to_string(),
                            discovered_names,
                            errors,
                        );
                    }
                }
                Some(Value::Table(targets)) => {
                    for target in targets.keys() {
                        push_missing_relationship(name, field, target, discovered_names, errors);
                    }
                }
                _ => {}
            }
        }
    }
}

fn push_missing_relationship(
    name: &str,
    field: &str,
    target: &str,
    discovered_names: &BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    if !discovered_names.contains(target) {
        errors.push(format!("{name}: {field} target missing: {target}"));
    }
}

fn validate_legacy_inventory(
    case: &Path,
    table: &Table,
    discovered_names: &BTreeSet<String>,
    hash_by_name: &BTreeMap<String, String>,
    infos: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    let label = case.join("work.toml");
    let records = match table.get("artifacts") {
        None => &[][..],
        Some(Value::Array(records)) => records.as_slice(),
        Some(_) => {
            errors.push(format!("{}: artifacts must be an array", label.display()));
            &[]
        }
    };
    let mut listed = BTreeSet::new();

    for record in records {
        let Some(record) = record.as_table() else {
            errors.push(format!("{}: malformed artifact record", label.display()));
            continue;
        };
        let Some(path) = record.get("path").and_then(Value::as_str) else {
            errors.push(format!("{}: invalid legacy artifact path", label.display()));
            continue;
        };

        if !equals_basename(path) {
            errors.push(format!("{}: invalid legacy artifact path", label.display()));
            continue;
        }

        listed.insert(path.to_owned());

        if !discovered_names.contains(path) {
            errors.push(format!(
                "{}: listed artifact missing: {path}",
                label.display()
            ));
        } else if record.get("sha256").and_then(Value::as_str)
            != hash_by_name.get(path).map(String::as_str)
        {
            errors.push(format!("{}: legacy hash mismatch: {path}", label.display()));
        }
    }

    for name in discovered_names.difference(&listed) {
        infos.push(format!(
            "discovered artifact absent from legacy manifest: {name}"
        ));
    }
}

fn read_required(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path).map_err(|error| path_error(path, &error))
}

fn read_ascii_text(path: &Path) -> Result<String, String> {
    let data = fs::read(path).map_err(|error| error.to_string())?;

    if let Some((position, byte)) = data
        .iter()
        .copied()
        .enumerate()
        .find(|(_, byte)| !byte.is_ascii())
    {
        return Err(format!(
            "'ascii' codec can't decode byte 0x{byte:02x} in position {position}: ordinal not in range(128)"
        ));
    }

    String::from_utf8(data)
        .map(normalize_newlines)
        .map_err(|error| error.to_string())
}

fn inventory_value(value: Option<&Value>) -> String {
    match value {
        None => "?".to_owned(),
        Some(value) => python_value_string(value),
    }
}

fn python_value_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Integer(value) => value.to_string(),
        Value::Float(value) => python_float_string(*value),
        Value::Boolean(true) => "True".to_owned(),
        Value::Boolean(false) => "False".to_owned(),
        Value::Datetime(value) => python_datetime_string(value),
        Value::Array(values) => python_array_string(values),
        Value::Table(values) => python_table_string(values),
    }
}

fn python_value_repr(value: &Value) -> String {
    match value {
        Value::String(value) => python_string_repr(value),
        Value::Datetime(value) => python_datetime_repr(value),
        _ => python_value_string(value),
    }
}

fn python_float_string(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }

    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }

    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".to_owned()
        } else {
            "0.0".to_owned()
        };
    }

    let sign = if value.is_sign_negative() { "-" } else { "" };
    let rendered = format!("{:?}", value.abs());
    let (mantissa, explicit_exponent) =
        rendered
            .split_once(['e', 'E'])
            .map_or((rendered.as_str(), 0), |(mantissa, exponent)| {
                (
                    mantissa,
                    exponent
                        .parse::<i32>()
                        .expect("Rust float debug exponent should be an integer"),
                )
            });
    let decimal_point = mantissa.find('.').unwrap_or(mantissa.len());
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .collect::<Vec<_>>();
    let first = digits
        .iter()
        .position(|digit| *digit != b'0')
        .expect("nonzero Rust float debug text should contain a nonzero digit");
    let last = digits
        .iter()
        .rposition(|digit| *digit != b'0')
        .expect("nonzero Rust float debug text should contain a nonzero digit");
    let significant_digits = std::str::from_utf8(&digits[first..=last])
        .expect("Rust float debug digits should be ASCII");
    let scientific_exponent = explicit_exponent
        + i32::try_from(decimal_point).expect("float mantissa length should fit i32")
        - i32::try_from(first).expect("float digit position should fit i32")
        - 1;

    if (-4..16).contains(&scientific_exponent) {
        format!(
            "{sign}{}",
            fixed_float_string(significant_digits, scientific_exponent)
        )
    } else {
        format!(
            "{sign}{}",
            scientific_float_string(significant_digits, scientific_exponent)
        )
    }
}

fn fixed_float_string(digits: &str, scientific_exponent: i32) -> String {
    if scientific_exponent < 0 {
        let zeroes = usize::try_from(-scientific_exponent - 1)
            .expect("negative float exponent magnitude should fit usize");
        return format!("0.{}{}", "0".repeat(zeroes), digits);
    }

    let integer_digits = usize::try_from(scientific_exponent + 1)
        .expect("nonnegative float exponent should fit usize");

    if integer_digits >= digits.len() {
        format!("{}{}.0", digits, "0".repeat(integer_digits - digits.len()))
    } else {
        format!(
            "{}.{}",
            &digits[..integer_digits],
            &digits[integer_digits..]
        )
    }
}

fn scientific_float_string(digits: &str, scientific_exponent: i32) -> String {
    let mut rendered = digits[..1].to_owned();

    if digits.len() > 1 {
        rendered.push('.');
        rendered.push_str(&digits[1..]);
    }

    let exponent_sign = if scientific_exponent < 0 { '-' } else { '+' };
    write!(
        rendered,
        "e{exponent_sign}{:02}",
        scientific_exponent.unsigned_abs()
    )
    .expect("writing to a String should succeed");
    rendered
}

fn python_datetime_string(value: &Datetime) -> String {
    match (value.date, value.time) {
        (Some(date), Some(time)) => {
            let mut rendered = format!("{date} {}", python_time_string(&time));

            if let Some(offset) = value.offset {
                rendered.push_str(&python_offset_string(offset));
            }

            rendered
        }
        (Some(date), None) => date.to_string(),
        (None, Some(time)) => python_time_string(&time),
        (None, None) => String::new(),
    }
}

fn python_time_string(value: &Time) -> String {
    let mut rendered = format!(
        "{:02}:{:02}:{:02}",
        value.hour,
        value.minute,
        value.second.unwrap_or(0)
    );
    let microsecond = value.nanosecond.unwrap_or(0) / 1_000;

    if microsecond != 0 {
        write!(rendered, ".{microsecond:06}").expect("writing to a String should succeed");
    }

    rendered
}

fn python_offset_string(value: Offset) -> String {
    let minutes = match value {
        Offset::Z => 0,
        Offset::Custom { minutes } => minutes,
    };
    let sign = if minutes < 0 { '-' } else { '+' };
    let magnitude = minutes.unsigned_abs();
    format!("{sign}{:02}:{:02}", magnitude / 60, magnitude % 60)
}

fn python_datetime_repr(value: &Datetime) -> String {
    match (value.date, value.time) {
        (Some(date), Some(time)) => {
            let mut arguments = format!(
                "{}, {}, {}, {}, {}",
                date.year, date.month, date.day, time.hour, time.minute
            );
            push_python_time_arguments(&mut arguments, &time);

            if let Some(offset) = value.offset {
                arguments.push_str(", tzinfo=");
                arguments.push_str(&python_timezone_repr(offset));
            }

            format!("datetime.datetime({arguments})")
        }
        (Some(date), None) => {
            format!("datetime.date({}, {}, {})", date.year, date.month, date.day)
        }
        (None, Some(time)) => {
            let mut arguments = format!("{}, {}", time.hour, time.minute);
            push_python_time_arguments(&mut arguments, &time);
            format!("datetime.time({arguments})")
        }
        (None, None) => "datetime.datetime()".to_owned(),
    }
}

fn push_python_time_arguments(arguments: &mut String, value: &Time) {
    let second = value.second.unwrap_or(0);
    let microsecond = value.nanosecond.unwrap_or(0) / 1_000;

    if second != 0 || microsecond != 0 {
        write!(arguments, ", {second}").expect("writing to a String should succeed");
    }

    if microsecond != 0 {
        write!(arguments, ", {microsecond}").expect("writing to a String should succeed");
    }
}

fn python_timezone_repr(value: Offset) -> String {
    let minutes = match value {
        Offset::Z => 0,
        Offset::Custom { minutes } => i32::from(minutes),
    };

    if minutes == 0 {
        return "datetime.timezone.utc".to_owned();
    }

    let total_seconds = minutes * 60;
    let timedelta = if total_seconds > 0 {
        format!("datetime.timedelta(seconds={total_seconds})")
    } else {
        let days = total_seconds.div_euclid(86_400);
        let seconds = total_seconds.rem_euclid(86_400);
        format!("datetime.timedelta(days={days}, seconds={seconds})")
    };
    format!("datetime.timezone({timedelta})")
}

fn python_array_string(values: &[Value]) -> String {
    let values = values
        .iter()
        .map(python_value_repr)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

fn python_table_string(values: &Table) -> String {
    let values = values
        .iter()
        .map(|(key, value)| format!("{}: {}", python_string_repr(key), python_value_repr(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{values}}}")
}

fn python_string_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(value.len() + 2);
    rendered.push(quote);

    for character in value.chars() {
        match character {
            character if character == quote => {
                rendered.push('\\');
                rendered.push(character);
            }
            '\\' => rendered.push_str("\\\\"),
            '\t' => rendered.push_str("\\t"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\'' | '"' => rendered.push(character),
            character if is_debug_printable(character) => rendered.push(character),
            character => push_python_character_escape(&mut rendered, character),
        }
    }

    rendered.push(quote);
    rendered
}

fn is_debug_printable(value: char) -> bool {
    let mut escaped = value.escape_debug();
    escaped.next() == Some(value) && escaped.next().is_none()
}

fn push_python_character_escape(rendered: &mut String, value: char) {
    let codepoint = u32::from(value);

    if codepoint <= 0xff {
        write!(rendered, "\\x{codepoint:02x}").expect("writing to a String should succeed");
    } else if codepoint <= 0xffff {
        write!(rendered, "\\u{codepoint:04x}").expect("writing to a String should succeed");
    } else {
        write!(rendered, "\\U{codepoint:08x}").expect("writing to a String should succeed");
    }
}

fn equals_basename(value: &str) -> bool {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .map_or(value.is_empty(), |name| name == value)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}
