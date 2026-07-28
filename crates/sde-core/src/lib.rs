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
pub use sde_shtep::ShtepError;

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

impl From<sde_shtep::ShtepChannel> for Channel {
    fn from(c: sde_shtep::ShtepChannel) -> Self {
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
const DISTANCE_CHANNEL_NAMES: &[&str] = &["LapDist", "Distance", "LapDistance_m"];

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

    /// Load a session from a `shtep`-exported `.tsv` + `.meta.json`
    /// sidecar pair on disk.
    ///
    /// # Errors
    ///
    /// Returns any [`ShtepError`] that `sde_shtep::parse` returns — i.e.
    /// if either file can't be read, the sidecar is missing/malformed, or
    /// the `.tsv` body itself is malformed.
    pub fn load_shtep(path: &Path) -> Result<Self, ShtepError> {
        let shtep_file = sde_shtep::parse(path)?;
        Ok(Self::from_shtep_file(shtep_file))
    }

    /// Build a `Session` from an already-parsed `sde_shtep::ShtepFile`.
    #[must_use]
    pub fn from_shtep_file(shtep_file: sde_shtep::ShtepFile) -> Self {
        let channels: HashMap<String, Channel> = shtep_file
            .channels
            .into_iter()
            .map(|c| (c.name.clone(), Channel::from(c)))
            .collect();

        // `SCHEMA.md` has no GPS lat/long channels (only world-space
        // Pos*_m, which `KeyChannelMap` has no slot for yet), so `lat`/
        // `long`/`alt` stay `None` for every `shtep` session.
        let key_channel_map = KeyChannelMap {
            speed: has(&channels, "Speed_kmh"),
            lat: None,
            long: None,
            alt: None,
            distance: find_distance_channel(&channels),
        };

        let end_time = channels
            .values()
            .filter_map(|c| c.timecodes.last().copied())
            .fold(0.0_f64, f64::max);

        // A rally "stage" recording is one continuous run by definition
        // (see `SCHEMA.md`'s write lifecycle: stage-start to stage-end,
        // one file), so only a "stint" session bothers looking for a
        // `LapNumber` channel to split on.
        let laps = if shtep_file.session_type == "stage" {
            boundaries_to_laps(&[0.0, end_time])
        } else {
            laps_from_lap_number_channel(&channels)
                .unwrap_or_else(|| boundaries_to_laps(&[0.0, end_time]))
        };

        let mut metadata = HashMap::new();
        metadata.insert("Sim".to_string(), shtep_file.sim);
        metadata.insert("Session Type".to_string(), shtep_file.session_type);
        metadata.insert("Context".to_string(), shtep_file.context);
        metadata.insert("Car".to_string(), shtep_file.car);
        metadata.insert("Driver".to_string(), shtep_file.driver);
        metadata.insert("Start Time (UTC)".to_string(), shtep_file.start_time_utc);
        metadata.insert("End Time (UTC)".to_string(), shtep_file.end_time_utc);

        Self {
            channels,
            laps,
            metadata,
            key_channel_map,
            file_name: shtep_file.file_name,
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

/// Derive lap boundaries from iRacing's explicit `Lap` channel. Started
/// as a near-verbatim port of TDA's `data/iracing.py` `_find_laps`, but
/// **deliberately diverges from it in one place** — see below — after
/// real-world validation against `.sample-data/iRacing/Hell RX/…`
/// surfaced a case where the oracle's own heuristic is simply wrong.
///
/// `LapCurrentLapTime` doesn't reset cleanly right at the lap boundary
/// (and never reacts at all on an out lap), so instead of trusting the
/// `Lap` sample's own timecode, the exact crossing instant is
/// back-computed from `LapDist`/`Speed`: `time_at_lapdist_0 =
/// speed.timecodes[b] - lapdist[b] / speed[b] * 1000`, clamped to be no
/// earlier than the previous sample's timecode.
///
/// **Divergence from the oracle:** TDA's `_find_laps` treats any
/// transition to `Lap == 0` as "probably no more useful data" and stops
/// looking for further laps right there. The real Hell RX rallycross
/// capture disproves that assumption outright: its `Lap` channel goes
/// `0 -> 1 -> 0 -> 1 -> 2 -> … -> 11` — a brief `Lap == 1` during the
/// pre-green-flag formation movement, then the counter resets to `0` at
/// the actual race start and counts up through eleven real racing laps
/// afterward. Applying the oracle's heuristic here truncated lap
/// detection to the first ~16.6s of a ~494s session (2 laps instead of
/// the ~13 boundaries the full file actually has). This port therefore
/// treats every `Lap` value change uniformly — including a drop back to
/// `0` — as an ordinary boundary, the same way [`laps_from_beacon`] and
/// `laps_from_lap_number_channel` (see `sde-shtep`) already do, rather
/// than special-casing zero.
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

        // A transition sample with `speed == 0` (a stationary car sitting
        // exactly on a lap boundary — not hypothetical, the real Hell RX
        // capture has exactly this at its very last sample) can make this
        // division non-finite (`±inf` when `lapdist != 0`, `NaN` when
        // both are `0`). `.max()` already discards `NaN` safely (Rust
        // defines `f64::max` to return the non-NaN operand), but `+inf`
        // is a valid `f64` that would win the `.max()` outright and
        // corrupt every boundary after it — falling back to the sample's
        // own raw timecode keeps this always finite.
        let time_at_lapdist_0 = speed.timecodes[b] - lapdist.values[b] / speed.values[b] * 1000.0;
        let time_at_lapdist_0 = if time_at_lapdist_0.is_finite() {
            time_at_lapdist_0
        } else {
            speed.timecodes[b]
        };
        let boundary = speed.timecodes[b - 1].max(time_at_lapdist_0);
        // A transition with a negative `LapDist` (the real Hell RX
        // capture has one, `-0.41` at `t=11566.7ms`) back-computes a
        // crossing time *after* the sample's own timecode, since the
        // formula assumes `lapdist >= 0`. Clamping against the previous
        // sample alone (above) doesn't protect against that overshoot
        // then landing past a *later* transition's own boundary — which
        // would otherwise make `boundaries` non-monotonic and produce a
        // `Lap` with `end_time < start_time`. Clamping against the last
        // *pushed* boundary too guarantees monotonicity regardless.
        boundaries.push(boundary.max(*boundaries.last().unwrap_or(&0.0)));
    }

    // Same monotonicity clamp as inside the loop: a large enough
    // negative-`LapDist` overshoot on the *last* transition could
    // otherwise push a final boundary past the session's own real end
    // time, producing an inverted final `Lap` instead of just a
    // degenerate (zero-length) one.
    let session_end = speed
        .timecodes
        .last()?
        .max(*boundaries.last().unwrap_or(&0.0));
    boundaries.push(session_end);

    Some(boundaries_to_laps(&boundaries))
}

/// Derive lap boundaries from a `shtep` `.tsv`'s `LapNumber` channel
/// (circuit `"stint"` sessions only — see `Session::from_shtep_file`,
/// which only calls this when the sidecar's `sessionType` isn't
/// `"stage"`). Unlike [`laps_from_lap_channel`]'s back-computed crossing
/// time (iRacing's `LapDist`/`Speed` give it exact sub-sample precision),
/// this is a simpler "boundary at the changed sample's own timecode" —
/// `SCHEMA.md`'s fixed 100 Hz default sample rate makes that a small
/// enough window not to be worth the extra complexity here; a known,
/// documented simplification, not an oversight.
fn laps_from_lap_number_channel(channels: &HashMap<String, Channel>) -> Option<Vec<Lap>> {
    let lap_number = channels.get("LapNumber")?;
    let n = lap_number.values.len();
    if n == 0 || lap_number.timecodes.len() != n {
        return None;
    }

    let mut boundaries = vec![0.0_f64];
    for b in 1..n {
        // Lap numbers are integer-valued floats, so exact comparison is
        // correct here, not a missing-epsilon bug — same rationale as
        // `laps_from_lap_channel` above.
        #[allow(clippy::float_cmp)]
        let lap_changed = lap_number.values[b] != lap_number.values[b - 1];
        if lap_changed {
            boundaries.push(lap_number.timecodes[b]);
        }
    }
    boundaries.push(*lap_number.timecodes.last()?);

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

#[cfg(test)]
mod lap_channel_tests {
    use super::*;

    fn channels(
        lap: &[f64],
        lapdist: &[f64],
        speed: &[f64],
        timecodes: &[f64],
    ) -> HashMap<String, Channel> {
        let mut m = HashMap::new();
        m.insert(
            "Lap".to_string(),
            Channel {
                name: "Lap".into(),
                units: String::new(),
                dec_pts: 0,
                interpolate: false,
                timecodes: timecodes.to_vec(),
                values: lap.to_vec(),
            },
        );
        m.insert(
            "LapDist".to_string(),
            Channel {
                name: "LapDist".into(),
                units: "m".into(),
                dec_pts: 2,
                interpolate: true,
                timecodes: timecodes.to_vec(),
                values: lapdist.to_vec(),
            },
        );
        m.insert(
            "Speed".to_string(),
            Channel {
                name: "Speed".into(),
                units: "m/s".into(),
                dec_pts: 2,
                interpolate: true,
                timecodes: timecodes.to_vec(),
                values: speed.to_vec(),
            },
        );
        m
    }

    #[test]
    fn a_zero_reset_mid_session_does_not_truncate_lap_detection() {
        // Shape of the real Hell RX rallycross capture (see
        // `laps_from_lap_channel`'s doc comment): a brief `Lap == 1`
        // during the pre-green-flag formation movement, then the counter
        // resets to `0` at the actual race start and keeps counting up
        // through several more real laps afterward. The old oracle-ported
        // heuristic stopped at the first `Lap == 0` transition and would
        // have produced only 2 laps here; the fix should see all of them.
        let timecodes = [0.0, 10_000.0, 15_000.0, 50_000.0, 90_000.0, 130_000.0];
        let lap = [0.0, 1.0, 0.0, 1.0, 2.0, 3.0];
        let lapdist = [0.0, 0.0, 0.0, 5.0, 5.0, 5.0];
        let speed = [0.0, 10.0, 10.0, 30.0, 30.0, 30.0];
        let channels = channels(&lap, &lapdist, &speed, &timecodes);

        let laps = laps_from_lap_channel(&channels).expect("Lap/LapDist/Speed all present");

        // 5 transitions (including the drop back to 0) split the session
        // into 6 laps, not the 2 the old early-termination heuristic
        // would have produced.
        assert_eq!(laps.len(), 6);
        // The final lap must reach all the way to the session's actual
        // end, not stop early at the zero-reset.
        assert_eq!(laps.last().unwrap().end_time, 130_000.0);
    }

    #[test]
    fn a_transition_at_zero_speed_never_produces_an_infinite_boundary() {
        // Regression guard for the `time_at_lapdist_0` division-by-zero
        // case (see the `is_finite()` fallback in
        // `laps_from_lap_channel`): a transition sample with `speed == 0`
        // and nonzero `lapdist` would otherwise back-compute an infinite
        // crossing time.
        let timecodes = [0.0, 10_000.0, 20_000.0];
        let lap = [0.0, 1.0, 1.0];
        let lapdist = [0.0, 996.37, 996.37]; // nonzero lapdist ...
        let speed = [0.0, 0.0, 0.0]; // ... paired with zero speed
        let channels = channels(&lap, &lapdist, &speed, &timecodes);

        let laps = laps_from_lap_channel(&channels).expect("Lap/LapDist/Speed all present");
        for lap in &laps {
            assert!(lap.start_time.is_finite());
            assert!(lap.end_time.is_finite());
        }
    }

    #[test]
    fn a_negative_lapdist_overshoot_never_produces_a_negative_duration_lap() {
        // A transition with negative `LapDist` (the real Hell RX capture
        // has one, `-0.41` at one transition) back-computes a crossing
        // time *after* the sample's own timecode, since the formula
        // assumes `lapdist >= 0`. Here the overshoot from the first
        // transition (huge negative lapdist, low speed) lands well past
        // the second transition's own (much earlier) back-computed time —
        // without clamping against the last *pushed* boundary too (not
        // just the previous raw sample), `boundaries` would go
        // non-monotonic and produce a `Lap` with `end_time < start_time`.
        let timecodes = [0.0, 1_000.0, 1_010.0, 5_000.0];
        let lap = [0.0, 1.0, 2.0, 2.0];
        let lapdist = [0.0, -500.0, 0.1, 0.1];
        let speed = [0.0, 5.0, 10.0, 10.0];
        let channels = channels(&lap, &lapdist, &speed, &timecodes);

        let laps = laps_from_lap_channel(&channels).expect("Lap/LapDist/Speed all present");
        for lap in &laps {
            assert!(
                lap.end_time >= lap.start_time,
                "lap {lap:?} has end_time before start_time"
            );
        }
    }
}
