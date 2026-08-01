//! Parsers for Richard Burns Rally / RallySimFans (RSF) companion files.
//!
//! RSF exports its telemetry as MoTeC `.ld` (handled by `sde-motec`), but
//! the files *around* that telemetry carry the context needed to make it
//! useful to a race engineer: which stage, which car, which setup, what
//! the surface and weather were, and whether the driver had to recover.
//! This crate reads those.
//!
//! Like the rest of `sde-formats`, it's UI-free and dependency-light (per
//! PROJECT_PLAN.md's modularity principles).
//!
//! Currently implemented:
//!
//! - [`replay`] — the replay metadata `.ini` sidecar NGPCarMenu writes
//!   next to every `.rpl`.
//! - [`install`] — install-root path discovery/configuration: resolving
//!   every location this app needs (telemetry, replays, setups, ...) from
//!   one required install root, with per-path overrides and validation.
//! - [`pairing`] — best-effort matching of a loaded telemetry file to the
//!   replay `.ini` sidecar describing the same run, since RSF/NGP has no
//!   shared filename/folder convention between the two.
//!
//! Planned (see PROJECT_PLAN.md's "RSF real-capture validation" section
//! for the reverse-engineering notes): the `.lsp` setup sheet, the
//! pacenote `.ini`, and the `.rpl` replay frame stream.

// clippy::pedantic/nursery notes, matching the convention in `sde-motec`:
// - doc_markdown fires on plain-English mentions of PROJECT_PLAN.md, RSF,
//   MoTeC and similar proper nouns throughout this crate's docs.
// - too_long_first_doc_paragraph fires on doc comments that front-load
//   full context in one paragraph rather than splitting off a terse
//   one-line summary — the chosen doc style here.
#![allow(clippy::doc_markdown, clippy::too_long_first_doc_paragraph)]

mod error;
pub mod ini;
pub mod install;
pub mod pairing;
pub mod replay;

pub use error::RbrError;
pub use ini::Ini;
pub use install::{
    read_ngp_settings, validate, InstallConfig, InstallPaths, MissingPath, NgpSettings,
    PathOverrides, ValidationReport,
};
pub use pairing::find_matching_replay_ini;
pub use replay::{
    parse_replay_ini, parse_replay_ini_str, CarInfo, Conditions, RecoverySpot, ReplayInfo,
    ResultInfo, StageInfo, Versions,
};
