mod format;
mod syntax;

use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::ExitCode,
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

    /// Rust files or directories. Reads stdin when omitted.
    #[arg(value_name = "PATH")]
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
    if args.paths.is_empty() {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("failed to read stdin: {error}"))?;
        let formatted = format::format_source(&source, None, !args.no_rustfmt)?;
        if args.check {
            return Ok(if source == formatted {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            });
        }
        print!("{formatted}");
        return Ok(ExitCode::SUCCESS);
    }

    let files = discover(&args.paths)?;
    let mut pending = Vec::new();
    let mut had_errors = false;

    for path in files {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("{}: {error}", path.display());
                had_errors = true;
                continue;
            }
        };
        match format::format_source(&source, Some(&path), !args.no_rustfmt) {
            Ok(formatted) if formatted != source => pending.push((path, formatted)),
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
        for (path, _) in &pending {
            println!("{}", path.display());
        }
        return Ok(if pending.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        });
    }
    for (path, formatted) in pending {
        fs::write(&path, formatted)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    Ok(ExitCode::SUCCESS)
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
