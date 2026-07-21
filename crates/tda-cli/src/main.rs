//! `dump_channels` — minimal CLI proving the tda-motec parser -> tda-core
//! `Session` data-model boundary works, ahead of any GUI (milestone 2's
//! goal per PROJECT_PLAN.md).
//!
//! Usage: `dump_channels <path-to-file.ld>`

// `doc_markdown`: fires on the plain-English `PROJECT_PLAN.md` mention
// above; not worth backtick-wrapping for a doc-only lint.
// `single_match_else`: the suggested `if let ... else { ...; return }`
// rewrite isn't clearer than the existing early-return `match` here.
#![allow(clippy::doc_markdown, clippy::single_match_else)]

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

const PREVIEW_SAMPLES: usize = 5;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let path = match args.next() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: dump_channels <path-to-file.ld>");
            return ExitCode::FAILURE;
        }
    };

    let session = match tda_core::Session::load_motec(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to load {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    println!("File: {}", session.file_name);
    if let Some(driver) = session.metadata.get("Driver") {
        println!("Driver: {driver}");
    }
    if let Some(vehicle) = session.metadata.get("Vehicle") {
        println!("Vehicle: {vehicle}");
    }
    if let Some(venue) = session.metadata.get("Venue") {
        println!("Venue: {venue}");
    }
    println!("Laps: {}", session.laps.len());
    println!("Channels: {}", session.channels.len());
    println!();

    let mut names: Vec<&String> = session.channels.keys().collect();
    names.sort();

    for name in names {
        let channel = &session.channels[name];
        let preview: Vec<String> = channel
            .values
            .iter()
            .take(PREVIEW_SAMPLES)
            .map(|v| format!("{v:.3}"))
            .collect();

        println!(
            "{name:<24} unit={:<8} samples={:<6} first {}: [{}]",
            channel.units,
            channel.values.len(),
            preview.len().min(PREVIEW_SAMPLES),
            preview.join(", ")
        );
    }

    ExitCode::SUCCESS
}
