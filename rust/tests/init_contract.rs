use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use agents_work::init::{InitRequest, init};
use agents_work::validate::validate_case;
use toml::Table;

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    repository: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "agents-work-init-{}-{label}-{unique}",
            std::process::id()
        ));
        let repository = root.join("repository");
        fs::create_dir_all(&repository).expect("repository fixture should be created");
        Self { root, repository }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn request<'a>(case: &'a Path, repository: &'a Path, title: &'a str) -> InitRequest<'a> {
    InitRequest {
        case,
        repository,
        title,
    }
}

#[test]
fn init_creates_an_inert_valid_schema_two_case() {
    let fixture = Fixture::new("success");
    let case = fixture.root.join("new-case");
    let mut standard_output = Vec::new();

    let manifest_path = init(
        request(&case, &fixture.repository, "New case"),
        &mut standard_output,
    )
    .expect("case initialization should succeed");
    let manifest = fs::read_to_string(&manifest_path)
        .expect("manifest should be readable")
        .parse::<Table>()
        .expect("manifest should be TOML");

    assert_eq!(manifest["schema_version"].as_integer(), Some(2));
    assert_eq!(manifest["id"].as_str(), Some("new-case"));
    assert_eq!(manifest["repository_name"].as_str(), Some("repository"));
    assert_eq!(manifest["phase"].as_str(), Some("planning"));
    assert_eq!(manifest["status"].as_str(), Some("deferred"));
    assert_eq!(manifest["next_agent"].as_str(), Some(""));
    assert_eq!(manifest["requested_action"].as_str(), Some(""));
    assert_eq!(manifest["created_at"], manifest["updated_at"]);
    assert_eq!(
        String::from_utf8(standard_output).expect("stdout should be UTF-8"),
        format!("{}\n", manifest_path.display())
    );
    assert!(
        validate_case(&case)
            .expect("created case should validate")
            .is_valid()
    );
}

#[test]
fn init_refuses_to_replace_an_existing_manifest() {
    let fixture = Fixture::new("existing");
    let case = fixture.root.join("existing-case");

    init(
        request(&case, &fixture.repository, "Original"),
        &mut Vec::new(),
    )
    .expect("first initialization should succeed");
    let before = fs::read(case.join("work.toml")).expect("manifest should be readable");
    let error = init(
        request(&case, &fixture.repository, "Replacement"),
        &mut Vec::new(),
    )
    .expect_err("second initialization should fail");

    assert!(error.to_string().ends_with("work.toml: already exists"));
    assert_eq!(
        fs::read(case.join("work.toml")).expect("manifest should remain readable"),
        before
    );
}

#[test]
fn init_requires_a_slug_case_name() {
    let fixture = Fixture::new("slug");
    let case = fixture.root.join("Not A Slug");
    let error = init(request(&case, &fixture.repository, "Case"), &mut Vec::new())
        .expect_err("invalid case ID should fail");

    assert!(
        error
            .to_string()
            .ends_with("case name must be a lowercase slug")
    );
    assert!(!case.exists());
}

#[test]
#[ignore = "requires AGENTS_WORK_PYTHON_REFERENCE"]
fn init_cli_matches_the_python_reference() {
    let fixture = Fixture::new("oracle");
    let python_case = fixture.root.join("python").join("new-case");
    let rust_case = fixture.root.join("rust").join("new-case");
    let reference = std::env::var_os("AGENTS_WORK_PYTHON_REFERENCE")
        .expect("AGENTS_WORK_PYTHON_REFERENCE must name agents_work.py");
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| OsString::from("python3"));

    let python_output = Command::new(python)
        .arg(reference)
        .arg("init")
        .arg(&python_case)
        .arg("--repository")
        .arg(&fixture.repository)
        .args(["--title", "New case"])
        .output()
        .expect("Python init should run");
    let rust_output = Command::new(env!("CARGO_BIN_EXE_agents-work"))
        .arg("init")
        .arg(&rust_case)
        .arg("--repository")
        .arg(&fixture.repository)
        .args(["--title", "New case"])
        .output()
        .expect("Rust init should run");

    assert_eq!(rust_output.status.code(), python_output.status.code());
    assert_eq!(rust_output.status.code(), Some(0));
    assert_eq!(rust_output.stderr, python_output.stderr);

    let mut python_manifest = fs::read_to_string(python_case.join("work.toml"))
        .expect("Python manifest should be readable")
        .parse::<Table>()
        .expect("Python manifest should be TOML");
    let mut rust_manifest = fs::read_to_string(rust_case.join("work.toml"))
        .expect("Rust manifest should be readable")
        .parse::<Table>()
        .expect("Rust manifest should be TOML");

    for manifest in [&mut python_manifest, &mut rust_manifest] {
        manifest.insert(
            "created_at".to_owned(),
            toml::Value::String("TIMESTAMP".to_owned()),
        );
        manifest.insert(
            "updated_at".to_owned(),
            toml::Value::String("TIMESTAMP".to_owned()),
        );
    }

    assert_eq!(rust_manifest, python_manifest);
}
