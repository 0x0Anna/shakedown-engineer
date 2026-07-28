//! Parser for `shtep` (the companion SimHub Telemetry Export Plugin
//! repo)'s TSV telemetry export format: a `{base}.tsv` data file plus a
//! `{base}.meta.json` sidecar, per `../shtep/SCHEMA.md` (v1.1).
//!
//! Unlike the other `sde-formats` crates, this format has no upstream
//! `TrackDataAnalysis` oracle to port — it's a bespoke schema this
//! project's own companion plugin writes, so `SCHEMA.md` itself is the
//! authoritative spec. No real plugin-exported fixture files exist yet
//! (the plugin is still being built) — the committed test fixture is
//! hand-authored directly from the spec, same role as the other crates'
//! synthetic fixtures.
//!
//! This crate is deliberately UI-free and dependency-light (per
//! `PROJECT_PLAN.md`'s modularity principles). Field names on
//! [`ShtepFile`]/[`ShtepChannel`] mirror the other format crates'
//! `LdFile`/`LdChannel`-shaped conventions so `sde-core` can wrap this
//! one consistently too.

mod error;
mod sidecar;

use std::fs;
use std::path::Path;

pub use error::ShtepError;
pub use sidecar::{Discontinuity, Rewind, Sidecar, SUPPORTED_SCHEMA_VERSION};

/// A single telemetry channel decoded from the `.tsv`'s data columns
/// (every column except `Time_s`, which instead becomes every channel's
/// shared `timecodes`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShtepChannel {
    /// The `.tsv` header's exact column name, e.g. `"Speed_kmh"`.
    pub name: String,
    /// Derived from the header name (see [`channel_meta`]) — `SCHEMA.md`
    /// bakes units into the header itself rather than declaring them
    /// separately, unlike MoTeC/IBT.
    pub unit: String,
    pub dec_pts: i16,
    pub interpolate: bool,
    /// Sample timecodes in milliseconds — `Time_s * 1000`, shared
    /// (uniform) across every channel in a `shtep` file, same as IBT.
    pub timecodes: Vec<f64>,
    pub values: Vec<f64>,
}

/// A fully parsed `shtep` `.tsv` + `.meta.json` pair.
#[derive(Debug, Clone, PartialEq)]
pub struct ShtepFile {
    pub sim: String,
    pub session_type: String,
    pub context: String,
    pub car: String,
    pub driver: String,
    pub start_time_utc: String,
    pub end_time_utc: String,
    /// Channels in the `.tsv` header's column order (minus `Time_s`).
    pub channels: Vec<ShtepChannel>,
    /// Optional, human-readable summaries carried straight from the
    /// sidecar — see `SCHEMA.md`'s "Discontinuity handling"/"Rewind
    /// handling" sections. Not consumed by `sde-core` yet (a documented
    /// follow-up — see `PROJECT_PLAN.md`); exposed here so a caller can
    /// still read them.
    pub discontinuities: Vec<Discontinuity>,
    pub rewinds: Vec<Rewind>,
    pub file_name: String,
}

impl ShtepFile {
    /// Look up a channel by name (first match).
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&ShtepChannel> {
        self.channels.iter().find(|c| c.name == name)
    }
}

/// Parse a `shtep`-exported `.tsv` file from disk, along with its
/// matching `{base}.meta.json` sidecar (same directory, `.tsv` swapped
/// for `.meta.json` on the shared `{base}`).
///
/// # Errors
///
/// [`ShtepError::Io`] if either file can't be read;
/// [`ShtepError::MissingSidecar`] if the sidecar doesn't exist;
/// [`ShtepError::MalformedSidecar`]/[`ShtepError::UnsupportedSchemaVersion`]
/// if the sidecar isn't valid/understood; the remaining [`ShtepError`]
/// variants for a malformed `.tsv` body — see [`parse_str`].
pub fn parse(tsv_path: &Path) -> Result<ShtepFile, ShtepError> {
    let tsv_text = fs::read_to_string(tsv_path).map_err(|source| ShtepError::Io {
        path: tsv_path.to_path_buf(),
        source,
    })?;

    let sidecar_path = sidecar_path_for(tsv_path);
    let sidecar_text = match fs::read_to_string(&sidecar_path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(ShtepError::MissingSidecar { path: sidecar_path });
        }
        Err(source) => {
            return Err(ShtepError::Io {
                path: sidecar_path,
                source,
            });
        }
    };

    let file_name = tsv_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    parse_str(&tsv_text, &sidecar_text, file_name)
}

/// The `{base}.meta.json` sidecar path for a given `{base}.tsv` path —
/// same directory, `.tsv` extension replaced with `.meta.json` on the
/// shared `{base}` stem (not just a `.with_extension("json")`, since the
/// sidecar suffix is two dotted segments, `.meta.json`, not one).
fn sidecar_path_for(tsv_path: &Path) -> std::path::PathBuf {
    let stem = tsv_path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    tsv_path.with_file_name(format!("{stem}.meta.json"))
}

/// Parse already-loaded `.tsv` text and `.meta.json` sidecar text — the
/// in-memory entry point [`parse`] delegates to after doing the file I/O,
/// and what tests exercise directly.
///
/// # Errors
///
/// See [`parse`]'s `# Errors` section — the same failure modes apply
/// here, minus [`ShtepError::Io`]/[`ShtepError::MissingSidecar`] (both
/// filesystem-specific).
pub fn parse_str(
    tsv_text: &str,
    sidecar_text: &str,
    file_name: impl Into<String>,
) -> Result<ShtepFile, ShtepError> {
    let file_name = file_name.into();
    let sidecar: Sidecar = serde_json::from_str(sidecar_text)?;
    if sidecar.schema_version > SUPPORTED_SCHEMA_VERSION {
        return Err(ShtepError::UnsupportedSchemaVersion {
            found: sidecar.schema_version,
            supported: SUPPORTED_SCHEMA_VERSION,
        });
    }

    let channels = parse_tsv(tsv_text, &file_name)?;

    Ok(ShtepFile {
        sim: sidecar.sim,
        session_type: sidecar.session_type,
        context: sidecar.context,
        car: sidecar.car,
        driver: sidecar.driver,
        start_time_utc: sidecar.start_time_utc,
        end_time_utc: sidecar.end_time_utc,
        channels,
        discontinuities: sidecar.discontinuities,
        rewinds: sidecar.rewinds,
        file_name,
    })
}

/// Parse the `.tsv` body: a header row (`Time_s` first, per `SCHEMA.md`),
/// then one tab-separated row per sample. Number parsing uses Rust's own
/// `f64::from_str`, which — like `SCHEMA.md`'s mandated
/// `CultureInfo.InvariantCulture` on the writer side — only ever accepts
/// a plain `.`-decimal ASCII format, so no locale handling is needed here
/// either.
fn parse_tsv(text: &str, file_name: &str) -> Result<Vec<ShtepChannel>, ShtepError> {
    let mut lines = text.lines();
    let header_line = lines.next().ok_or_else(|| ShtepError::EmptyFile {
        file_name: file_name.to_string(),
    })?;
    let headers: Vec<&str> = header_line.split('\t').collect();
    if headers.first() != Some(&"Time_s") {
        return Err(ShtepError::MissingTimeColumn {
            file_name: file_name.to_string(),
            found: headers.first().unwrap_or(&"").to_string(),
        });
    }

    let mut columns: Vec<Vec<f64>> = vec![Vec::new(); headers.len()];

    for (row_idx, line) in lines.enumerate() {
        // 1-indexed, plus one for the header row already consumed above.
        let line_no = row_idx + 2;
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != headers.len() {
            return Err(ShtepError::MalformedRow {
                file_name: file_name.to_string(),
                line: line_no,
                expected_columns: headers.len(),
                found_columns: fields.len(),
            });
        }
        for (col_idx, field) in fields.iter().enumerate() {
            let value: f64 = field.parse().map_err(|_| ShtepError::MalformedNumber {
                file_name: file_name.to_string(),
                line: line_no,
                column: headers[col_idx].to_string(),
                value: (*field).to_string(),
            })?;
            columns[col_idx].push(value);
        }
    }

    let timecodes: Vec<f64> = columns[0].iter().map(|&t_s| t_s * 1000.0).collect();

    let channels = headers
        .into_iter()
        .zip(columns)
        .skip(1) // Time_s itself becomes `timecodes`, not a channel.
        .map(|(name, values)| {
            let (unit, interpolate, dec_pts) = channel_meta(name);
            ShtepChannel {
                name: name.to_string(),
                unit,
                dec_pts,
                interpolate,
                timecodes: timecodes.clone(),
                values,
            }
        })
        .collect();

    Ok(channels)
}

/// `(unit, interpolate, dec_pts)` for a `.tsv` header name, derived from
/// `SCHEMA.md`'s canonical channel table — units live in the header name
/// itself (`_kmh`, `_pct`, `_deg`, `_mm`, `_m` suffixes) rather than a
/// separate declared field, unlike MoTeC/IBT, so this is a name-pattern
/// lookup instead of reading a unit field. `Paused`/`Discontinuity`/
/// `Gear`/`LapNumber` are discrete/state channels (held, not
/// interpolated) per the same reasoning `sde-motec`/`sde-ibt` use for
/// their own discrete channels; a channel outside the canonical table
/// (`SCHEMA.md` explicitly allows adding rows "as channels are needed")
/// falls back to a safe default: unitless, continuous, 2 decimal points.
fn channel_meta(name: &str) -> (String, bool, i16) {
    match name {
        "Paused" | "Discontinuity" | "Gear" | "LapNumber" => (String::new(), false, 0),
        "RPM" => ("rpm".to_string(), true, 0),
        _ if name.starts_with("TyreTemp") => ("\u{b0}C".to_string(), true, 1),
        _ if name.ends_with("_kmh") => ("km/h".to_string(), true, 2),
        _ if name.ends_with("_pct") => ("%".to_string(), true, 1),
        _ if name.ends_with("_deg") => ("deg".to_string(), true, 2),
        // Checked before `_m` below: `_mm` would otherwise also match the
        // shorter suffix and be mislabeled as meters.
        _ if name.ends_with("_mm") => ("mm".to_string(), true, 2),
        _ if name.ends_with("_m") => ("m".to_string(), true, 2),
        _ => (String::new(), true, 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SIDECAR: &str = r#"{
        "schemaVersion": 1,
        "sim": "rbr",
        "sessionType": "stage",
        "context": "Maantie 1",
        "car": "Ford Escort MkII",
        "driver": "Annalise",
        "startTimeUtc": "2026-07-27T14:32:05Z",
        "endTimeUtc": "2026-07-27T14:32:05.020Z",
        "sampleRateHz": 100,
        "channels": ["Paused", "Discontinuity", "Speed_kmh", "RPM", "Gear"],
        "pluginVersion": "0.1.0"
    }"#;

    #[test]
    fn parses_channels_and_metadata() {
        let tsv = "Time_s\tPaused\tDiscontinuity\tSpeed_kmh\tRPM\tGear\n\
                   0.000\t0\t0\t0.0\t800.0\t1\n\
                   0.010\t0\t0\t12.5\t1200.0\t1\n";
        let file = parse_str(tsv, VALID_SIDECAR, "test.tsv").unwrap();

        assert_eq!(file.sim, "rbr");
        assert_eq!(file.session_type, "stage");
        assert_eq!(file.context, "Maantie 1");
        assert_eq!(file.channels.len(), 5);

        let speed = file.channel("Speed_kmh").unwrap();
        assert_eq!(speed.unit, "km/h");
        assert!(speed.interpolate);
        assert_eq!(speed.timecodes, vec![0.0, 10.0]); // Time_s * 1000
        assert_eq!(speed.values, vec![0.0, 12.5]);

        let gear = file.channel("Gear").unwrap();
        assert!(!gear.interpolate);
        assert_eq!(gear.values, vec![1.0, 1.0]);
    }

    #[test]
    fn missing_channel_column_is_absent_not_erroring() {
        // SCHEMA.md: "Missing/unavailable channel for a given sim: omit
        // the column entirely" — a .tsv with only some of the canonical
        // channels should parse fine, exposing only what's present.
        let tsv = "Time_s\tSpeed_kmh\n0.000\t0.0\n";
        let file = parse_str(tsv, VALID_SIDECAR, "test.tsv").unwrap();
        assert_eq!(file.channels.len(), 1);
        assert!(file.channel("RPM").is_none());
    }

    #[test]
    fn rejects_missing_time_s_column() {
        let tsv = "Speed_kmh\tRPM\n0.0\t800.0\n";
        let err = parse_str(tsv, VALID_SIDECAR, "test.tsv").unwrap_err();
        assert!(matches!(err, ShtepError::MissingTimeColumn { .. }));
    }

    #[test]
    fn rejects_a_row_with_the_wrong_column_count() {
        let tsv = "Time_s\tSpeed_kmh\n0.000\t0.0\n0.010\t1.0\t2.0\n";
        let err = parse_str(tsv, VALID_SIDECAR, "test.tsv").unwrap_err();
        match err {
            ShtepError::MalformedRow {
                line,
                expected_columns,
                found_columns,
                ..
            } => {
                assert_eq!(line, 3);
                assert_eq!(expected_columns, 2);
                assert_eq!(found_columns, 3);
            }
            other => panic!("expected MalformedRow, got {other:?}"),
        }
    }

    #[test]
    fn rejects_an_unparseable_number() {
        let tsv = "Time_s\tSpeed_kmh\n0.000\tnot-a-number\n";
        let err = parse_str(tsv, VALID_SIDECAR, "test.tsv").unwrap_err();
        assert!(matches!(err, ShtepError::MalformedNumber { .. }));
    }

    #[test]
    fn rejects_a_newer_schema_version_than_this_parser_understands() {
        let sidecar = VALID_SIDECAR.replace("\"schemaVersion\": 1", "\"schemaVersion\": 99");
        let tsv = "Time_s\n0.000\n";
        let err = parse_str(tsv, &sidecar, "test.tsv").unwrap_err();
        match err {
            ShtepError::UnsupportedSchemaVersion { found, supported } => {
                assert_eq!(found, 99);
                assert_eq!(supported, SUPPORTED_SCHEMA_VERSION);
            }
            other => panic!("expected UnsupportedSchemaVersion, got {other:?}"),
        }
    }

    #[test]
    fn tolerates_unknown_sidecar_fields_and_missing_optional_ones() {
        // Backward-compat policy: absent optional fields and unrecognized
        // extra fields are both fine, not errors.
        let sidecar = r#"{
            "schemaVersion": 1,
            "somethingFutureVersionsAdd": "ignored",
            "channels": []
        }"#;
        let tsv = "Time_s\n0.000\n";
        let file = parse_str(tsv, sidecar, "test.tsv").unwrap();
        assert_eq!(file.sim, "");
        assert!(file.discontinuities.is_empty());
    }

    #[test]
    fn rejects_malformed_sidecar_json() {
        let tsv = "Time_s\n0.000\n";
        let err = parse_str(tsv, "not json", "test.tsv").unwrap_err();
        assert!(matches!(err, ShtepError::MalformedSidecar(_)));
    }

    #[test]
    fn sidecar_path_swaps_tsv_for_meta_json_on_the_shared_stem() {
        let path = Path::new("/data/rbr_maantie1_20260727_143205.tsv");
        assert_eq!(
            sidecar_path_for(path),
            Path::new("/data/rbr_maantie1_20260727_143205.meta.json")
        );
    }
}
