//! `diff_setups` — print a car setup sheet, or the differences between
//! two of them (milestone 6 in PROJECT_PLAN.md).
//!
//! The CLI counterpart to `dump_channels`: it proves the
//! `sde-rbr` parser -> `sde-setup` model -> diff boundary works
//! independently of the GUI, and is the quickest way to answer "what
//! actually changed between these two runs?" against real captures.
//!
//! Usage:
//!   `diff_setups <setup.lsp>`               — print the whole sheet
//!   `diff_setups <left.lsp> <right.lsp>`    — print only what differs

// `doc_markdown`: fires on the plain-English `PROJECT_PLAN.md` mention
// above, matching the allow in `dump_channels`.
#![allow(clippy::doc_markdown)]

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<PathBuf> = env::args_os().skip(1).map(PathBuf::from).collect();

    match args.as_slice() {
        [one] => match sde_setup::rbr::load_lsp(one) {
            Ok(setup) => {
                print_setup(&setup);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("failed to load {}: {e}", one.display());
                ExitCode::FAILURE
            }
        },
        [left, right] => {
            let (left, right) = match (
                sde_setup::rbr::load_lsp(left),
                sde_setup::rbr::load_lsp(right),
            ) {
                (Ok(l), Ok(r)) => (l, r),
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("failed to load setup: {e}");
                    return ExitCode::FAILURE;
                }
            };
            print_diff(&sde_setup::diff(&left, &right));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: diff_setups <setup.lsp> [<other-setup.lsp>]");
            ExitCode::FAILURE
        }
    }
}

fn print_setup(setup: &sde_setup::Setup) {
    println!("Setup: {} ({})", setup.name, setup.source);
    if let Some(car) = &setup.car {
        println!("Car: {car}");
    }
    println!(
        "{} entries in {} groups",
        setup.entry_count(),
        setup.groups.len()
    );
    for group in &setup.groups {
        println!("\n[{}]", group.name);
        for entry in &group.entries {
            println!("  {:<34} {}", entry.label, entry.display());
        }
    }
}

fn print_diff(diff: &sde_setup::SetupDiff) {
    println!("{} -> {}", diff.left_name, diff.right_name);
    if diff.is_empty() {
        println!("No differences.");
        return;
    }
    println!("{} values differ\n", diff.change_count());

    for group in &diff.groups {
        println!("[{}]", group.name);
        for entry in &group.entries {
            let percent = entry
                .percent_change()
                .map_or(String::new(), |p| format!("  [{p:+.1}%]"));
            println!("  {:<34} {}{percent}", entry.label, entry.summary());
        }
        println!();
    }
}
