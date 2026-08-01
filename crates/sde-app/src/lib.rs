//! `sde-app`: the Slint GUI shell. Per `PROJECT_PLAN.md`, this is the
//! *only* crate in the workspace allowed to depend on Slint.
//!
//! The library target only exposes the Slint-free pure logic (channel
//! selection, path-building, cursor value lookup) so it can be unit
//! tested without a display. The actual window/event loop lives in
//! `main.rs`.

pub mod graph;
pub mod replay_check;
