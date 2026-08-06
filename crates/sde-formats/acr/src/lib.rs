//! Parser for [`acr_telemetry`](https://github.com/decnet100/acr_telemetry)'s
//! `acr_export --csv` output: a MoTeC-style CSV (header format ported from
//! [pyxrk-dev](https://github.com/mitchdetailed/pyxrk-dev), see that
//! project's `src/export/motec_csv.rs`) that carries the full ACC/AC Rally
//! shared-memory physics field set — a much larger channel list than
//! either SimHub (`sde-shtep`'s source) or `acr_telemetry`'s own MoTeC
//! `.ld` export can provide, since the `.ld` path is capped by whatever a
//! hand-written `motec_profiles/*.toml` channel mapping declares. See this
//! project's `acr-telemetry-capture-scoping` memory for the full
//! reasoning behind parsing the CSV directly instead.
//!
//! `acr_telemetry` is PolyForm Noncommercial-licensed; this crate is an
//! independent implementation written from its documented/observed output
//! format only (no code ported or vendored), the same relationship
//! `sde-shtep` has to the SimHub Telemetry Export Plugin.
//!
//! Unlike `sde-shtep`, there's no sidecar file — one CSV is fully
//! self-describing (metadata preamble + channel names + units, all in one
//! file), so [`parse`] only ever reads the one path given.
//!
//! No real `acr_export` output exists yet as a committed fixture (same gap
//! `sde-shtep` had at first) — this parser is written directly from
//! `motec_csv.rs`'s source (the exact header/row shape it writes), and
//! this module's own tests are hand-authored against that spec. It should
//! be validated against a real capture before being treated as done to the
//! same bar `sde-motec`/`sde-ibt` are.

mod error;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub use error::AcrError;

/// A single telemetry channel decoded from the CSV's data columns (every
/// column except `Time`, which instead becomes every channel's shared
/// `timecodes`).
#[derive(Debug, Clone, PartialEq)]
pub struct AcrChannel {
    /// The CSV names row's exact column name, e.g. `"speed_kmh"`.
    pub name: String,
    /// The matching column in the CSV's units row — may be empty, same as
    /// `motec_csv.rs`'s own `units` list (e.g. `gear`, `tc`, `abs` all
    /// have no unit).
    pub unit: String,
    pub dec_pts: i16,
    pub interpolate: bool,
    /// Sample timecodes in milliseconds — the `Time` column (seconds) *
    /// 1000, same convention as `sde-shtep`/`sde-ibt`.
    pub timecodes: Vec<f64>,
    pub values: Vec<f64>,
}

/// A fully parsed `acr_export` CSV.
#[derive(Debug, Clone, PartialEq)]
pub struct AcrFile {
    pub venue: String,
    pub vehicle: String,
    pub driver: String,
    pub log_date: String,
    pub log_time: String,
    /// From the metadata preamble's `"Sample Rate"` row, if present and
    /// numeric. Informational only — every channel's own `timecodes`
    /// already carries the real per-sample time, so nothing here depends
    /// on this being right.
    pub sample_rate_hz: Option<f64>,
    /// Channels in the CSV names row's column order (minus `Time`).
    pub channels: Vec<AcrChannel>,
    pub file_name: String,
}

impl AcrFile {
    /// Look up a channel by name (first match).
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&AcrChannel> {
        self.channels.iter().find(|c| c.name == name)
    }
}

/// Parse an `acr_export --csv` file from disk.
///
/// # Errors
///
/// [`AcrError::Io`] if the file can't be read; the remaining [`AcrError`]
/// variants for a malformed/unrecognized CSV body — see [`parse_str`].
pub fn parse(path: &Path) -> Result<AcrFile, AcrError> {
    let text = fs::read_to_string(path).map_err(|source| AcrError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    parse_str(&text, file_name)
}

/// Parse already-loaded CSV text — the in-memory entry point [`parse`]
/// delegates to after doing the file I/O, and what tests exercise
/// directly.
///
/// # Errors
///
/// See [`parse`]'s `# Errors` section — the same failure modes apply here,
/// minus [`AcrError::Io`] (filesystem-specific).
pub fn parse_str(text: &str, file_name: impl Into<String>) -> Result<AcrFile, AcrError> {
    let file_name = file_name.into();
    let mut lines = text.lines();

    let first_line = lines.next().ok_or_else(|| AcrError::EmptyFile {
        file_name: file_name.clone(),
    })?;
    let first_fields = split_quoted_row(first_line);
    if first_fields.first().map(String::as_str) != Some("Format")
        || first_fields.get(1).map(String::as_str) != Some("MoTeC CSV File")
    {
        return Err(AcrError::NotAcrCsv { file_name });
    }

    // The metadata preamble is a run of "key","value",... rows (only the
    // first pair per row is kept — a documented simplification, see
    // `insert_metadata_pair`) up to the row that names the channels,
    // recognized by its first field being exactly "Time" (what
    // `motec_csv.rs` always calls the elapsed-time column). Two blank rows
    // separate the preamble from that row in `acr_export`'s own output,
    // but nothing here depends on there being exactly two, or on the
    // preamble's exact row count/order — only on eventually finding the
    // "Time" row, so a future `acr_export` version adding/reordering
    // metadata rows doesn't break this parser.
    let mut metadata: HashMap<String, String> = HashMap::new();
    insert_metadata_pair(&mut metadata, &first_fields);

    let mut names: Option<Vec<String>> = None;
    for line in lines.by_ref() {
        let fields = split_quoted_row(line);
        if fields.first().map(String::as_str) == Some("Time") {
            names = Some(fields);
            break;
        }
        insert_metadata_pair(&mut metadata, &fields);
    }
    let names = names.ok_or_else(|| AcrError::MissingTimeColumn {
        file_name: file_name.clone(),
    })?;

    let units_line = lines.next().ok_or_else(|| AcrError::MissingUnitsRow {
        file_name: file_name.clone(),
    })?;
    let units = split_quoted_row(units_line);

    // One (timecode, value) pair per data-row cell; index 0 ("Time")
    // stays empty and is skipped below, since it becomes every channel's
    // own `timecodes` rather than a channel of its own.
    let mut columns: Vec<Vec<(f64, f64)>> = vec![Vec::new(); names.len()];

    for line in lines {
        if line.is_empty() {
            continue;
        }
        let fields = split_quoted_row(line);
        // A row whose column count doesn't match the names row is
        // dropped entirely rather than failing the whole load — same
        // "one bad row shouldn't sink the file" reasoning `sde-shtep`
        // applies to its own real-world write glitches (e.g. a process
        // killed mid-flush leaving a truncated final line).
        if fields.len() != names.len() {
            continue;
        }
        // A `Time` field that doesn't parse leaves nothing to key the
        // row's other values against, so the whole row is skipped rather
        // than erroring the file — same allowance `sde-shtep` makes.
        let Ok(time_s) = fields[0].parse::<f64>() else {
            continue;
        };
        let time_ms = time_s * 1000.0;

        for (col_idx, field) in fields.iter().enumerate().skip(1) {
            let value: f64 = field.parse().map_err(|_| AcrError::MalformedNumber {
                file_name: file_name.clone(),
                column: names[col_idx].clone(),
                value: field.clone(),
            })?;
            columns[col_idx].push((time_ms, value));
        }
    }

    let channels = names
        .iter()
        .enumerate()
        .skip(1) // "Time" itself becomes `timecodes`, not a channel.
        .map(|(i, name)| {
            let unit = units.get(i).cloned().unwrap_or_default();
            let interpolate = is_continuous(name);
            let (timecodes, values): (Vec<f64>, Vec<f64>) = columns[i].iter().copied().unzip();
            AcrChannel {
                name: name.clone(),
                unit,
                dec_pts: if interpolate { 2 } else { 0 },
                interpolate,
                timecodes,
                values,
            }
        })
        .collect();

    Ok(AcrFile {
        venue: metadata.remove("Venue").unwrap_or_default(),
        vehicle: metadata.remove("Vehicle").unwrap_or_default(),
        driver: metadata.remove("Driver").unwrap_or_default(),
        log_date: metadata.remove("Log Date").unwrap_or_default(),
        log_time: metadata.remove("Log Time").unwrap_or_default(),
        sample_rate_hz: metadata.get("Sample Rate").and_then(|v| v.parse().ok()),
        channels,
        file_name,
    })
}

/// Split one `"a","b","c"` CSV row into its unquoted fields. Safe to do
/// naively (`split(',')` then trim one layer of `"`) because
/// `motec_csv.rs`'s writer (`quote_row`) unconditionally wraps every
/// field in quotes with no internal escaping — its own fields (numbers,
/// short fixed strings like `"MoTeC CSV File"`/`"acr_recorder export"`)
/// never themselves contain a `,` or `"`, so there's nothing here for a
/// real escaping scheme to protect against.
fn split_quoted_row(line: &str) -> Vec<String> {
    line.split(',')
        .map(|f| f.trim_matches('"').to_string())
        .collect()
}

/// Record `fields[0]` -> `fields[1]` in `metadata` if the row has both and
/// a non-empty key. `acr_export`'s metadata rows sometimes carry a
/// *second* key/value pair further along the same row (e.g. `"Format",
/// "MoTeC CSV File","","","Workbook"` — a `Workbook` key with no value in
/// this export), which is intentionally not captured; nothing this crate
/// exposes needs it, and reading it would mean hardcoding which column
/// index every possible second pair lives at.
fn insert_metadata_pair(metadata: &mut HashMap<String, String>, fields: &[String]) {
    if let (Some(k), Some(v)) = (fields.first(), fields.get(1)) {
        if !k.is_empty() {
            metadata.insert(k.clone(), v.clone());
        }
    }
}

/// Channels that hold a discrete/state value rather than a continuously
/// varying physical quantity — held (not interpolated) between samples,
/// same distinction `sde-shtep`'s `channel_meta` makes for its own
/// discrete channels. Unlike `sde-shtep`, the CSV has no unit-suffix
/// naming convention to key off (units come from its own units row
/// instead), so this is an explicit list of the boolean/enum/counter
/// fields documented in `acr_telemetry`'s `docs/FIELDS.md`, rather than a
/// name-pattern match. Anything not listed defaults to continuous —
/// correct for the vast majority of the field set, which is real sensor
/// data (speeds, forces, temperatures, positions, angles, …).
const DISCRETE_CHANNEL_NAMES: &[&str] = &[
    "packet_id",
    "gear",
    "autoshifter_on",
    "ignition_on",
    "starter_engine_on",
    "is_engine_running",
    "tc",
    "abs",
    "pit_limiter_on",
    "tc_in_action",
    "abs_in_action",
    "is_ai_controlled",
    "drs",
    "drs_available",
    "drs_enabled",
    "p2p_activation",
    "p2p_status",
    "front_brake_compound",
    "rear_brake_compound",
    "number_of_tyres_out",
    "car_damage_front",
    "car_damage_rear",
    "car_damage_left",
    "car_damage_right",
    "car_damage_center",
    "ers_is_charging",
];

fn is_continuous(name: &str) -> bool {
    !DISCRETE_CHANNEL_NAMES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but structurally faithful stand-in for `motec_csv.rs`'s
    /// real output: the same `"Format","MoTeC CSV File"` signature and
    /// `"Time"`-led names/units row shape, trimmed to a handful of
    /// channels rather than the full ~190-field set.
    const SAMPLE_CSV: &str = concat!(
        "\"Format\",\"MoTeC CSV File\",\"\",\"\",\"Workbook\"\n",
        "\"Venue\",\"AC Rally\",\"\",\"\",\"Worksheet\"\n",
        "\"Vehicle\",\"Telemetry\",\"\",\"Vehicle Desc\"\n",
        "\"Driver\",\"Annalise\",\"\",\"\",\"Engine ID\"\n",
        "\"Device\",\"ACR Recorder\"\n",
        "\"Comment\",\"acr_recorder export\",\"\",\"\",\"Session\",\"Practice\"\n",
        "\"Log Date\",\"27/07/2026\",\"\",\"\",\"Origin Time\",\"0\",\"s\"\n",
        "\"Log Time\",\"14:32:05\",\"\",\"\",\"Start Time\",\"0\",\"s\"\n",
        "\"Sample Rate\",\"333\",\"Hz\",\"\",\"End Time\",\"1\",\"s\"\n",
        "\"Duration\",\"1\",\"s\",\"\",\"Start Distance\"\n",
        "\"Range\",\"entire outing\",\"\",\"\",\"End Distance\"\n",
        "\"Beacon Markers\",\"0\"\n",
        "\n",
        "\n",
        "\"Time\",\"gear\",\"speed_kmh\",\"rpm\"\n",
        "\"s\",\"\",\"km/h\",\"rpm\"\n",
        "\"0.000000\",\"1\",\"0.0000\",\"800\"\n",
        "\"0.010000\",\"1\",\"12.5000\",\"1200\"\n",
    );

    #[test]
    fn parses_metadata_and_channels() {
        let file = parse_str(SAMPLE_CSV, "test.csv").unwrap();

        assert_eq!(file.venue, "AC Rally");
        assert_eq!(file.vehicle, "Telemetry");
        assert_eq!(file.driver, "Annalise");
        assert_eq!(file.log_date, "27/07/2026");
        assert_eq!(file.log_time, "14:32:05");
        assert_eq!(file.sample_rate_hz, Some(333.0));
        assert_eq!(file.channels.len(), 3);

        let speed = file.channel("speed_kmh").unwrap();
        assert_eq!(speed.unit, "km/h");
        assert!(speed.interpolate);
        assert_eq!(speed.timecodes, vec![0.0, 10.0]); // Time * 1000
        assert_eq!(speed.values, vec![0.0, 12.5]);

        let gear = file.channel("gear").unwrap();
        assert!(!gear.interpolate);
        assert_eq!(gear.values, vec![1.0, 1.0]);
    }

    #[test]
    fn rejects_a_file_without_the_format_signature() {
        let text = "\"Not\",\"A Format Row\"\n\"Time\"\n\"s\"\n";
        let err = parse_str(text, "test.csv").unwrap_err();
        assert!(matches!(err, AcrError::NotAcrCsv { .. }));
    }

    #[test]
    fn rejects_an_empty_file() {
        let err = parse_str("", "test.csv").unwrap_err();
        assert!(matches!(err, AcrError::EmptyFile { .. }));
    }

    #[test]
    fn rejects_a_file_with_no_time_row() {
        let text = "\"Format\",\"MoTeC CSV File\"\n\"Venue\",\"AC Rally\"\n";
        let err = parse_str(text, "test.csv").unwrap_err();
        assert!(matches!(err, AcrError::MissingTimeColumn { .. }));
    }

    #[test]
    fn rejects_a_time_row_with_no_units_row_after_it() {
        let text = "\"Format\",\"MoTeC CSV File\"\n\"Time\",\"gear\"\n";
        let err = parse_str(text, "test.csv").unwrap_err();
        assert!(matches!(err, AcrError::MissingUnitsRow { .. }));
    }

    #[test]
    fn rejects_an_unparseable_non_time_value() {
        let text = concat!(
            "\"Format\",\"MoTeC CSV File\"\n",
            "\"Time\",\"gear\"\n",
            "\"s\",\"\"\n",
            "\"0.000000\",\"not-a-number\"\n",
        );
        let err = parse_str(text, "test.csv").unwrap_err();
        assert!(matches!(err, AcrError::MalformedNumber { .. }));
    }

    #[test]
    fn a_row_with_the_wrong_column_count_is_dropped_not_a_load_failure() {
        // Real captures can end mid-flush (process killed while
        // recording) leaving a truncated final line — losing that one
        // sample beats refusing to load the rest of a good file.
        let text = concat!(
            "\"Format\",\"MoTeC CSV File\"\n",
            "\"Time\",\"gear\"\n",
            "\"s\",\"\"\n",
            "\"0.000000\",\"1\"\n",
            "\"0.010000\",\"1\",\"2\"\n",
            "\"0.020000\",\"3\"\n",
        );
        let file = parse_str(text, "test.csv").unwrap();
        let gear = file.channel("gear").unwrap();
        assert_eq!(gear.timecodes, vec![0.0, 20.0]);
        assert_eq!(gear.values, vec![1.0, 3.0]);
    }

    #[test]
    fn a_row_with_an_unparseable_time_field_is_dropped_not_a_load_failure() {
        let text = concat!(
            "\"Format\",\"MoTeC CSV File\"\n",
            "\"Time\",\"gear\"\n",
            "\"s\",\"\"\n",
            "\"garbled\",\"1\"\n",
            "\"0.020000\",\"3\"\n",
        );
        let file = parse_str(text, "test.csv").unwrap();
        let gear = file.channel("gear").unwrap();
        assert_eq!(gear.timecodes, vec![20.0]);
        assert_eq!(gear.values, vec![3.0]);
    }

    #[test]
    fn metadata_preamble_row_count_does_not_matter() {
        // Only finding a "Time"-led row matters, not how many metadata
        // rows come before it or in what order — a future acr_export
        // version reordering/adding preamble rows shouldn't break this.
        let text = concat!(
            "\"Format\",\"MoTeC CSV File\"\n",
            "\"Venue\",\"AC Rally\"\n",
            "\"Extra Future Field\",\"some value\"\n",
            "\n",
            "\"Time\",\"gear\"\n",
            "\"s\",\"\"\n",
            "\"0.000000\",\"1\"\n",
        );
        let file = parse_str(text, "test.csv").unwrap();
        assert_eq!(file.venue, "AC Rally");
        assert_eq!(file.channels.len(), 1);
    }
}
