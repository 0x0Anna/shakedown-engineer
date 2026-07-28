//! `{base}.meta.json` sidecar model — see `../shtep/SCHEMA.md` (v1.1) for
//! the full spec this mirrors.
//!
//! Compatibility policy per `SCHEMA.md`: unknown/absent fields are not an
//! error (`serde` already ignores fields it doesn't know about, and every
//! field below that isn't part of the schema's required core is marked
//! `#[serde(default)]`) — only a `schemaVersion` *newer* than
//! [`SUPPORTED_SCHEMA_VERSION`] is refused (checked by the caller in
//! `lib.rs`, not here, since rejecting is a parse-level decision, not a
//! deserialization one).

use serde::Deserialize;

/// The `schemaVersion` this parser was written against and understands.
/// Sidecars declaring anything higher are refused (see
/// `ShtepError::UnsupportedSchemaVersion`); anything lower or equal is
/// accepted, missing/unknown fields and all.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidecar {
    pub schema_version: u32,
    #[serde(default)]
    pub sim: String,
    /// `"stage"` (rally, whole file = one stage) or `"stint"` (circuit,
    /// pit-exit to pit-entry) — kept as a plain string rather than an enum
    /// so an unrecognized future value doesn't fail to parse, matching the
    /// module's backward-compatible-by-intent stance.
    #[serde(default)]
    pub session_type: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub car: String,
    #[serde(default)]
    pub driver: String,
    #[serde(default)]
    pub start_time_utc: String,
    #[serde(default)]
    pub end_time_utc: String,
    #[serde(default)]
    pub sample_rate_hz: f64,
    /// Exact list/order of data columns actually present in the paired
    /// `.tsv`, per `SCHEMA.md` — informational only here; the parser
    /// matches the `.tsv`'s own header row by name, never trusting this
    /// list blindly (same caveat `SCHEMA.md` itself calls out).
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub discontinuities: Vec<Discontinuity>,
    #[serde(default)]
    pub rewinds: Vec<Rewind>,
    #[serde(default)]
    pub plugin_version: String,
    #[serde(default)]
    pub recovered_from_crash: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discontinuity {
    pub start_time_s: f64,
    pub end_time_s: f64,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rewind {
    pub truncated_from_time_s: f64,
    pub truncated_to_time_s: f64,
    #[serde(default)]
    pub rows_removed: u64,
}
