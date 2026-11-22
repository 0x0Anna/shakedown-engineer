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
//!
//! AC Rally keeps every saved setup in one `CarSetupsDataSaveSlot.sav`
//! (in the `acr/Saved/SaveGames` folder under `%LOCALAPPDATA%`), so there:
//!   `diff_setups <file.sav>`                — list the saved setups
//!   `diff_setups <file.sav>::<name>`        — print one setup
//!   `diff_setups <file.sav>::<a> <file.sav>::<b>` — diff two of them
//! Either form can be mixed, and `<name>` can be `<name>@<car id>` when
//! two cars share a setup name.

// `doc_markdown`: fires on the plain-English `PROJECT_PLAN.md` mention
// above, matching the allow in `dump_channels`.
#![allow(clippy::doc_markdown)]

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    match args.as_slice() {
        [one] if is_bare_sav(one) => list_sav(Path::new(one)),
        [one] => match load(one) {
            Ok(setup) => {
                print_setup(&setup);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("failed to load {one}: {e}");
                ExitCode::FAILURE
            }
        },
        [left, right] => {
            let (left, right) = match (load(left), load(right)) {
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
            eprintln!("usage: diff_setups <setup.lsp|file.sav[::name]> [<other>]");
            ExitCode::FAILURE
        }
    }
}

/// A `.sav` path with no `::name` selector.
fn is_bare_sav(arg: &str) -> bool {
    !arg.contains("::")
        && Path::new(arg)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("sav"))
}

/// Load an RBR `.lsp`, or one named setup from an AC Rally `.sav`.
fn load(arg: &str) -> Result<sde_setup::Setup, String> {
    let Some((path, selector)) = arg.split_once("::") else {
        return sde_setup::rbr::load_lsp(&PathBuf::from(arg)).map_err(|e| e.to_string());
    };
    let (name, car) = match selector.split_once('@') {
        Some((n, c)) => (n, Some(c)),
        None => (selector, None),
    };
    let save = sde_acr::savegame::parse_car_setups(Path::new(path)).map_err(|e| e.to_string())?;
    save.find(name, car)
        .map(sde_setup::acr::from_saved)
        .ok_or_else(|| format!("no setup named {name:?} in {path}"))
}

fn list_sav(path: &Path) -> ExitCode {
    match sde_acr::savegame::parse_car_setups(path) {
        Ok(save) => {
            for car in &save.cars {
                println!("{}", car.car_id);
                for s in &car.setups {
                    let surface = s.surface.as_deref().unwrap_or("-");
                    println!(
                        "  {:<18} {:<28} {:<7} v{}",
                        s.name, s.stage, surface, s.game_version
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("failed to load {}: {e}", path.display());
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
