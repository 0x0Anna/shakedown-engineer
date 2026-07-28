//! Core, UI-free data model: `Session` (built on top of `sde-formats`
//! parsers) mirroring `TrackDataAnalysis`'s `data/base.py` `LogFile`
//! dataclass shape (see PROJECT_PLAN.md).
//!
//! This crate has no GUI dependency and stays that way per the
//! workspace's modularity principles — only `sde-app` (not yet built)
//! is allowed to depend on Slint.

// clippy::pedantic/nursery notes (not part of the default lint set the
// project otherwise keeps clean), applying crate-wide:
// - doc_markdown fires repeatedly on plain-English mentions of
//   `PROJECT_PLAN.md`/MoTeC in doc comments; not worth backtick-wrapping
//   every occurrence for a doc-only lint.
#![allow(clippy::doc_markdown)]

use std::collections::HashMap;
use std::path::Path;

pub mod mathexpr;

pub use sde_ibt::IbtError;
pub use sde_motec::{LdError, TimePenalty};

/// A single telemetry channel. Field names mirror TDA's `Channel`
/// dataclass (`data/base.py`).
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub name: String,
    pub units: String,
    pub dec_pts: i16,
    /// `false` means "hold the previous value until the next timecode"
    /// rather than interpolate between samples.
    pub interpolate: bool,
    /// Sample timecodes in milliseconds, strictly increasing. Usually
    /// measured from the start of the session, but see
    /// [`sde_motec::LdChannel::timecodes`]: RSF/NGP stage logs are rebased
    /// so t=0 is the stage start, which makes the pre-start idle samples
    /// negative. Don't assume `timecodes[0] >= 0`.
    pub timecodes: Vec<f64>,
    pub values: Vec<f64>,
}

impl From<sde_motec::LdChannel> for Channel {
    fn from(c: sde_motec::LdChannel) -> Self {
        Self {
            name: c.name,
            units: c.unit,
            dec_pts: c.dec_pts,
            interpolate: c.interpolate,
            timecodes: c.timecodes,
            values: c.values,
        }
    }
}

impl From<sde_ibt::IbtChannel> for Channel {
    fn from(c: sde_ibt::IbtChannel) -> Self {
        Self {
            name: c.name,
            units: c.unit,
            dec_pts: c.dec_pts,
            interpolate: c.interpolate,
            timecodes: c.timecodes,
            values: c.values,
        }
    }
}

/// A lap within the session, in milliseconds from the start of the log.
/// Mirrors TDA's `Lap` dataclass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lap {
    pub num: u32,
    pub start_time: f64,
    pub end_time: f64,
}

/// The four "key" channels a lot of TDA's views special-case, if present
/// in the session. Mirrors TDA's `LogFile.key_channel_map`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyChannelMap {
    pub speed: Option<String>,
    pub lat: Option<String>,
    pub long: Option<String>,
    pub alt: Option<String>,
    /// A monotonically-increasing-within-a-lap distance channel (e.g.
    /// iRacing's `LapDist`, meters from the current lap/stage start), if
    /// present — see [`DISTANCE_CHANNEL_NAMES`]. Backs `sde-app`'s
    /// distance x-axis mode (PROJECT_PLAN.md's "UI/UX direction" design
    /// note, principle 3).
    pub distance: Option<String>,
}

/// Channel names observed to carry a lap/stage-relative distance signal
/// across different exporters, checked in order. iRacing's IBT format
/// exposes `LapDist` directly (meters, resets to 0 each lap — see
/// `sde_ibt`'s findings in PROJECT_PLAN.md); `"Distance"` is the more
/// generic name real MoTeC hardware/software tends to use. Neither
/// project's MoTeC reference oracle (`TrackDataAnalysis`) special-cases a
/// distance channel by name the way it does `Beacon`, so this list is a
/// best-effort guess pending a confirmed real-hardware example — same
/// caveat as [`BEACON_CHANNEL_NAMES`]'s ACC-only validation.
const DISTANCE_CHANNEL_NAMES: &[&str] = &["LapDist", "Distance"];

/// Look up the first name from [`DISTANCE_CHANNEL_NAMES`] present in
/// `channels`, shared by every format's `Session` loader.
fn find_distance_channel(channels: &HashMap<String, Channel>) -> Option<String> {
    DISTANCE_CHANNEL_NAMES
        .iter()
        .find(|name| channels.contains_key(**name))
        .map(|name| (*name).to_string())
}

/// A parsed telemetry session. Mirrors TDA's `LogFile` dataclass:
/// channels keyed by name, a lap list, freeform string metadata, and the
/// key-channel map.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub channels: HashMap<String, Channel>,
    pub laps: Vec<Lap>,
    pub metadata: HashMap<String, String>,
    pub key_channel_map: KeyChannelMap,
    pub file_name: String,
    /// Stage-time penalties (RSF/NGP "recover vehicle" events). They affect
    /// only the *scored* stage time, not the channel timecodes — see
    /// [`TimePenalty`]. Empty for formats that have no such concept.
    pub time_penalties: Vec<TimePenalty>,
}

impl Session {
    /// Load a session from a MoTeC `.ld` file on disk.
    ///
    /// # Errors
    ///
    /// Returns any [`LdError`] that `sde_motec::parse` returns — i.e. if
    /// the file can't be read, or isn't a well-formed MoTeC `.ld` file.
    pub fn load_motec(path: &Path) -> Result<Self, LdError> {
        let ld_file = sde_motec::parse(path)?;
        // The `.ldx` sidecar (see `sde_motec::ldx`) is optional supplemental
        // data some exporters (e.g. Assetto Corsa Competizione) write
        // alongside the `.ld` file. Its absence or malformedness is not an
        // error for loading the session — it just means lap splitting falls
        // back to the `.ld`'s own Beacon channel, per `laps_from_beacon`.
        let ldx = sde_motec::parse_ldx(&path.with_extension("ldx")).ok();
        Ok(Self::from_ld_file_and_ldx(ld_file, ldx))
    }

    /// Build a `Session` from an already-parsed `sde_motec::LdFile`, with
    /// no `.ldx` sidecar data (lap splitting uses the `.ld`'s own Beacon
    /// channel — see `laps_from_beacon`).
    #[must_use]
    pub fn from_ld_file(ld_file: sde_motec::LdFile) -> Self {
        Self::from_ld_file_and_ldx(ld_file, None)
    }

    /// Load a session from an iRacing `.ibt` file on disk.
    ///
    /// # Errors
    ///
    /// Returns any [`IbtError`] that `sde_ibt::parse` returns — i.e. if
    /// the file can't be read, or isn't a well-formed `.ibt` file.
    pub fn load_ibt(path: &Path) -> Result<Self, IbtError> {
        let ibt_file = sde_ibt::parse(path)?;
        Ok(Self::from_ibt_file(ibt_file))
    }

    /// Build a `Session` from an already-parsed `sde_ibt::IbtFile`.
    #[must_use]
    pub fn from_ibt_file(ibt_file: sde_ibt::IbtFile) -> Self {
        let channels: HashMap<String, Channel> = ibt_file
            .channels
            .into_iter()
            .map(|c| (c.name.clone(), Channel::from(c)))
            .collect();

        let key_channel_map = KeyChannelMap {
            speed: has(&channels, "Speed"),
            lat: has(&channels, "Lat"),
            long: has(&channels, "Lon"),
            alt: has(&channels, "Alt"),
            distance: find_distance_channel(&channels),
        };

        let end_time = channels
            .values()
            .filter_map(|c| c.timecodes.last().copied())
            .fold(0.0_f64, f64::max);

        // iRacing exposes an explicit `Lap` channel directly (unlike MoTeC,
        // which needs the Beacon state machine), so this is normally
        // available; falls back to treating the whole session as one lap
        // if `Lap`/`LapDist`/`Speed` aren't all present with sane units.
        let laps = laps_from_lap_channel(&channels)
            .unwrap_or_else(|| boundaries_to_laps(&[0.0, end_time]));

        let metadata = build_ibt_metadata(&ibt_file.metadata);

        Self {
            channels,
            laps,
            metadata,
            key_channel_map,
            file_name: ibt_file.file_name,
            time_penalties: Vec::new(),
        }
    }

    fn from_ld_file_and_ldx(ld_file: sde_motec::LdFile, ldx: Option<sde_motec::LdxFile>) -> Self {
        let channels: HashMap<String, Channel> = ld_file
            .channels
            .into_iter()
            .map(|c| (c.name.clone(), Channel::from(c)))
            .collect();

        let key_channel_map = KeyChannelMap {
            speed: has(&channels, "Ground Speed"),
            lat: has(&channels, "GPS Latitude"),
            long: has(&channels, "GPS Longitude"),
            alt: has(&channels, "Altitude"),
            distance: find_distance_channel(&channels),
        };

        let end_time = channels
            .values()
            .filter_map(|c| c.timecodes.last().copied())
            .fold(0.0_f64, f64::max);

        // Prefer `.ldx` marker times when available and non-empty (e.g. ACC
        // exports, whose embedded Beacon-equivalent channel never carries
        // real trigger values — see `sde_motec::ldx`'s doc comment). Falls
        // back to the `.ld`'s own Beacon channel otherwise.
        let laps = match ldx.filter(|l| !l.marker_times_ms.is_empty()) {
            Some(ldx) => laps_from_markers(&ldx.marker_times_ms, end_time),
            None => laps_from_beacon(&channels, end_time),
        };

        let metadata = build_metadata(&ld_file.metadata);

        Self {
            channels,
            laps,
            metadata,
            key_channel_map,
            file_name: ld_file.file_name,
            time_penalties: ld_file.time_penalties,
        }
    }
}

fn has(channels: &HashMap<String, Channel>, name: &str) -> Option<String> {
    channels.contains_key(name).then(|| name.to_string())
}

/// Channel names observed to carry the beacon/lap-marker signal across
/// different exporters. Real MoTeC hardware/software uses `"Beacon"`
/// (TDA's Python oracle looks up that exact key); Assetto Corsa
/// Competizione's `.ld` export instead names it `"LAP_BEACON"`. Checked
/// in order, case-sensitively (matching each exporter's actual casing).
const BEACON_CHANNEL_NAMES: &[&str] = &["Beacon", "LAP_BEACON"];

/// Derive lap boundaries from the beacon channel (see
/// [`BEACON_CHANNEL_NAMES`]), porting the state machine in TDA's
/// `data/motec.py` `MOTEC()` (lines ~106-127) almost verbatim. If none of
/// the known beacon channel names are present, the whole session is
/// treated as a single lap (matches Python's `laps = [0]` with no
/// further appends when `'Beacon' not in data`).
///
/// `end_time` is the max timecode across all channels in the session
/// (mirrors Python's `max(np.max(d.timecodes) for d in data.values())`,
/// always appended as the final boundary regardless of the beacon channel).
fn laps_from_beacon(channels: &HashMap<String, Channel>, end_time: f64) -> Vec<Lap> {
    let mut boundaries = vec![0.0_f64];

    let beacon_channel = BEACON_CHANNEL_NAMES
        .iter()
        .find_map(|name| channels.get(*name));

    if let Some(beacon) = beacon_channel {
        let mut seq_start_tc: Option<f64> = None;
        // Mirrors Python's `last_val`, which is only ever assigned from
        // `int(v)` while `v >= 16384` (i.e. always non-negative at that
        // point), then masked/adjusted before being consumed.
        let mut last_val: i64 = 0;

        for (&tc, &v) in beacon.timecodes.iter().zip(beacon.values.iter()) {
            match seq_start_tc {
                None => {
                    if v < 0.0 {
                        seq_start_tc = Some(tc);
                    }
                }
                Some(start) => {
                    if v >= 16384.0 {
                        // `v` is a small non-negative encoded beacon value
                        // (see doc comment above) far below i64::MAX, so
                        // this cast never truncates in practice.
                        #[allow(clippy::cast_possible_truncation)]
                        {
                            last_val = v as i64;
                        }
                    } else if v >= 0.0 {
                        // `v == 100 || v == 2` — intentionally narrow,
                        // exact magic-number check per upstream (Python
                        // comment: "but not 56?"); ported as-is. These
                        // are integer-valued beacon codes, not computed
                        // floats, so exact `==` is correct here, not a
                        // missing-epsilon bug.
                        #[allow(clippy::float_cmp)]
                        let is_lap_trigger = v == 100.0 || v == 2.0;
                        if is_lap_trigger {
                            last_val &= 16383;
                            if last_val >= 8192 {
                                last_val -= 16384;
                            }
                            // `last_val` is masked to 14 bits (`&= 16383`)
                            // just above, so it's always in -16384..16384
                            // — comfortably exact in f64.
                            #[allow(clippy::cast_precision_loss)]
                            boundaries.push(start - 1000.0 + last_val as f64);
                        }
                        seq_start_tc = None;
                    }
                }
            }
        }
    }

    boundaries.push(end_time);
    boundaries_to_laps(&boundaries)
}

/// Derive lap boundaries from `.ldx` sidecar marker times (see
/// `sde_motec::ldx`), instead of the `.ld`'s own Beacon channel. `markers`
/// is expected pre-sorted ascending (as `LdxFile::marker_times_ms` is).
/// Markers outside `(0, end_time)` are dropped since they'd produce a
/// zero-length or out-of-range lap.
fn laps_from_markers(markers: &[f64], end_time: f64) -> Vec<Lap> {
    let mut boundaries = vec![0.0_f64];
    boundaries.extend(markers.iter().copied().filter(|&t| t > 0.0 && t < end_time));
    boundaries.push(end_time);
    boundaries_to_laps(&boundaries)
}

/// Turn a sorted list of lap-boundary timestamps (starting at `0.0` and
/// ending at the session's `end_time`) into consecutive `Lap`s.
fn boundaries_to_laps(boundaries: &[f64]) -> Vec<Lap> {
    boundaries
        .windows(2)
        .enumerate()
        .map(|(num, w)| {
            // A session with more than u32::MAX laps isn't realistic.
            #[allow(clippy::cast_possible_truncation)]
            let num = num as u32;
            Lap {
                num,
                start_time: w[0],
                end_time: w[1],
            }
        })
        .collect()
}

/// Derive lap boundaries from iRacing's explicit `Lap` channel, porting
/// TDA's `data/iracing.py` `_find_laps` almost verbatim.
///
/// `LapCurrentLapTime` doesn't reset cleanly right at the lap boundary
/// (and never reacts at all on an out lap), so instead of trusting the
/// `Lap` sample's own timecode, the exact crossing instant is
/// back-computed from `LapDist`/`Speed`: `time_at_lapdist_0 =
/// speed.timecodes[b] - lapdist[b] / speed[b] * 1000`, clamped to be no
/// earlier than the previous sample's timecode.
///
/// Returns `None` if `Lap`/`LapDist`/`Speed` aren't all present with the
/// expected units (`m`/`m/s`) and matching sample counts — the caller
/// falls back to a single whole-session lap in that case.
fn laps_from_lap_channel(channels: &HashMap<String, Channel>) -> Option<Vec<Lap>> {
    let lap = channels.get("Lap")?;
    let lapdist = channels.get("LapDist")?;
    let speed = channels.get("Speed")?;

    if lapdist.units != "m" || speed.units != "m/s" {
        return None;
    }
    let n = lap.values.len();
    if n == 0
        || lapdist.values.len() != n
        || speed.values.len() != n
        || speed.timecodes.len() != n
    {
        return None;
    }

    let mut boundaries = vec![0.0_f64];
    let mut add_end = true;

    for b in 1..n {
        // Lap numbers are integer-valued floats (cast from an int32
        // channel), so exact comparison is correct here, not a
        // missing-epsilon bug — same rationale as `laps_from_beacon`'s
        // magic-number check.
        #[allow(clippy::float_cmp)]
        let lap_changed = lap.values[b] != lap.values[b - 1];
        if !lap_changed {
            continue;
        }

        #[allow(clippy::float_cmp)]
        let is_reset_to_zero = lap.values[b] == 0.0;
        if is_reset_to_zero {
            // Probably no more useful data (e.g. session ended mid-lap).
            boundaries.push(speed.timecodes[b]);
            add_end = false;
            break;
        }

        let time_at_lapdist_0 = speed.timecodes[b] - lapdist.values[b] / speed.values[b] * 1000.0;
        boundaries.push(speed.timecodes[b - 1].max(time_at_lapdist_0));
    }

    if add_end {
        boundaries.push(*speed.timecodes.last()?);
    }

    Some(boundaries_to_laps(&boundaries))
}

fn build_ibt_metadata(m: &sde_ibt::IbtMetadata) -> HashMap<String, String> {
    let mut out = HashMap::new();
    out.insert("Log Date".into(), m.log_date.clone());
    out.insert("Log Time".into(), m.log_time.clone());
    out.insert("Driver".into(), m.driver.clone());
    out.insert("Venue".into(), m.venue.clone());
    out
}

fn build_metadata(m: &sde_motec::LdMetadata) -> HashMap<String, String> {
    let mut out = HashMap::new();
    out.insert("Device Serial".into(), m.device_serial.to_string());
    out.insert("Device Type".into(), m.device_type.clone());
    out.insert("Device Version".into(), m.device_version.clone());
    out.insert("Log Date".into(), m.log_date.clone());
    out.insert("Log Time".into(), m.log_time.clone());
    out.insert("Driver".into(), m.driver.clone());
    out.insert("Vehicle".into(), m.vehicle.clone());
    out.insert("Venue".into(), m.venue.clone());
    out.insert("Session".into(), m.session.clone());
    out.insert("Short Comment".into(), m.short_comment.clone());

    if let Some(v) = &m.event_name {
        out.insert("Event Name".into(), v.clone());
    }
    if let Some(v) = &m.event_session {
        out.insert("Event Session".into(), v.clone());
    }
    if let Some(v) = &m.long_comment {
        out.insert("Long Comment".into(), v.clone());
    }
    if let Some(v) = &m.venue_name {
        out.insert("Venue Name".into(), v.clone());
    }
    if let Some(v) = &m.vehicle_id {
        out.insert("Vehicle Id".into(), v.clone());
    }
    if let Some(v) = &m.vehicle_desc {
        out.insert("Vehicle Desc".into(), v.clone());
    }
    if let Some(v) = m.vehicle_weight {
        out.insert("Vehicle Weight".into(), v.to_string());
    }
    if let Some(v) = &m.vehicle_type {
        out.insert("Vehicle Type".into(), v.clone());
    }
    if let Some(v) = &m.vehicle_comment {
        out.insert("Vehicle Comment".into(), v.clone());
    }
    if let Some(v) = m.diff_ratio {
        out.insert("Diff Ratio".into(), format!("{v:.3}"));
    }
    for (gear, ratio) in &m.gear_ratios {
        out.insert(format!("Gear {gear}"), format!("{ratio:.3}"));
    }
    if let Some(v) = m.vehicle_wheelbase_mm {
        out.insert("Vehicle Wheelbase [mm]".into(), v.to_string());
    }

    out
}
