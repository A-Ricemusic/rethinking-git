#[path = "support/fixture.rs"]
mod fixture;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("fixture-generator: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "--list" {
        return Err("--list takes no additional arguments".into());
    }
    if args.as_slice() == ["--list"] {
        for profile in fixture::PROFILES {
            println!(
                "{}\t{} files\t{} bytes/file\t{} directories",
                profile.name, profile.files, profile.bytes_per_file, profile.directories
            );
        }
        return Ok(());
    }
    if args.len() == 2 {
        let profile = fixture::named_profile(&args[0])
            .ok_or_else(|| format!("unknown profile `{}`; use --list", args[0]))?;
        return create(&args[1], profile);
    }
    if args.len() == 5 && args[0] == "--custom" {
        let files = parse_positive("files", &args[2])?;
        let bytes = parse_positive("bytes per file", &args[3])?;
        let directories = parse_positive("directories", &args[4])?;
        return create(
            &args[1],
            fixture::custom("custom", files, bytes, directories),
        );
    }
    Err("usage: fixture-generator <profile> <output> | --custom <output> <files> <bytes-per-file> <directories> | --list".into())
}

fn parse_positive(label: &str, value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("invalid {label}: `{value}`"))?;
    if parsed == 0 {
        Err(format!("{label} must be positive"))
    } else {
        Ok(parsed)
    }
}

fn create(output: &str, profile: fixture::FixtureProfile) -> Result<(), String> {
    let root = PathBuf::from(output);
    if root.exists()
        && root
            .read_dir()
            .map_err(|error| error.to_string())?
            .next()
            .is_some()
    {
        return Err(format!(
            "output must be absent or empty: {}",
            root.display()
        ));
    }
    let bytes = fixture::generate(&root, profile)
        .map_err(|error| format!("failed to generate {}: {error}", root.display()))?;
    println!(
        "generated profile={} files={} logical_bytes={} directories={} path={}",
        profile.name,
        profile.files,
        bytes,
        profile.directories,
        root.display()
    );
    Ok(())
}
