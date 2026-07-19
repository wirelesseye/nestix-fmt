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
fn rustfmt_config_controls_rust_width_and_dsl_indentation() {
    let directory = temp_dir("config");
    fs::write(
        directory.join("rustfmt.toml"),
        "max_width = 50\ntab_spaces = 2\nuse_field_init_shorthand = true\n",
    )
    .unwrap();
    let file = directory.join("view.rs");
    fs::write(
        &file,
        "fn view(first: u8) { let _ = Example { first: first }; layout! {Root{Widget(.value=build_value(first_argument,second_argument,third_argument))}} }",
    )
    .unwrap();

    let output = Command::new(binary()).arg(&file).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = fs::read_to_string(&file).unwrap();
    assert!(formatted.contains("Example { first }"));
    assert!(formatted.contains("\n    Root {\n      Widget("));
    assert!(formatted.contains("build_value(\n"));
    assert!(formatted.lines().all(|line| line.len() <= 50));

    assert!(
        Command::new(binary())
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), formatted);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn hard_tabs_are_used_for_dsl_indentation() {
    let directory = temp_dir("hard-tabs");
    fs::write(
        directory.join("rustfmt.toml"),
        "hard_tabs = true\ntab_spaces = 4\n",
    )
    .unwrap();
    let file = directory.join("view.rs");
    fs::write(&file, "layout! {Root{Child}}").unwrap();

    assert!(
        Command::new(binary())
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    let formatted = fs::read_to_string(&file).unwrap();
    assert_eq!(formatted, "layout! {\n\tRoot {\n\t\tChild\n\t}\n}\n");
    assert!(
        Command::new(binary())
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), formatted);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn nested_files_use_the_nearest_rustfmt_config() {
    let directory = temp_dir("nested-config");
    let nested = directory.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(directory.join("rustfmt.toml"), "tab_spaces = 2\n").unwrap();
    fs::write(nested.join("rustfmt.toml"), "tab_spaces = 6\n").unwrap();
    let outer = directory.join("outer.rs");
    let inner = nested.join("inner.rs");
    fs::write(&outer, "layout! {Root{Child}}").unwrap();
    fs::write(&inner, "layout! {Root{Child}}").unwrap();

    assert!(
        Command::new(binary())
            .arg(&directory)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        fs::read_to_string(outer)
            .unwrap()
            .contains("\n  Root {\n    Child")
    );
    assert!(
        fs::read_to_string(inner)
            .unwrap()
            .contains("\n      Root {\n            Child")
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stdin_reads_rustfmt_config_from_the_current_directory() {
    let directory = temp_dir("stdin-config");
    fs::write(directory.join("rustfmt.toml"), "tab_spaces = 2\n").unwrap();
    let mut child = Command::new(binary())
        .current_dir(&directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"layout! {Root{Child}}")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "layout! {\n  Root {\n    Child\n  }\n}\n"
    );
    fs::remove_dir_all(directory).unwrap();
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
fn formatted_crlf_file_is_unchanged_and_passes_check() {
    let directory = temp_dir("crlf-check");
    let file = directory.join("view.rs");
    let source = "layout! {\r\n    Root {\r\n        Child\r\n    }\r\n}\r\n";
    fs::write(&file, source).unwrap();

    assert!(
        Command::new(binary())
            .args(["--check", file.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(binary())
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read(&file).unwrap(), source.as_bytes());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn formatting_preserves_crlf_line_endings() {
    let directory = temp_dir("crlf-format");
    let file = directory.join("view.rs");
    fs::write(&file, "fn view(){\r\nlayout! {Root{Child}}\r\n}\r\n").unwrap();

    assert!(
        Command::new(binary())
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    let formatted = String::from_utf8(fs::read(&file).unwrap()).unwrap();
    assert!(formatted.contains("\r\n"));
    assert!(!formatted.replace("\r\n", "").contains('\n'));
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
fn no_rustfmt_preserves_crlf_line_endings() {
    let directory = temp_dir("crlf-no-rustfmt");
    let file = directory.join("view.rs");
    fs::write(&file, "fn view( ){\r\nlayout! {Root{Child}}\r\n}\r\n").unwrap();

    assert!(
        Command::new(binary())
            .args(["--no-rustfmt", file.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let formatted = String::from_utf8(fs::read(&file).unwrap()).unwrap();
    assert!(formatted.starts_with("fn view( ){\r\n"));
    assert!(formatted.contains("\r\n"));
    assert!(!formatted.replace("\r\n", "").contains('\n'));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn explicit_newline_style_overrides_the_source_style() {
    let directory = temp_dir("newline-config");
    let unix = directory.join("unix");
    let windows = directory.join("windows");
    fs::create_dir(&unix).unwrap();
    fs::create_dir(&windows).unwrap();
    fs::write(unix.join("rustfmt.toml"), "newline_style = \"Unix\"\n").unwrap();
    fs::write(
        windows.join("rustfmt.toml"),
        "newline_style = \"Windows\"\n",
    )
    .unwrap();
    let unix_file = unix.join("view.rs");
    let windows_file = windows.join("view.rs");
    fs::write(&unix_file, "layout! {Root{Child}}\r\n").unwrap();
    fs::write(&windows_file, "layout! {Root{Child}}\n").unwrap();

    assert!(
        Command::new(binary())
            .args([unix_file.to_str().unwrap(), windows_file.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let unix_formatted = String::from_utf8(fs::read(unix_file).unwrap()).unwrap();
    let windows_formatted = String::from_utf8(fs::read(windows_file).unwrap()).unwrap();
    assert!(!unix_formatted.contains("\r\n"));
    assert!(windows_formatted.contains("\r\n"));
    assert!(!windows_formatted.replace("\r\n", "").contains('\n'));
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

#[test]
fn invalid_rustfmt_config_prevents_batch_writes_but_no_rustfmt_ignores_it() {
    let directory = temp_dir("invalid-config");
    fs::write(directory.join("rustfmt.toml"), "max_width = nope\n").unwrap();
    let first = directory.join("a.rs");
    let second = directory.join("b.rs");
    fs::write(&first, "layout! {Root}").unwrap();
    fs::write(&second, "layout! {Child}").unwrap();

    let output = Command::new(binary()).arg(&directory).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read_to_string(&first).unwrap(), "layout! {Root}");
    assert_eq!(fs::read_to_string(&second).unwrap(), "layout! {Child}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("failed to read rustfmt configuration")
    );

    assert!(
        Command::new(binary())
            .args(["--no-rustfmt", directory.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(fs::read_to_string(first).unwrap(), "layout! { Root }");
    assert_eq!(fs::read_to_string(second).unwrap(), "layout! { Child }");
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
    assert_eq!(
        fs::read_to_string(directory.join("one/src/lib.rs")).unwrap(),
        "layout! { Root }"
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
    assert_eq!(
        fs::read_to_string(directory.join("app/src/lib.rs")).unwrap(),
        "layout! { Root }"
    );
    assert_eq!(
        fs::read_to_string(directory.join("dependency/src/lib.rs")).unwrap(),
        "layout! { Root }"
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
    assert_eq!(
        fs::read_to_string(directory.join("app/src/lib.rs")).unwrap(),
        "layout! { Root }"
    );
    fs::remove_dir_all(directory).unwrap();
}
