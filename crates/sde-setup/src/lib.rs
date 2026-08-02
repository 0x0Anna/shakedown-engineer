//! Sim-agnostic car setup sheet model, and a diff between two setups
//! (milestone 6 in PROJECT_PLAN.md — the first feature in this workspace
//! that isn't a port of `TrackDataAnalysis`).
//!
//! The model is deliberately *descriptive*, not prescriptive: a [`Setup`]
//! is an ordered list of named groups of named entries, not a fixed struct
//! of springs/dampers/ARBs. Every sim exposes a different subset of
//! adjustments under different names, and a fixed schema would either
//! discard whatever doesn't fit or force every adapter to invent values it
//! doesn't have. Ordered groups keep each sim's own sheet layout — the
//! layout its users already know — while still giving [`diff`] a stable
//! key to match entries by.
//!
//! Where this sits in the workspace: above `sde-formats` (it consumes the
//! per-sim parsers) and beside `sde-core` (which does the same for
//! telemetry). Like both, it's UI-free.
//!
//! ```no_run
//! let a = sde_setup::rbr::load_lsp("Tarmac Bumpy.lsp".as_ref()).unwrap();
//! let b = sde_setup::rbr::load_lsp("my tarmac test.lsp".as_ref()).unwrap();
//! for group in &sde_setup::diff(&a, &b).groups {
//!     for entry in &group.entries {
//!         println!("{} / {}: {}", group.name, entry.label, entry.summary());
//!     }
//! }
//! ```

// Matching the convention in the rest of the workspace: `doc_markdown`
// fires on plain-English mentions of PROJECT_PLAN.md, RBR and similar
// proper nouns, and `too_long_first_doc_paragraph` on this crate's chosen
// doc style (context first, no terse one-line summary).
#![allow(clippy::doc_markdown, clippy::too_long_first_doc_paragraph)]

pub mod rbr;

mod diff;
mod model;

pub use diff::{diff, EntryDiff, GroupDiff, SetupChange, SetupDiff};
pub use model::{Setup, SetupEntry, SetupGroup, SetupValue};
