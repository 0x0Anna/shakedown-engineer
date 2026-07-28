//! Parser for iRacing `.ibt` binary telemetry log files.
//!
//! This crate is deliberately UI-free and dependency-light (per
//! PROJECT_PLAN.md's modularity principles) so it can be reused outside
//! the `sde-app` GUI — as a CLI tool, or in another project entirely.
//!
//! Field names on [`IbtFile`] / [`IbtChannel`] intentionally mirror
//! `TrackDataAnalysis`'s `data/base.py` `LogFile`/`Channel` dataclasses
//! (see `sde-motec`'s `LdFile`/`LdChannel` for the same convention),
//! since `sde-core` builds its `Session` model directly on top of this
//! crate's output.
//!
//! The binary layout (fixed header, variable-header array, strided
//! sample buffer, trailing session-info YAML block) is documented in
//! full in `PROJECT_PLAN.md`'s "IBT (iRacing) format findings" section,
//! cross-checked against a real `.ibt` capture before this was written.
//! Lap splitting and GPS zero-filtering are ported from
//! `TrackDataAnalysis/data/iracing.py`'s `_find_laps`/`_filter_gps`; see
//! that section for why lap splitting lives in `sde-core` instead of
//! here (matching the MoTeC crate split) while GPS filtering stays in
//! this crate.

// See sde-motec's lib.rs for why these two pedantic/nursery lints are
// allowed crate-wide (doc comments deliberately mention proper nouns and
// front-load context in one paragraph).
#![allow(clippy::doc_markdown, clippy::too_long_first_doc_paragraph)]

mod error;
mod raw;

use std::fs;
use std::io::Cursor;
use std::path::Path;

use binrw::BinReaderExt;
use serde::Deserialize;

pub use error::IbtError;
use raw::{RawHeader, RawVarHeader};

/// Byte size of one `RawVarHeader` record on disk (see `raw.rs`).
const VAR_HEADER_LEN: usize = 144;

/// A single telemetry channel decoded from the log file.
#[derive(Debug, Clone, PartialEq)]
pub struct IbtChannel {
    /// Variable name, e.g. `"Speed"`, `"LapDist"`, `"Lat"`.
    pub name: String,
    /// Physical unit string, e.g. `"m/s"`, `"m"`, `"%"` (stored as a 0..1
    /// ratio in the file; converted to an actual percentage here — see
    /// `decode_var`).
    pub unit: String,
    /// `2` for float/double variables, `0` for everything else (char,
    /// bool, int, bitfield) — these carry no meaningful sub-integer
    /// precision.
    pub dec_pts: i16,
    /// `true` for float/double variables (continuous signals), `false`
    /// for char/bool/int/bitfield (discrete/state signals, held until the
    /// next sample rather than interpolated).
    pub interpolate: bool,
    /// Sample timecodes in milliseconds, strictly increasing:
    /// `i * (1000 / tick_rate)` for sample index `i`. Shared (uniform)
    /// across every channel in a `.ibt` file, unlike MoTeC where each
    /// channel may have its own `sample_rate`.
    pub timecodes: Vec<f64>,
    pub values: Vec<f64>,
}

/// Metadata mirroring the small set of fields `TrackDataAnalysis`'s
/// `iracing.py` oracle actually reads out of the session-info YAML block
/// and the binary header — not a full model of iRacing's much larger
/// session-info schema (see `PROJECT_PLAN.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IbtMetadata {
    /// Formatted `MM/DD/YYYY`, from the header's `session_start_date`
    /// (Unix timestamp), treated as UTC.
    pub log_date: String,
    /// Formatted `HH:MM:SS`, from the same timestamp.
    pub log_time: String,
    /// `DriverInfo.Drivers[].UserName` for the driver matching
    /// `DriverInfo.DriverUserID` (the active driver — `Drivers` can also
    /// list other cars/replay data).
    pub driver: String,
    /// `WeekendInfo.TrackDisplayName`.
    pub venue: String,
}

/// A fully parsed iRacing `.ibt` file.
#[derive(Debug, Clone, PartialEq)]
pub struct IbtFile {
    pub metadata: IbtMetadata,
    /// Channels in file order (the order variables appear in the
    /// variable-header array).
    pub channels: Vec<IbtChannel>,
    pub file_name: String,
}

impl IbtFile {
    /// Look up a channel by name (first match), analogous to TDA's
    /// `LogFile.channels[name]` dict access.
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&IbtChannel> {
        self.channels.iter().find(|c| c.name == name)
    }
}

/// Parse an iRacing `.ibt` file from disk.
///
/// # Errors
///
/// Returns [`IbtError::Io`] if the file can't be read, or any of the
/// parse-related [`IbtError`] variants if `data` isn't a well-formed
/// `.ibt` file.
pub fn parse(path: &Path) -> Result<IbtFile, IbtError> {
    let data = fs::read(path).map_err(|source| IbtError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    parse_bytes(&data, file_name)
}

/// Parse an iRacing `.ibt` file already loaded into memory.
///
/// # Errors
///
/// See [`parse`]'s `# Errors` section — the same failure modes apply
/// here, minus [`IbtError::Io`].
pub fn parse_bytes(data: &[u8], file_name: impl Into<String>) -> Result<IbtFile, IbtError> {
    let mut cursor = Cursor::new(data);
    let header: RawHeader = cursor.read_le()?;

    // The oracle only ever reads varBuf[0] and just warns for num_buf !=
    // 1; real captures always have exactly one buffer, so treat more as
    // an explicit "not supported yet" error rather than silently
    // dropping data.
    if header.num_buf != 1 {
        return Err(IbtError::UnsupportedBufferCount {
            num_buf: header.num_buf,
        });
    }

    let record_count = usize::try_from(header.session_record_count.max(0)).unwrap_or(0);
    let buf_len = usize::try_from(header.buf_len.max(0)).unwrap_or(0);
    let buf_offset = usize::try_from(header.var_buf[0].buf_offset.max(0)).unwrap_or(0);
    let sample_len = buf_len * record_count;
    let samples =
        data.get(buf_offset..buf_offset + sample_len)
            .ok_or(IbtError::TruncatedSampleBuffer {
                offset: buf_offset,
                len: sample_len,
                file_len: data.len(),
            })?;

    let tick_rate = header.tick_rate.max(1);
    // `record_count` (samples per channel) is realistically in the tens
    // of thousands, nowhere near f64's 2^52 exact-integer range.
    #[allow(clippy::cast_precision_loss)]
    let timecodes: Vec<f64> = (0..record_count)
        .map(|i| i as f64 * (1000.0 / f64::from(tick_rate)))
        .collect();

    let num_vars = header.num_vars.max(0);
    let mut channels = Vec::with_capacity(usize::try_from(num_vars).unwrap_or(0));
    for i in 0..num_vars {
        let offset =
            header.var_header_offset as usize + usize::try_from(i).unwrap_or(0) * VAR_HEADER_LEN;
        let slice = data.get(offset..).ok_or(IbtError::TruncatedVarHeader {
            index: i,
            offset,
            file_len: data.len(),
        })?;
        let mut var_cursor = Cursor::new(slice);
        let var: RawVarHeader = var_cursor.read_le()?;
        channels.push(decode_var(
            samples,
            record_count,
            buf_len,
            &var,
            &timecodes,
        )?);
    }

    filter_gps(&mut channels);

    let metadata = decode_metadata(data, &header)?;

    Ok(IbtFile {
        metadata,
        channels,
        file_name: file_name.into(),
    })
}

/// Byte width of one variable's sample, by its `RawVarHeader::var_type`
/// index (`0..=5`); `None` for anything else.
fn elem_size(var_type: i32) -> Option<usize> {
    match var_type {
        0 | 1 => Some(1),
        2..=4 => Some(4),
        5 => Some(8),
        _ => None,
    }
}

fn decode_var(
    samples: &[u8],
    record_count: usize,
    buf_len: usize,
    var: &RawVarHeader,
    timecodes: &[f64],
) -> Result<IbtChannel, IbtError> {
    let name = decode_ascii(&var.name);
    let unit = decode_ascii(&var.unit);

    let elem_size = elem_size(var.var_type).ok_or_else(|| IbtError::UnknownVarType {
        name: name.clone(),
        var_type: var.var_type,
    })?;
    let var_offset = usize::try_from(var.offset.max(0)).unwrap_or(0);

    let required_end = if record_count == 0 {
        0
    } else {
        (record_count - 1) * buf_len + var_offset + elem_size
    };
    if samples.len() < required_end {
        return Err(IbtError::TruncatedSampleBuffer {
            offset: var_offset,
            len: required_end,
            file_len: samples.len(),
        });
    }

    let mut values = Vec::with_capacity(record_count);
    for r in 0..record_count {
        let start = r * buf_len + var_offset;
        let bytes = &samples[start..start + elem_size];
        let v = match var.var_type {
            0 => f64::from(bytes[0]),
            1 => f64::from(u8::from(bytes[0] != 0)),
            2 => f64::from(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            3 => f64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            4 => f64::from(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            5 => f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
            _ => unreachable!("var_type validated by elem_size() above"),
        };
        values.push(v);
    }

    // iRacing stores '%' variables as a 0..1 ratio, not an actual
    // percentage — ported from the oracle's `_decode_var`.
    if unit == "%" {
        for v in &mut values {
            *v *= 100.0;
        }
    }

    Ok(IbtChannel {
        name,
        unit,
        dec_pts: if var.var_type >= 4 { 2 } else { 0 },
        interpolate: var.var_type >= 4,
        timecodes: timecodes.to_vec(),
        values,
    })
}

/// Zero out `Lat`/`Lon`/`Alt` samples taken before GPS lock (or for
/// cars/replay data that never had it) — ported from the oracle's
/// `_filter_gps`. A no-op unless all three channels are present.
fn filter_gps(channels: &mut [IbtChannel]) {
    let Some(lat_idx) = channels.iter().position(|c| c.name == "Lat") else {
        return;
    };
    let Some(lon_idx) = channels.iter().position(|c| c.name == "Lon") else {
        return;
    };
    let Some(alt_idx) = channels.iter().position(|c| c.name == "Alt") else {
        return;
    };

    let keep: Vec<bool> = channels[lat_idx]
        .values
        .iter()
        .zip(channels[lon_idx].values.iter())
        .map(|(&lat, &lon)| lat != 0.0 || lon != 0.0)
        .collect();

    for idx in [lat_idx, lon_idx, alt_idx] {
        let channel = &mut channels[idx];
        let (timecodes, values) = channel
            .timecodes
            .iter()
            .zip(channel.values.iter())
            .zip(keep.iter())
            .filter(|(_, &k)| k)
            .map(|((&t, &v), _)| (t, v))
            .unzip();
        channel.timecodes = timecodes;
        channel.values = values;
    }
}

#[derive(Debug, Deserialize)]
struct SessionInfoYaml {
    #[serde(rename = "WeekendInfo", default)]
    weekend_info: Option<WeekendInfoYaml>,
    #[serde(rename = "DriverInfo", default)]
    driver_info: Option<DriverInfoYaml>,
}

#[derive(Debug, Deserialize)]
struct WeekendInfoYaml {
    #[serde(rename = "TrackDisplayName", default)]
    track_display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DriverInfoYaml {
    #[serde(rename = "DriverUserID", default)]
    driver_user_id: Option<i64>,
    #[serde(rename = "Drivers", default)]
    drivers: Option<Vec<DriverYaml>>,
}

#[derive(Debug, Deserialize)]
struct DriverYaml {
    #[serde(rename = "UserID", default)]
    user_id: Option<i64>,
    #[serde(rename = "UserName", default)]
    user_name: Option<String>,
}

fn decode_metadata(data: &[u8], header: &RawHeader) -> Result<IbtMetadata, IbtError> {
    let offset = usize::try_from(header.session_info_offset.max(0)).unwrap_or(0);
    let len = usize::try_from(header.session_info_len.max(0)).unwrap_or(0);
    let yaml_bytes = data
        .get(offset..offset + len)
        .ok_or(IbtError::TruncatedSessionInfo {
            offset,
            len,
            file_len: data.len(),
        })?;

    let yaml_str = String::from_utf8_lossy(yaml_bytes);
    let info: SessionInfoYaml = serde_yaml::from_str(&yaml_str)?;

    let driver = info
        .driver_info
        .as_ref()
        .and_then(|di| {
            let uid = di.driver_user_id?;
            di.drivers
                .as_ref()?
                .iter()
                .find(|d| d.user_id == Some(uid))?
                .user_name
                .clone()
        })
        .unwrap_or_default();

    let venue = info
        .weekend_info
        .and_then(|w| w.track_display_name)
        .unwrap_or_default();

    let (log_date, log_time) = format_log_date_time(header.session_start_date);

    Ok(IbtMetadata {
        log_date,
        log_time,
        driver,
        venue,
    })
}

/// Decode a NUL-terminated (or NUL-padded) ASCII byte slice into a
/// `String`, matching TDA's `_dec_str` (truncate at the first NUL, no
/// trimming — unlike `sde-motec`'s equivalent helper, since MoTeC names
/// don't rely on it either way in practice, but staying literal here
/// keeps this port faithful to the oracle it's cross-checked against).
fn decode_ascii(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Format a Unix timestamp as `(MM/DD/YYYY, HH:MM:SS)`, treating it as
/// UTC. The oracle formats via `time.localtime`, which depends on the
/// *parsing* machine's timezone rather than the recording sim's — an
/// already-nondeterministic choice not worth replicating (and not worth
/// pulling in a date/time crate for two formatted strings).
fn format_log_date_time(unix_seconds: u32) -> (String, String) {
    let total_secs = i64::from(unix_seconds);
    let days = total_secs.div_euclid(86400);
    let secs_of_day = total_secs.rem_euclid(86400);

    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    (
        format!("{month:02}/{day:02}/{year}"),
        format!("{hour:02}:{minute:02}:{second:02}"),
    )
}

/// Days-since-Unix-epoch to civil (year, month, day), Howard Hinnant's
/// well-known proleptic-Gregorian algorithm
/// (<https://howardhinnant.github.io/date_algorithms.html#civil_from_days>).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    #[allow(clippy::cast_sign_loss)]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    #[allow(clippy::cast_sign_loss)]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod date_tests {
    use super::format_log_date_time;

    #[test]
    fn known_epoch_values_format_correctly() {
        // 2026-07-27 21:16:26 UTC
        let (date, time) = format_log_date_time(1_785_186_986);
        assert_eq!(date, "07/27/2026");
        assert_eq!(time, "21:16:26");
    }

    #[test]
    fn epoch_zero_is_new_year_1970() {
        let (date, time) = format_log_date_time(0);
        assert_eq!(date, "01/01/1970");
        assert_eq!(time, "00:00:00");
    }
}
