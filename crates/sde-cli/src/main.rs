//! `dump_channels` — minimal CLI proving the sde-formats parsers -> sde-core
//! `Session` data-model boundary works, ahead of any GUI (milestone 2's
//! goal per PROJECT_PLAN.md).
//!
//! Usage: `dump_channels <path-to-file.ld|.ibt>`

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
            eprintln!("usage: dump_channels <path-to-file.ld|.ibt>");
            return ExitCode::FAILURE;
        }
    };

    let is_ibt = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ibt"));

    let session = if is_ibt {
        sde_core::Session::load_ibt(&path).map_err(|e| e.to_string())
    } else {
        sde_core::Session::load_motec(&path).map_err(|e| e.to_string())
    };
    let session = match session {
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
    for lap in &session.laps {
        println!(
            "  Lap {:<3} start={:>10.1}ms end={:>10.1}ms dur={:>8.1}ms",
            lap.num + 1,
            lap.start_time,
            lap.end_time,
            lap.end_time - lap.start_time
        );
    }
    if !session.time_penalties.is_empty() {
        println!("Time penalties: {}", session.time_penalties.len());
        for p in &session.time_penalties {
            println!(
                "  at {:>10.1}ms  +{:.1}s",
                p.timecode_ms,
                p.penalty_ms / 1000.0
            );
        }
    }
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
            "{name:<24} unit={:<8} samples={:<6} t=[{:.1}..{:.1}] first {}: [{}]",
            channel.units,
            channel.values.len(),
            channel.timecodes.first().copied().unwrap_or(0.0),
            channel.timecodes.last().copied().unwrap_or(0.0),
            preview.len().min(PREVIEW_SAMPLES),
            preview.join(", ")
        );
    }

    ExitCode::SUCCESS
}
