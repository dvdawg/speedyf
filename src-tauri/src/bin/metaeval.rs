//! Grade the library scan against the metadata a library already settled.
//! See `library::metaeval`. Offline; reads the library, writes nothing to it.
//!
//! metaeval [--library <library.json>] [--results <dir>]

use speedyf_lib::engine::EngineHandle;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    if let Err(error) = run() {
        eprintln!("metaeval: {error}");
        std::process::exit(1);
    }
}

/// The app's own library on macOS; elsewhere pass `--library`.
fn default_library() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("Library/Application Support/com.speedyf.desktop/library.json")
}

fn run() -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut library = default_library();
    let mut results = manifest.join("../bench/results");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--library" => library = args.next().ok_or("--library needs a path")?.into(),
            "--results" => results = args.next().ok_or("--results needs a path")?.into(),
            "-h" | "--help" => {
                println!("metaeval [--library <library.json>] [--results <dir>]");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let scratch = env::temp_dir().join(format!("speedyf-metaeval-{stamp}"));
    fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    fs::create_dir_all(&results).map_err(|e| e.to_string())?;

    let engine = EngineHandle::start(vec![manifest.join("pdfium")], false, None);
    let report = speedyf_lib::metaeval(&engine, &library, &scratch)?;
    let _ = fs::remove_dir_all(&scratch);

    println!(
        "{} settled items, {} without a file on disk, {} scan errors",
        report.items,
        report.skipped_no_file,
        report.scan_errors.len()
    );
    for (name, tally) in [
        ("title", &report.title),
        ("authors", &report.authors),
        ("year", &report.year),
    ] {
        println!("{name:8} {} graded: {:?}", tally.graded, tally.grades);
        for (source, grades) in &tally.by_source {
            println!("         from {source}: {grades:?}");
        }
    }
    let output = results.join(format!("metaeval-{stamp}.json"));
    let json = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    fs::write(&output, json).map_err(|e| format!("cannot write {}: {e}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}
