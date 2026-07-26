//! Parser for MoTeC `.ld` binary telemetry log files.
//!
//! This crate is deliberately UI-free and dependency-light (per
//! PROJECT_PLAN.md's modularity principles) so it can be reused outside
//! the `sde-app` GUI — as a CLI tool, or in another project entirely.
//!
//! Field names on [`LdFile`] / [`LdChannel`] intentionally mirror
//! `TrackDataAnalysis`'s `data/base.py` `LogFile`/`Channel` dataclasses,
//! since `sde-core` builds its `Session` model directly on top of this
//! crate's output.
//!
//! The binary layout and the physical-value conversion formula
//! (`value = raw * mul / (scale * 10^dec_pts) + offset`) are documented
//! in `PROJECT_PLAN.md`'s "Validation findings" section; that formula is
//! `TrackDataAnalysis`'s (not `ldparser`'s — the two differ whenever
//! `shift`/`offset != 0`).

// clippy::pedantic/nursery notes (not part of the default lint set the
// project otherwise keeps clean), applying crate-wide:
// - doc_markdown fires repeatedly on plain-English mentions of MoTeC,
//   `PROJECT_PLAN.md`, TDA, etc. across this crate's doc comments;
//   backtick-wrapping every proper noun isn't worth the churn.
// - too_long_first_doc_paragraph fires on doc comments that deliberately
//   front-load full context in one paragraph rather than splitting off a
//   terse one-line summary — that's this crate's chosen doc style.
#![allow(clippy::doc_markdown, clippy::too_long_first_doc_paragraph)]

mod bytes;
mod error;
mod ldx;
mod raw;

use std::fs;
use std::io::Cursor;
use std::path::Path;

use binrw::BinReaderExt;

pub use error::{LdError, LdxError};
pub use ldx::{parse_ldx, parse_ldx_str, LdxFile};
use raw::{RawChannelRecord, RawHeader};

/// A single telemetry channel decoded from the log file.
#[derive(Debug, Clone, PartialEq)]
pub struct LdChannel {
    /// Full channel name, e.g. `"Ground Speed"`.
    pub name: String,
    /// MoTeC's separate abbreviated name field (rarely used downstream).
    pub short_name: String,
    /// Physical unit string, e.g. `"km/h"`, `"m"`, `""`.
    pub unit: String,
    /// Logging rate in Hz.
    pub sample_rate: u16,
    /// Decimal places, as recorded in the file (clamped to >= 0, matching
    /// TDA's `max(dec_pts, 0)` — the raw field can be negative, meaning
    /// "round to a coarser-than-integer precision", which isn't a valid
    /// display precision).
    pub dec_pts: i16,
    /// `false` means "hold the previous value until the next timecode"
    /// rather than interpolate — TDA's heuristic: channels with unit `s`
    /// or no unit at all are treated as discrete/event channels.
    ///
    /// That heuristic assumes real MoTeC hardware/software populates unit
    /// strings for genuine analog channels. Some game-exported `.ld` files
    /// (observed from Assetto Corsa Competizione) leave *every* channel's
    /// unit blank, which would otherwise mark all of them (including
    /// clearly continuous ones like speed/RPM/G-force) as non-interpolating.
    /// See `decode_channels`' file-level fallback for the correction applied
    /// in that case.
    pub interpolate: bool,
    /// Sample timecodes in milliseconds, strictly increasing:
    /// `i * (1000 / sample_rate)` for sample index `i`.
    ///
    /// Measured from the start of the log, except for RSF/NGP files, which
    /// [`apply_ngp_timebase`] shifts so t=0 is the *stage start*. The
    /// pre-start countdown samples then carry negative timecodes — so don't
    /// assume `timecodes[0] >= 0`. Only the origin moves; the spacing is
    /// always uniform.
    pub timecodes: Vec<f64>,
    /// Physical (converted) sample values, one per timecode.
    pub values: Vec<f64>,
}

impl LdChannel {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Header/event/venue/vehicle metadata, as key/value strings — mirrors
/// TDA's `LogFile.metadata: Dict[str, str]` in shape, but is exposed as
/// a struct here since Rust field access is nicer than stringly-typed
/// dict keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LdMetadata {
    pub device_serial: u32,
    pub device_type: String,
    /// Formatted like TDA's `"%.2f" % (raw / 100)`, e.g. `"4.20"`.
    pub device_version: String,
    pub log_date: String,
    pub log_time: String,
    pub driver: String,
    pub vehicle: String,
    pub venue: String,
    pub session: String,
    pub short_comment: String,

    // Populated only if the file has an event sub-record (event_addr != 0).
    pub event_name: Option<String>,
    pub event_session: Option<String>,
    pub long_comment: Option<String>,

    // Populated only if the event sub-record points at a venue record.
    pub venue_name: Option<String>,

    // Populated only if the venue sub-record points at a vehicle record.
    pub vehicle_id: Option<String>,
    pub vehicle_desc: Option<String>,
    pub vehicle_weight: Option<u32>,
    pub vehicle_type: Option<String>,
    pub vehicle_comment: Option<String>,
    pub diff_ratio: Option<f64>,
    /// Gear ratios for gears 1..=9, in order, skipping any that were zero
    /// in the file (matching TDA's `_set_if`, which only records nonzero
    /// values).
    pub gear_ratios: Vec<(u8, f64)>,
    pub vehicle_wheelbase_mm: Option<u16>,
}

/// A stage-time penalty detected as a discontinuity in the scored stage
/// clock (see [`apply_ngp_timebase`]). In RSF/NGP these correspond to
/// "recover vehicle" events: the car is repositioned on the road book and
/// a fixed penalty (35 s as of NGP 7.5) is added to the stage time without
/// any corresponding wall-clock time passing.
///
/// A penalty affects only the *scored* stage time, never
/// [`LdChannel::timecodes`]: no wall-clock time passes, and the sample
/// stream keeps its steady cadence straight through. So this is purely an
/// event record — but a useful one, since where and how often a driver had
/// to recover is worth showing, and it cross-checks against the
/// `[RunkiSpots]` section of the replay `.ini` sidecar.
///
/// It also means scored finish times aren't comparable between a run that
/// took a penalty and one that didn't; subtract these to compare driving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimePenalty {
    /// When the penalty was applied, on the same axis as
    /// [`LdChannel::timecodes`] (t=0 = stage start).
    pub timecode_ms: f64,
    /// Penalty added to the scored stage time, in milliseconds. Carries
    /// ~1 ms of noise from the stage clock's own quantization, so an
    /// exactly-35 s penalty reads as ~35001 ms.
    pub penalty_ms: f64,
}

/// A fully parsed MoTeC `.ld` file.
#[derive(Debug, Clone, PartialEq)]
pub struct LdFile {
    pub metadata: LdMetadata,
    /// Channels in the order they appear in the file's linked list.
    pub channels: Vec<LdChannel>,
    pub file_name: String,
    /// Stage-time penalties, in chronological order. These do not affect
    /// the channel timecodes — see [`TimePenalty`]. Empty for files without
    /// an RSF/NGP stage clock, and for clean runs that took no penalty.
    pub time_penalties: Vec<TimePenalty>,
}

impl LdFile {
    /// Look up a channel by name (first match), analogous to TDA's
    /// `LogFile.channels[name]` dict access.
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&LdChannel> {
        self.channels.iter().find(|c| c.name == name)
    }
}

/// Parse a MoTeC `.ld` file from disk.
///
/// # Errors
///
/// Returns [`LdError::Io`] if the file can't be read, or any of the
/// parse-related [`LdError`] variants ([`LdError::BadMagic`],
/// [`LdError::Binrw`], [`LdError::Truncated`],
/// [`LdError::TruncatedSampleData`], [`LdError::UnknownElemType`],
/// [`LdError::UnsupportedElemSize`]) if `data` isn't a well-formed MoTeC
/// `.ld` file.
pub fn parse(path: &Path) -> Result<LdFile, LdError> {
    let data = fs::read(path).map_err(|source| LdError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    parse_bytes(&data, file_name)
}

/// Parse a MoTeC `.ld` file already loaded into memory.
///
/// # Errors
///
/// See [`parse`]'s `# Errors` section — the same failure modes apply
/// here, minus [`LdError::Io`] (there's no filesystem access on this
/// path).
pub fn parse_bytes(data: &[u8], file_name: impl Into<String>) -> Result<LdFile, LdError> {
    let mut cursor = Cursor::new(data);
    let raw_header: RawHeader = cursor.read_le()?;

    if raw_header.ld_marker != 0x40 {
        return Err(LdError::BadMagic(raw_header.ld_marker));
    }

    let metadata = decode_metadata(data, &raw_header)?;
    let mut channels =
        decode_channels(data, raw_header.channel_meta_addr, raw_header.num_channels)?;
    let time_penalties = apply_ngp_timebase(&mut channels);

    Ok(LdFile {
        metadata,
        channels,
        file_name: file_name.into(),
        time_penalties,
    })
}

fn decode_metadata(data: &[u8], header: &RawHeader) -> Result<LdMetadata, LdError> {
    use bytes::{ascii_at, decode_ascii, u16_at, u32_at};

    let mut metadata = LdMetadata {
        device_serial: header.device_serial,
        device_type: decode_ascii(&header.device_type),
        device_version: format!("{:.2}", f64::from(header.device_version_raw) / 100.0),
        log_date: decode_ascii(&header.log_date),
        log_time: decode_ascii(&header.log_time),
        driver: decode_ascii(&header.driver),
        vehicle: decode_ascii(&header.vehicle),
        venue: decode_ascii(&header.venue),
        session: decode_ascii(&header.session),
        short_comment: decode_ascii(&header.short_comment),
        ..Default::default()
    };

    let event_addr = header.event_addr as usize;
    if event_addr != 0 {
        metadata.event_name = Some(ascii_at(data, event_addr, 64));
        metadata.event_session = Some(ascii_at(data, event_addr + 64, 64));
        metadata.long_comment = Some(ascii_at(data, event_addr + 128, 1024));

        let venue_addr = u32_at(data, event_addr + 1152)? as usize;
        if venue_addr != 0 {
            metadata.venue_name = Some(ascii_at(data, venue_addr, 64));

            let vehicle_addr = u32_at(data, venue_addr + 1098)? as usize;
            if vehicle_addr != 0 {
                metadata.vehicle_id = Some(ascii_at(data, vehicle_addr, 64));
                metadata.vehicle_desc = Some(ascii_at(data, vehicle_addr + 64, 64));

                let weight = u32_at(data, vehicle_addr + 192)?;
                if weight != 0 {
                    metadata.vehicle_weight = Some(weight);
                }
                let vtype = ascii_at(data, vehicle_addr + 196, 32);
                if !vtype.is_empty() {
                    metadata.vehicle_type = Some(vtype);
                }
                let vcomment = ascii_at(data, vehicle_addr + 228, 32);
                if !vcomment.is_empty() {
                    metadata.vehicle_comment = Some(vcomment);
                }

                let diff_raw = u16_at(data, vehicle_addr + 260)?;
                if diff_raw != 0 {
                    metadata.diff_ratio = Some(f64::from(diff_raw) / 1000.0);
                }

                for gear in 1u8..=9 {
                    let gear_raw = u16_at(data, vehicle_addr + 260 + gear as usize * 2)?;
                    if gear_raw != 0 {
                        metadata
                            .gear_ratios
                            .push((gear, f64::from(gear_raw) / 1000.0));
                    }
                }

                let wheelbase = u16_at(data, vehicle_addr + 284)?;
                if wheelbase != 0 {
                    metadata.vehicle_wheelbase_mm = Some(wheelbase);
                }
            }
        }
    }

    Ok(metadata)
}

fn decode_channels(
    data: &[u8],
    channel_meta_addr: u32,
    num_channels: u16,
) -> Result<Vec<LdChannel>, LdError> {
    let mut channels = Vec::with_capacity(num_channels as usize);
    let mut addr = channel_meta_addr;

    for _ in 0..num_channels {
        if addr == 0 {
            // Linked list ended early; trust num_channels loosely and
            // just stop rather than erroring, matching real-world files
            // that might not perfectly agree.
            break;
        }

        let slice = data.get(addr as usize..).ok_or(LdError::Truncated {
            what: "channel meta record",
            offset: addr as usize,
        })?;
        let mut cursor = Cursor::new(slice);
        let record: RawChannelRecord = cursor.read_le()?;

        channels.push(decode_channel(data, &record)?);
        addr = record.next_addr;
    }

    // Some game-exported .ld files (observed from Assetto Corsa Competizione)
    // never populate the unit field for any channel, which would otherwise
    // leave every channel — including obviously continuous ones like speed,
    // RPM, and G-force — marked non-interpolating under TDA's unit-based
    // heuristic. Only apply the correction when the *whole file* lacks
    // units (a strong signal the exporter just doesn't record them), so
    // genuine MoTeC hardware/software files keep their original behavior.
    if !channels.is_empty() && channels.iter().all(|c| c.unit.is_empty()) {
        for channel in &mut channels {
            channel.interpolate = !is_discrete_channel_name(&channel.name);
        }
    }

    Ok(channels)
}

/// Name-based fallback for channels that are clearly discrete/event
/// signals (gear number, beacon/lap markers, ABS/TC intervention flags)
/// even though they carry no unit to say so. Matched against `_`-split
/// tokens (case-insensitive) so e.g. `"LAP_BEACON"` matches via its
/// `"BEACON"` token without also matching unrelated channels that merely
/// contain the substring.
fn is_discrete_channel_name(name: &str) -> bool {
    const DISCRETE_TOKENS: &[&str] = &["GEAR", "BEACON", "ABS", "TC", "DRS", "FLAG"];
    name.split('_')
        .any(|tok| DISCRETE_TOKENS.contains(&tok.to_ascii_uppercase().as_str()))
}

/// NGP telemetry fields recorded as int32 fixed-point scaled by 10^6 with
/// `dec_pts` left at 0, so the declared conversion yields values a million
/// times too large (brake disc temperatures of 6.7e8 K rather than 672 K).
///
/// The scaling originates in NGP's own telemetry recorder, not in the
/// `.ld` conversion: the recorder's native `.tsv` output carries the same
/// `672235712` for `LF.brakeDiskTemp`, and `ngp2MoTeC` passes it through
/// unchanged.
///
/// Keyed on the field name rather than the element type on purpose: these
/// are exactly the members typed `float` in NGP's own
/// `Plugins\NGP\sdk\rbr.telemetry.data.TelemetryData.h` under
/// `BrakeDisk { layerTemperature_, temperature_, wear_ }`, while sibling
/// channels sharing the same int32 element type — `currentTyreSegment`
/// (0..7) and `helperSpringActive` (0/1) — are genuinely integral and
/// must *not* be rescaled.
///
/// A channel RSF adds later that belongs on this list but isn't yet on it
/// fails loudly (absurd magnitudes), which is the safe direction to err.
const NGP_MICRO_FIXED_POINT_FIELDS: &[&str] = &["brakeDiskLayerTemp", "brakeDiskTemp", "brakeWear"];

/// Divisor applied to [`NGP_MICRO_FIXED_POINT_FIELDS`] channels.
const NGP_MICRO_FIXED_POINT_SCALE: f64 = 1e6;

/// Display precision substituted for the exporter's bogus `dec_pts = 0`
/// on rescaled channels — without it a UI would render 2.16 % brake wear
/// as a flat `2 %`, discarding signal the correction just recovered.
const NGP_MICRO_FIXED_POINT_DEC_PTS: i16 = 3;

/// True for RSF/NGP channel names whose int32 samples are 10^6
/// fixed-point — matched on the trailing dotted component, so the
/// per-corner `"LF."`/`"RF."`/`"LB."`/`"RB."` prefixes all resolve to the
/// same NGP field name (see [`NGP_MICRO_FIXED_POINT_FIELDS`]).
fn is_ngp_micro_fixed_point(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .is_some_and(|field| NGP_MICRO_FIXED_POINT_FIELDS.contains(&field))
}

/// RSF/NGP's scored stage-clock channel, in seconds since the stage start.
const NGP_RACE_TIME_CHANNEL: &str = "raceTime";

/// A jump in the stage clock above this is a penalty, not elapsed time.
///
/// Sits in a very wide empty band: the largest genuine inter-sample gap
/// measured across clean RSF runs is ~24 ms (and a bad frame hitch is
/// still only a few hundred), while the smallest penalty NGP applies is
/// 35 s. Anything in between doesn't occur in practice.
const PENALTY_JUMP_THRESHOLD_MS: f64 = 1000.0;

// (No duplicate-nudging constant: the uniform axis this function keeps is
// strictly increasing by construction. `raceTime`'s repeated values are a
// property of the stage clock, not of the sample spacing.)

/// Rebase an RSF/NGP session's time axis so t=0 is the stage start, and
/// report any stage-time penalties.
///
/// **The uniform `index / sample_rate` axis is kept.** An earlier revision
/// of this function rebuilt the axis from `raceTime` instead, on the
/// evidence that dividing the row count by the `raceTime` span gave
/// 152.6 Hz and 154.3 Hz against a declared 144 Hz. That reasoning was
/// wrong, and the `.tsv` NGP writes alongside the `.ld` shows why: it
/// carries a `utcSystemTime` wall-clock column (dropped in `.ld`
/// conversion), and against it **both captures sample at 144.095 Hz**. The
/// declared rate is right to within 0.07%.
///
/// The phantom rates came from `raceTime` not spanning the whole
/// recording. It is the *stage* clock, so it:
///
/// - reads a flat 0 through the countdown before the start (1009 rows in
///   both captures — a fixed physics-tick count: `totalSteps` reads 5050 at
///   the start of each),
/// - jumps on a penalty without wall-clock time passing, and
/// - **freezes at the finish while recording continues** for a fixed ~20 s
///   run-out (2881 and 2882 rows), during which the car is still braking
///   from 116 and 140 km/h with `distanceToEnd` running to -277 m.
///
/// Deriving timecodes from it therefore compressed that run-out to a
/// single instant. Nothing needs correcting in the sample spacing: the
/// samples really are evenly spaced in wall-clock time.
///
/// What `raceTime` is genuinely good for, and all this function uses it
/// for:
///
/// 1. **Locating the stage start**, so the axis can be shifted to put t=0
///    there. Two runs then align with no per-run offset, and the pre-start
///    countdown rows take honest negative timecodes rather than being
///    dropped. (Driver reaction time stays inside the stage time, where it
///    belongs — the clock starts at countdown expiry, not at first motion.)
/// 2. **Detecting penalties.** A "recover vehicle" event adds a fixed 35 s
///    to the scored clock with no wall-clock time passing, so it leaves the
///    sample axis untouched and is reported as a [`TimePenalty`] event
///    only. Cross-checks against the replay `.ini`'s `[RunkiSpots]`.
///
/// A no-op returning no penalties for any file without a usable `raceTime`
/// channel, which includes every non-RSF MoTeC file.
fn apply_ngp_timebase(channels: &mut [LdChannel]) -> Vec<TimePenalty> {
    let Some(race_time) = channels.iter().find(|c| c.name == NGP_RACE_TIME_CHANNEL) else {
        return Vec::new();
    };
    let n = race_time.values.len();
    if n < 2 || race_time.timecodes.len() != n {
        return Vec::new();
    }

    // Stage start: the first row where the clock leaves 0. Everything before
    // it is the pre-start countdown.
    let start = race_time
        .values
        .iter()
        .position(|&v| v > 0.0)
        .unwrap_or(0)
        .min(n - 1);
    let origin = race_time.timecodes[start];

    // Penalties are read off the scored clock but timestamped on the
    // (unmodified, wall-clock-uniform) sample axis. The scored clock
    // advanced by `jump` across a step in which `elapsed` of real time
    // passed, so the penalty proper is the difference — recovering a round
    // 35.001 s from an observed 35.008 s jump, the residual being
    // `raceTime`'s own ~1 ms quantization.
    let penalties: Vec<TimePenalty> = (1..n)
        .filter_map(|i| {
            let jump = (race_time.values[i] - race_time.values[i - 1]) * 1000.0;
            let elapsed = race_time.timecodes[i] - race_time.timecodes[i - 1];
            (jump > PENALTY_JUMP_THRESHOLD_MS).then(|| TimePenalty {
                timecode_ms: race_time.timecodes[i] - origin,
                penalty_ms: jump - elapsed,
            })
        })
        .collect();

    // Shift only channels sharing this row count; a file mixing sample rates
    // keeps the odd ones out on their own origin rather than being silently
    // misaligned.
    for channel in channels.iter_mut().filter(|c| c.timecodes.len() == n) {
        for t in &mut channel.timecodes {
            *t -= origin;
        }
    }

    penalties
}

fn decode_channel(data: &[u8], record: &RawChannelRecord) -> Result<LdChannel, LdError> {
    let name = bytes::decode_ascii(&record.name);
    let short_name = bytes::decode_ascii(&record.short_name);
    let unit = bytes::decode_ascii(&record.unit);

    let sample_rate = record.sample_rate.max(1);
    let data_count = record.data_count as usize;
    let elem_size = record.elem_size as usize;
    let byte_len = data_count * elem_size;

    let raw_bytes = data
        .get(record.data_addr as usize..record.data_addr as usize + byte_len)
        .ok_or_else(|| LdError::TruncatedSampleData {
            name: name.clone(),
            offset: record.data_addr,
            len: byte_len,
            file_len: data.len(),
        })?;

    // TDA's formula (see PROJECT_PLAN.md "Conversion formula discrepancy"):
    //   value = raw * mul / (scale * 10^dec_pts) + offset
    // NOT ldparser's `(raw / scale * 10^-dec + shift) * mul` — the two
    // diverge whenever shift/offset != 0.
    let mul = f64::from(record.mul);
    let scale = f64::from(record.scale);
    let dec_pts = f64::from(record.dec_pts);
    let offset = f64::from(record.shift);

    // Integer element types (0/3/5) only; the same NGP field logged as a
    // float32 (element type 7) carries its true physical value directly.
    let is_int_elem = matches!(record.elem_type, 0 | 3 | 5);
    let micro_fixed_point = is_int_elem && is_ngp_micro_fixed_point(&name);
    let micro = if micro_fixed_point {
        NGP_MICRO_FIXED_POINT_SCALE
    } else {
        1.0
    };

    let denom = scale * 10f64.powf(dec_pts) * micro;
    let convert = |raw: f64| raw * mul / denom + offset;

    // Single pass from raw bytes straight to converted physical values —
    // avoids materializing an intermediate `Vec<f64>` of raw samples
    // before mapping it into the final `values` Vec, which matters for
    // logs with hundreds of channels at tens of thousands of samples
    // each (halves peak transient allocation per channel during decode).
    let values: Vec<f64> = match (record.elem_type, record.elem_size) {
        (0 | 3 | 5, 2) => raw_bytes
            .chunks_exact(2)
            .map(|c| convert(f64::from(i16::from_le_bytes([c[0], c[1]]))))
            .collect(),
        (0 | 3 | 5, 4) => raw_bytes
            .chunks_exact(4)
            .map(|c| convert(f64::from(i32::from_le_bytes([c[0], c[1], c[2], c[3]]))))
            .collect(),
        (7, 4) => raw_bytes
            .chunks_exact(4)
            .map(|c| convert(f64::from(f32::from_le_bytes([c[0], c[1], c[2], c[3]]))))
            .collect(),
        (0 | 3 | 5 | 7, size) => {
            return Err(LdError::UnsupportedElemSize {
                name,
                elem_type: record.elem_type,
                elem_size: size,
            })
        }
        (other, _) => {
            return Err(LdError::UnknownElemType {
                name,
                elem_type: other,
            })
        }
    };

    // `i as f64`: `data_count` (sample count per channel) is realistically
    // in the tens of thousands, nowhere near f64's 2^52 exact-integer
    // range, so this cast never actually loses precision.
    #[allow(clippy::cast_precision_loss)]
    let timecodes: Vec<f64> = (0..data_count)
        .map(|i| i as f64 * (1000.0 / f64::from(sample_rate)))
        .collect();

    // Best-guess heuristic ported from TDA: `interpolate = units not in ('s', '')`.
    let interpolate = unit != "s" && !unit.is_empty();

    Ok(LdChannel {
        name,
        short_name,
        unit,
        sample_rate: record.sample_rate,
        dec_pts: if micro_fixed_point {
            NGP_MICRO_FIXED_POINT_DEC_PTS
        } else {
            record.dec_pts.max(0)
        },
        interpolate,
        timecodes,
        values,
    })
}

#[cfg(test)]
mod rsf_ngp_tests {
    //! Regression tests for the two RSF/NGP exporter defects found against
    //! the real `.sample-data/RBR/` captures: 10^6 fixed-point int32
    //! channels (see [`NGP_MICRO_FIXED_POINT_FIELDS`]) and the bogus
    //! `sample_rate` (see [`apply_ngp_timebase`]). Built as in-memory `.ld`
    //! buffers so the multi-megabyte real captures aren't needed to run
    //! them.
    use super::*;

    const HEADER_LEN: usize = 1636;
    /// Bytes of `RawChannelRecord` we populate. The real on-disk record is
    /// 124 bytes, but records are reached via `next_addr`, never by
    /// sequential position, so packing them tighter is fine.
    const RECORD_LEN: usize = 84;

    struct Chan {
        name: &'static str,
        elem_type: u16,
        data: Vec<u8>,
    }

    fn f32_chan(name: &'static str, values: &[f32]) -> Chan {
        Chan {
            name,
            elem_type: 7,
            data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        }
    }

    fn i32_chan(name: &'static str, values: &[i32]) -> Chan {
        Chan {
            name,
            elem_type: 5,
            data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        }
    }

    /// Assemble a minimal but well-formed `.ld`: header, a linked list of
    /// channel meta records, then each channel's sample data.
    fn build_ld(channels: &[Chan], sample_rate: u16) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_LEN];
        buf[0..4].copy_from_slice(&0x40u32.to_le_bytes());
        #[allow(clippy::cast_possible_truncation)]
        let num_channels = channels.len() as u16;
        buf[8..12].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
        buf[86..88].copy_from_slice(&num_channels.to_le_bytes());

        let data_base = HEADER_LEN + channels.len() * RECORD_LEN;
        let mut data_addr = data_base;

        for (i, ch) in channels.iter().enumerate() {
            let mut rec = vec![0u8; RECORD_LEN];
            let next = if i + 1 == channels.len() {
                0
            } else {
                (HEADER_LEN + (i + 1) * RECORD_LEN) as u32
            };
            let count = (ch.data.len() / 4) as u32;
            rec[4..8].copy_from_slice(&next.to_le_bytes());
            rec[8..12].copy_from_slice(&(data_addr as u32).to_le_bytes());
            rec[12..16].copy_from_slice(&count.to_le_bytes());
            rec[18..20].copy_from_slice(&ch.elem_type.to_le_bytes());
            rec[20..22].copy_from_slice(&4u16.to_le_bytes()); // elem_size
            rec[22..24].copy_from_slice(&sample_rate.to_le_bytes());
            rec[24..26].copy_from_slice(&0i16.to_le_bytes()); // shift
            rec[26..28].copy_from_slice(&1i16.to_le_bytes()); // mul
            rec[28..30].copy_from_slice(&1i16.to_le_bytes()); // scale
            rec[30..32].copy_from_slice(&0i16.to_le_bytes()); // dec_pts
            let name = ch.name.as_bytes();
            rec[32..32 + name.len()].copy_from_slice(name);
            buf.extend_from_slice(&rec);
            data_addr += ch.data.len();
        }

        for ch in channels {
            buf.extend_from_slice(&ch.data);
        }
        buf
    }

    #[test]
    fn micro_fixed_point_int_channels_are_rescaled() {
        // Real first samples from Run1's LF corner. Brake disc temperatures
        // are logged as int32 kelvin * 1e6.
        let ld = build_ld(
            &[
                i32_chan("LF.brakeDiskTemp", &[672_235_712, 672_232_384]),
                i32_chan("LF.brakeWear", &[2_159_789, 2_159_789]),
            ],
            144,
        );
        let file = parse_bytes(&ld, "rsf.ld").unwrap();

        let temp = file.channel("LF.brakeDiskTemp").unwrap();
        assert!(
            (temp.values[0] - 672.235_712).abs() < 1e-6,
            "got {}",
            temp.values[0]
        );
        // The exporter's dec_pts of 0 would render 2.16% wear as a flat "2".
        assert_eq!(temp.dec_pts, NGP_MICRO_FIXED_POINT_DEC_PTS);

        let wear = file.channel("LF.brakeWear").unwrap();
        assert!(
            (wear.values[0] - 2.159_789).abs() < 1e-6,
            "got {}",
            wear.values[0]
        );
    }

    #[test]
    fn genuinely_integral_int_channels_are_left_alone() {
        // Same int32 element type as the brake channels, but these really
        // are small integers — rescaling them would be the bug.
        let ld = build_ld(
            &[
                i32_chan("LF.currentTyreSegment", &[7, 6]),
                i32_chan("LF.helperSpringActive", &[0, 1]),
            ],
            144,
        );
        let file = parse_bytes(&ld, "rsf.ld").unwrap();

        assert_eq!(
            file.channel("LF.currentTyreSegment").unwrap().values,
            vec![7.0, 6.0]
        );
        assert_eq!(
            file.channel("LF.helperSpringActive").unwrap().values,
            vec![0.0, 1.0]
        );
    }

    #[test]
    fn stage_start_becomes_the_origin_and_spacing_stays_uniform() {
        // Three countdown rows, a repeated stage-clock value, a 35 s
        // recovery penalty, then a post-finish plateau where raceTime has
        // frozen but recording continues — the shape of a real RSF stage
        // log, in ten rows. 100 Hz gives an exact 10 ms row period.
        let race_time = [
            0.0, 0.0, 0.0, 0.010, 0.020, 0.020, 0.030, 35.040, 35.050, 35.050,
        ];
        let ld = build_ld(
            &[
                f32_chan("raceTime", &race_time),
                f32_chan("speed", &[0.0; 10]),
            ],
            100,
        );
        let file = parse_bytes(&ld, "rsf.ld").unwrap();

        // Only the origin moves, to the stage start at row 3. Spacing stays
        // uniform — including across the repeat and the trailing plateau,
        // which represent real elapsed time.
        let tc = &file.channel("raceTime").unwrap().timecodes;
        let expected = [-30.0, -20.0, -10.0, 0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
        for (i, (&got, &want)) in tc.iter().zip(expected.iter()).enumerate() {
            assert!(
                (got - want).abs() < 1e-6,
                "timecode[{i}] = {got}, expected {want}"
            );
        }

        assert_eq!(tc.len(), race_time.len());
        assert!(
            tc.windows(2).all(|w| w[1] > w[0]),
            "timecodes must strictly increase: {tc:?}"
        );

        // Every channel on the same axis is rebased, not just raceTime.
        assert_eq!(&file.channel("speed").unwrap().timecodes, tc);

        // The penalty is recorded as an event without perturbing the axis.
        // Loose tolerance: the stage clock is f32 seconds, one ulp at 35 s
        // is ~3.8 us.
        assert_eq!(file.time_penalties.len(), 1);
        let p = file.time_penalties[0];
        assert!(
            (p.penalty_ms - 35_000.0).abs() < 1e-1,
            "got {}",
            p.penalty_ms
        );
        assert!((p.timecode_ms - 40.0).abs() < 1e-2, "got {}", p.timecode_ms);
    }

    #[test]
    fn post_finish_run_out_is_not_compressed() {
        // Regression test for a real defect: an earlier revision rebuilt the
        // axis from raceTime, which freezes at the finish while recording
        // continues for a fixed ~20 s. That collapsed the entire run-out —
        // the car braking from ~120 km/h to a stop, genuine telemetry — into
        // a few microseconds.
        let mut race_time = vec![0.0, 0.5, 1.0];
        race_time.resize(100, 1.0); // frozen post-finish run-out
        let ld = build_ld(&[f32_chan("raceTime", &race_time)], 100);
        let file = parse_bytes(&ld, "rsf.ld").unwrap();

        // Origin is row 1; the remaining 98 rows span 980 ms of real time.
        let tc = &file.channel("raceTime").unwrap().timecodes;
        assert!(
            (tc.last().unwrap() - 980.0).abs() < 1e-6,
            "run-out collapsed: last timecode {}",
            tc.last().unwrap()
        );
    }

    #[test]
    fn files_without_a_race_time_channel_keep_the_declared_rate() {
        // Guards the non-RSF path: a plain 10 Hz log must keep its
        // synthetic index/sample_rate axis and report no penalties.
        let ld = build_ld(&[f32_chan("Ground Speed", &[1.0, 2.0, 3.0])], 10);
        let file = parse_bytes(&ld, "hardware.ld").unwrap();

        assert_eq!(
            file.channel("Ground Speed").unwrap().timecodes,
            vec![0.0, 100.0, 200.0]
        );
        assert!(file.time_penalties.is_empty());
    }
}

#[cfg(test)]
mod error_path_tests {
    //! Deliberately-malformed byte buffers exercising each `LdError`
    //! variant, complementing the happy-path fixture-driven test in
    //! `tests/integration.rs`. These don't need real `.ld` files — just
    //! enough of the header/channel-record layout (see `raw.rs`'s
    //! documented offsets) to steer the parser into each error branch.
    use super::*;

    /// Header is 1636 bytes total (`short_comment` starts at 1572 and is
    /// 64 bytes long — the last field `raw::RawHeader` seeks to).
    const HEADER_LEN: u32 = 1636;

    fn header_bytes(ld_marker: u32, num_channels: u16, channel_meta_addr: u32) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_LEN as usize];
        buf[0..4].copy_from_slice(&ld_marker.to_le_bytes());
        buf[8..12].copy_from_slice(&channel_meta_addr.to_le_bytes());
        buf[86..88].copy_from_slice(&num_channels.to_le_bytes());
        buf
    }

    /// 84-byte channel meta record (the "contiguous real fields" prefix
    /// documented on `raw::RawChannelRecord`; the 40 bytes of trailing
    /// padding aren't needed since we never rely on sequential layout).
    #[allow(clippy::too_many_arguments)]
    fn channel_record_bytes(
        next_addr: u32,
        data_addr: u32,
        data_count: u32,
        elem_type: u16,
        elem_size: u16,
        sample_rate: u16,
        name: &str,
    ) -> Vec<u8> {
        let mut buf = vec![0u8; 84];
        buf[0..4].copy_from_slice(&0u32.to_le_bytes()); // prev_addr, unused
        buf[4..8].copy_from_slice(&next_addr.to_le_bytes());
        buf[8..12].copy_from_slice(&data_addr.to_le_bytes());
        buf[12..16].copy_from_slice(&data_count.to_le_bytes());
        buf[18..20].copy_from_slice(&elem_type.to_le_bytes());
        buf[20..22].copy_from_slice(&elem_size.to_le_bytes());
        buf[22..24].copy_from_slice(&sample_rate.to_le_bytes());
        let name_bytes = name.as_bytes();
        let n = name_bytes.len().min(32);
        buf[32..32 + n].copy_from_slice(&name_bytes[..n]);
        buf
    }

    #[test]
    fn bad_magic_is_rejected() {
        let data = header_bytes(0xff, 0, 0);
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(matches!(err, LdError::BadMagic(0xff)), "got {err:?}");
    }

    #[test]
    fn truncated_header_is_a_binrw_error_not_a_panic() {
        // Buffer far too short to hold the full header — must error
        // gracefully, not panic on an out-of-bounds seek/read.
        let data = vec![0x40, 0, 0, 0];
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(matches!(err, LdError::Binrw(_)), "got {err:?}");
    }

    #[test]
    fn channel_meta_addr_out_of_bounds_is_a_graceful_error_not_a_panic() {
        // channel_meta_addr points far past the end of the buffer —
        // regression test for a bug where `decode_channels` indexed
        // `&data[addr..]` directly and panicked on malformed input
        // instead of returning `LdError::Truncated`.
        let data = header_bytes(0x40, 1, 999_999);
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(matches!(err, LdError::Truncated { .. }), "got {err:?}");
    }

    #[test]
    fn unknown_elem_type_is_rejected() {
        let mut data = header_bytes(0x40, 1, HEADER_LEN);
        data.extend_from_slice(&channel_record_bytes(0, 0, 0, 99, 2, 1, "Weird"));
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(
            matches!(err, LdError::UnknownElemType { elem_type: 99, .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn unsupported_elem_size_is_rejected() {
        // elem_type 7 (float) only supports elem_size 4; 8 is bogus.
        let mut data = header_bytes(0x40, 1, HEADER_LEN);
        data.extend_from_slice(&channel_record_bytes(0, 0, 0, 7, 8, 1, "Weird"));
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(
            matches!(err, LdError::UnsupportedElemSize { elem_size: 8, .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn truncated_sample_data_is_rejected() {
        let record_addr = HEADER_LEN;
        let mut data = header_bytes(0x40, 1, record_addr);
        // data_addr points past the end of the buffer we're about to
        // build, with data_count=10 float32 samples (40 bytes) that
        // don't actually exist.
        let bogus_data_addr = record_addr + 84;
        data.extend_from_slice(&channel_record_bytes(
            0,
            bogus_data_addr,
            10,
            7,
            4,
            1,
            "Ground Speed",
        ));
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(
            matches!(err, LdError::TruncatedSampleData { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn io_error_on_missing_file() {
        let err = parse(std::path::Path::new("/definitely/does/not/exist/nope.ld")).unwrap_err();
        assert!(matches!(err, LdError::Io { .. }), "got {err:?}");
    }
}
