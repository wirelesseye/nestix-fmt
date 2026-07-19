use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_nestix-fmt")
}

fn temp_dir(test: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("nestix-fmt-{test}-{nonce}"));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn stdin_formats_to_stdout() {
    let mut child = Command::new(binary())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"fn view(){layout! {Root{Child}}}")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "fn view() {\n    layout! {\n        Root {\n            Child\n        }\n    }\n}\n"
    );
}

#[test]
fn no_rustfmt_preserves_outer_rust_and_disables_embedded_rustfmt() {
    let mut child = Command::new(binary())
        .arg("--no-rustfmt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"fn view( ){layout! {Root{Widget(.value=long_call(first,second,third))}}}")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let formatted = String::from_utf8(output.stdout).unwrap();
    assert!(formatted.starts_with("fn view( ){layout! {"));
    assert!(formatted.contains("Widget(.value = long_call(first,"));
}

#[test]
fn check_reports_then_in_place_formatting_fixes_a_file() {
    let directory = temp_dir("check");
    let file = directory.join("view.rs");
    let source = "layout! {Root{Child}}";
    fs::write(&file, source).unwrap();

    let check = Command::new(binary())
        .args(["--check", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(check.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&file).unwrap(), source);

    assert!(
        Command::new(binary())
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary())
            .args(["--check", file.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn directory_walk_skips_ignored_hidden_and_target_files() {
    let directory = temp_dir("walk");
    fs::create_dir(directory.join(".git")).unwrap();
    fs::create_dir(directory.join(".hidden")).unwrap();
    fs::create_dir(directory.join("target")).unwrap();
    fs::write(directory.join(".gitignore"), "ignored.rs\n").unwrap();
    for relative in ["kept.rs", "ignored.rs", ".hidden/view.rs", "target/view.rs"] {
        fs::write(directory.join(relative), "layout! {Root}").unwrap();
    }

    assert!(
        Command::new(binary())
            .arg(&directory)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        fs::read_to_string(directory.join("kept.rs"))
            .unwrap()
            .contains('\n')
    );
    for relative in ["ignored.rs", ".hidden/view.rs", "target/view.rs"] {
        assert_eq!(
            fs::read_to_string(directory.join(relative)).unwrap(),
            "layout! {Root}"
        );
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn parse_error_prevents_all_batch_writes() {
    let directory = temp_dir("atomic");
    let valid = directory.join("a.rs");
    let invalid = directory.join("b.rs");
    fs::write(&valid, "layout! {Root}").unwrap();
    fs::write(&invalid, "layout! {if broken}").unwrap();

    let output = Command::new(binary()).arg(&directory).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read_to_string(valid).unwrap(), "layout! {Root}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("b.rs:1:")
    );
    fs::remove_dir_all(directory).unwrap();
}

fn make_package(root: &std::path::Path, name: &str) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "layout! {Root}").unwrap();
}

#[test]
fn package_formats_only_the_selected_workspace_member() {
    let directory = temp_dir("package");
    make_package(&directory.join("one"), "one");
    make_package(&directory.join("two"), "two");
    fs::write(
        directory.join("Cargo.toml"),
        "[workspace]\nmembers = [\"one\", \"two\"]\nresolver = \"3\"\n",
    )
    .unwrap();

    let output = Command::new(binary())
        .current_dir(&directory)
        .args(["--no-rustfmt", "--package", "one"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(directory.join("one/src/lib.rs"))
            .unwrap()
            .contains('\n')
    );
    assert_eq!(
        fs::read_to_string(directory.join("two/src/lib.rs")).unwrap(),
        "layout! {Root}"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn all_includes_local_path_dependencies() {
    let directory = temp_dir("all");
    make_package(&directory.join("app"), "app");
    make_package(&directory.join("dependency"), "dependency");
    fs::write(
        directory.join("app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\ndependency = { path = \"../dependency\" }\n",
    )
    .unwrap();

    let output = Command::new(binary())
        .current_dir(directory.join("app"))
        .args(["--no-rustfmt", "--all"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(directory.join("app/src/lib.rs"))
            .unwrap()
            .contains('\n')
    );
    assert!(
        fs::read_to_string(directory.join("dependency/src/lib.rs"))
            .unwrap()
            .contains('\n')
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn manifest_path_selects_a_manifest_outside_the_current_directory() {
    let directory = temp_dir("manifest");
    make_package(&directory.join("app"), "app");
    let manifest = directory.join("app/Cargo.toml");

    let output = Command::new(binary())
        .current_dir(std::env::temp_dir())
        .arg("--no-rustfmt")
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(directory.join("app/src/lib.rs"))
            .unwrap()
            .contains('\n')
    );
    fs::remove_dir_all(directory).unwrap();
}
