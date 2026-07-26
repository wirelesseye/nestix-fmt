mod format;
mod syntax;

use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use clap::Parser;
use ignore::WalkBuilder;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Check formatting without changing files.
    #[arg(long)]
    check: bool,

    /// Disable rustfmt for both the complete source and Rust inside layout DSL.
    #[arg(long)]
    no_rustfmt: bool,

    /// Format the specified Cargo package.
    #[arg(short = 'p', long = "package", value_name = "SPEC", action = clap::ArgAction::Append)]
    packages: Vec<String>,

    /// Path to Cargo.toml.
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<PathBuf>,

    /// Format all packages and local path dependencies.
    #[arg(long, conflicts_with = "packages")]
    all: bool,

    /// Rust files or directories. Reads stdin when omitted.
    #[arg(value_name = "PATH", conflicts_with_all = ["packages", "manifest_path", "all"])]
    paths: Vec<PathBuf>,
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("nestix-fmt: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Args) -> Result<ExitCode, String> {
    if !args.no_rustfmt {
        format::ensure_rustfmt()?;
    }
    let cargo_mode = !args.packages.is_empty() || args.manifest_path.is_some() || args.all;
    if args.paths.is_empty() && !cargo_mode {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("failed to read stdin: {error}"))?;
        let formatted = format::format_source(&source, None, !args.no_rustfmt)?;
        if args.check {
            if source == formatted {
                return Ok(ExitCode::SUCCESS);
            }
            print_diff("<stdin>", &source, &formatted);
            return Ok(ExitCode::from(1));
        }
        print!("{formatted}");
        return Ok(ExitCode::SUCCESS);
    }

    let roots = if cargo_mode {
        cargo_package_roots(&args)?
    } else {
        args.paths.clone()
    };
    let files = discover(&roots)?;
    let mut pending = Vec::new();
    let mut had_errors = false;

    let mut sources = Vec::with_capacity(files.len());
    for path in files {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("{}: {error}", path.display());
                had_errors = true;
                continue;
            }
        };
        sources.push((path, source));
    }

    let formatted = format::format_files(&sources, !args.no_rustfmt);
    for ((path, source), result) in sources.into_iter().zip(formatted) {
        match result {
            Ok(formatted) if formatted != source => pending.push((path, source, formatted)),
            Ok(_) => {}
            Err(error) => {
                eprintln!("{error}");
                had_errors = true;
            }
        }
    }

    if had_errors {
        return Ok(ExitCode::from(2));
    }
    if args.check {
        for (path, source, formatted) in &pending {
            print_diff(&path.display().to_string(), source, formatted);
        }
        return Ok(if pending.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        });
    }
    for (path, _, formatted) in pending {
        fs::write(&path, formatted)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Clone, Copy)]
enum DiffLine<'a> {
    Same(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

fn print_diff(name: &str, original: &str, formatted: &str) {
    let original_has_newline = original.ends_with('\n');
    let formatted_has_newline = formatted.ends_with('\n');
    let original: Vec<_> = original.lines().collect();
    let formatted: Vec<_> = formatted.lines().collect();
    let mut lines = diff_lines(&original, &formatted);
    if !original_has_newline && formatted_has_newline {
        lines.push(DiffLine::Added(""));
    } else if original_has_newline && !formatted_has_newline {
        lines.push(DiffLine::Removed(""));
    }

    let changes: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (!matches!(line, DiffLine::Same(_))).then_some(index))
        .collect();
    let mut change = 0;
    while change < changes.len() {
        let start = changes[change].saturating_sub(3);
        let mut last_change = changes[change];
        change += 1;
        while change < changes.len() && changes[change] <= last_change + 6 {
            last_change = changes[change];
            change += 1;
        }
        let end = (last_change + 4).min(lines.len());
        let old_line = 1 + lines[..start]
            .iter()
            .filter(|line| !matches!(line, DiffLine::Added(_)))
            .count();
        println!("Diff in {name}:{old_line}:");
        for line in &lines[start..end] {
            match line {
                DiffLine::Same(line) => println!(" {line}"),
                DiffLine::Removed(line) => println!("-{line}"),
                DiffLine::Added(line) => println!("+{line}"),
            }
        }
        println!(" ");
    }
}

fn diff_lines<'a>(original: &[&'a str], formatted: &[&'a str]) -> Vec<DiffLine<'a>> {
    let mut lengths = vec![vec![0usize; formatted.len() + 1]; original.len() + 1];
    for old in (0..original.len()).rev() {
        for new in (0..formatted.len()).rev() {
            lengths[old][new] = if original[old] == formatted[new] {
                lengths[old + 1][new + 1] + 1
            } else {
                lengths[old + 1][new].max(lengths[old][new + 1])
            };
        }
    }

    let (mut old, mut new) = (0, 0);
    let mut lines = Vec::new();
    while old < original.len() || new < formatted.len() {
        if old < original.len() && new < formatted.len() && original[old] == formatted[new] {
            lines.push(DiffLine::Same(original[old]));
            old += 1;
            new += 1;
        } else if old < original.len()
            && (new == formatted.len() || lengths[old + 1][new] >= lengths[old][new + 1])
        {
            lines.push(DiffLine::Removed(original[old]));
            old += 1;
        } else {
            lines.push(DiffLine::Added(formatted[new]));
            new += 1;
        }
    }
    lines
}

fn cargo_package_roots(args: &Args) -> Result<Vec<PathBuf>, String> {
    let mut command = Command::new("cargo");
    command.args(["metadata", "--format-version", "1"]);
    if let Some(path) = &args.manifest_path {
        command.arg("--manifest-path").arg(path);
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to start cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid cargo metadata: {error}"))?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| "cargo metadata did not return packages".to_owned())?;
    let workspace_members: BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str())
        .collect();

    let selected: BTreeSet<&str> = if !args.packages.is_empty() {
        let mut ids = BTreeSet::new();
        for spec in &args.packages {
            let matches: Vec<_> = packages
                .iter()
                .filter(|package| {
                    package["id"]
                        .as_str()
                        .is_some_and(|id| workspace_members.contains(id))
                })
                .filter(|package| package["name"].as_str() == Some(spec.as_str()))
                .filter_map(|package| package["id"].as_str())
                .collect();
            match matches.as_slice() {
                [id] => {
                    ids.insert(*id);
                }
                [] => {
                    return Err(format!(
                        "package `{spec}` is not a member of this workspace"
                    ));
                }
                _ => return Err(format!("package specification `{spec}` is ambiguous")),
            }
        }
        ids
    } else if args.all {
        packages
            .iter()
            .filter(|package| package["source"].is_null())
            .filter_map(|package| package["id"].as_str())
            .collect()
    } else {
        default_package_ids(&metadata, packages, &workspace_members)?
    };

    packages
        .iter()
        .filter(|package| {
            package["id"]
                .as_str()
                .is_some_and(|id| selected.contains(id))
        })
        .map(|package| {
            package["manifest_path"]
                .as_str()
                .and_then(|path| Path::new(path).parent())
                .map(Path::to_path_buf)
                .ok_or_else(|| "cargo metadata returned an invalid manifest path".to_owned())
        })
        .collect()
}

fn default_package_ids<'a>(
    metadata: &'a serde_json::Value,
    packages: &'a [serde_json::Value],
    workspace_members: &BTreeSet<&'a str>,
) -> Result<BTreeSet<&'a str>, String> {
    let current = std::env::current_dir().map_err(|error| error.to_string())?;
    if let Some(package) = packages
        .iter()
        .filter(|package| {
            package["id"]
                .as_str()
                .is_some_and(|id| workspace_members.contains(id))
        })
        .filter(|package| {
            package["manifest_path"]
                .as_str()
                .and_then(|path| Path::new(path).parent())
                .is_some_and(|root| current.starts_with(root))
        })
        .max_by_key(|package| package["manifest_path"].as_str().map_or(0, str::len))
    {
        return Ok([package["id"].as_str().unwrap()].into_iter().collect());
    }
    Ok(metadata["workspace_default_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str())
        .collect())
}

fn discover(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = BTreeSet::new();
    for path in paths {
        if path.is_file() {
            if is_rust(path) {
                files.insert(path.clone());
            }
            continue;
        }
        if !path.is_dir() {
            return Err(format!("path does not exist: {}", path.display()));
        }

        let mut builder = WalkBuilder::new(path);
        builder
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .filter_entry(|entry| entry.file_name() != "target");
        for entry in builder.build() {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry.file_type().is_some_and(|kind| kind.is_file()) && is_rust(entry.path()) {
                files.insert(entry.into_path());
            }
        }
    }
    Ok(files.into_iter().collect())
}

fn is_rust(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
}
