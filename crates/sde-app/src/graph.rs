//! Pure, Slint-free graph/cursor logic: turning a `sde_core::Channel`
//! into an SVG-style path string for the `Path` element, and looking up
//! the value at an arbitrary cursor time. Kept separate from `main.rs`
//! so it can be unit tested without any GUI dependency or display.

// clippy::pedantic/nursery notes (not part of the default lint set the
// project otherwise keeps clean):
// - too_long_first_doc_paragraph fires on several of this module's doc
//   comments, which deliberately front-load full context in one
//   paragraph rather than splitting off a one-line summary; that's the
//   project's chosen doc style, not an oversight.
// - suboptimal_flops wants `mul_add` for the y-flip/lerp arithmetic
//   below; for graph-plotting math running once per sample on datasets
//   of realistic size, the marginal FMA precision/perf gain isn't worth
//   the readability hit.
#![allow(clippy::too_long_first_doc_paragraph, clippy::suboptimal_flops)]

use std::fmt::Write as _;

use sde_core::Channel;

/// A normalized plot: an SVG `Path`-compatible `commands` string plus the
/// `viewbox-width`/`viewbox-height` it was built against, and the time
/// range (ms) it covers (needed to convert a 0..1 cursor fraction back to
/// a timecode).
#[derive(Debug, Clone, PartialEq)]
pub struct PlotData {
    pub commands: String,
    pub view_width: f64,
    pub view_height: f64,
    pub min_time: f64,
    pub max_time: f64,
}

/// Build an SVG path (`M x y L x y L x y ...`) plotting `channel.values`
/// against `channel.timecodes`, normalized into a `view_width` x
/// `view_height` box. Y is flipped (SVG/Slint y grows downward, but a
/// larger channel value should plot higher on screen).
///
/// Returns `None` if the channel has no samples.
///
/// # Panics
///
/// Never panics in practice: the `.last().unwrap()` below is only
/// reached after the empty-channel check above, so `timecodes` is
/// guaranteed non-empty at that point.
#[must_use]
pub fn build_plot(channel: &Channel, view_width: f64, view_height: f64) -> Option<PlotData> {
    if channel.timecodes.is_empty() {
        return None;
    }

    let min_time = channel.timecodes[0];
    let max_time = *channel.timecodes.last().unwrap();
    let time_span = (max_time - min_time).max(f64::EPSILON);

    let min_val = channel.values.iter().copied().fold(f64::INFINITY, f64::min);
    let max_val = channel
        .values
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let val_span = (max_val - min_val).max(f64::EPSILON);

    let mut commands = String::new();
    for (i, (&t, &v)) in channel
        .timecodes
        .iter()
        .zip(channel.values.iter())
        .enumerate()
    {
        let x = (t - min_time) / time_span * view_width;
        // flip: larger value -> smaller y (closer to top)
        let y = view_height - (v - min_val) / val_span * view_height;
        // `write!` to a String never fails.
        let _ = if i == 0 {
            write!(commands, "M {x} {y} ")
        } else {
            write!(commands, "L {x} {y} ")
        };
    }

    Some(PlotData {
        commands,
        view_width,
        view_height,
        min_time,
        max_time,
    })
}

/// Look up the value of `channel` at time `t` (milliseconds), respecting
/// `channel.interpolate`: when `true`, linearly interpolate between the
/// bracketing samples; when `false`, hold the previous sample's value
/// (matching `sde-core`'s `Channel::interpolate` semantics, ported from
/// TDA's `data/base.py` `Channel` dataclass).
///
/// `t` is clamped to the channel's time range: values before the first
/// sample hold the first sample, values after the last sample hold the
/// last sample. Returns `None` only if the channel has no samples.
#[must_use]
pub fn value_at(channel: &Channel, t: f64) -> Option<f64> {
    value_at_raw(&channel.timecodes, &channel.values, channel.interpolate, t)
}

/// Same as [`value_at`] but operating on raw slices, so it can be unit
/// tested with synthetic data independent of `Channel`/parsing.
///
/// # Panics
///
/// Never panics in practice: `partial_cmp` only returns `None` for NaN
/// operands, and telemetry timecodes are never NaN.
#[must_use]
pub fn value_at_raw(timecodes: &[f64], values: &[f64], interpolate: bool, t: f64) -> Option<f64> {
    if timecodes.is_empty() || timecodes.len() != values.len() {
        return None;
    }

    if t <= timecodes[0] {
        return Some(values[0]);
    }
    let last_idx = timecodes.len() - 1;
    if t >= timecodes[last_idx] {
        return Some(values[last_idx]);
    }

    // `t` is strictly between the first and last timecode here, so
    // binary_search's Err(insertion point) is guaranteed to be in
    // 1..=last_idx, making `i0 = idx - 1` safe.
    let idx = match timecodes.binary_search_by(|probe| probe.partial_cmp(&t).unwrap()) {
        Ok(i) => return Some(values[i]),
        Err(i) => i,
    };
    let i0 = idx - 1;
    let i1 = idx;

    if interpolate {
        let (t0, t1) = (timecodes[i0], timecodes[i1]);
        let (v0, v1) = (values[i0], values[i1]);
        let frac = (t - t0) / (t1 - t0);
        Some(v0 + (v1 - v0) * frac)
    } else {
        Some(values[i0])
    }
}

/// Pick which channel to plot by default: the first `interpolate == true`
/// channel in alphabetical order by name, or if none qualify, simply the
/// first channel alphabetically. This is a deliberate milestone-3
/// shortcut — no channel-picker UI exists yet (deferred to milestone 5's
/// "core UI parity").
#[must_use]
pub fn pick_default_channel(session: &sde_core::Session) -> Option<&Channel> {
    let mut names: Vec<&String> = session.channels.keys().collect();
    names.sort();

    names
        .iter()
        .filter_map(|n| session.channels.get(*n))
        .find(|c| c.interpolate)
        .or_else(|| names.first().and_then(|n| session.channels.get(*n)))
}

#[cfg(test)]
// These tests assert on exact floating-point values produced by
// deterministic, exactly-representable arithmetic (small integer inputs,
// or values echoed straight back from a fixture) — exact `==` is the
// correct check here, not an approximation bug.
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn channel(interpolate: bool) -> Channel {
        Channel {
            name: "Test".into(),
            units: "u".into(),
            dec_pts: 2,
            interpolate,
            timecodes: vec![0.0, 10.0, 20.0, 30.0],
            values: vec![1.0, 2.0, 4.0, 8.0],
        }
    }

    #[test]
    fn interpolates_between_samples() {
        let c = channel(true);
        assert_eq!(value_at(&c, 0.0), Some(1.0));
        assert_eq!(value_at(&c, 10.0), Some(2.0));
        assert_eq!(value_at(&c, 15.0), Some(3.0)); // halfway between 2.0 and 4.0
        assert_eq!(value_at(&c, 25.0), Some(6.0)); // halfway between 4.0 and 8.0
        assert_eq!(value_at(&c, 30.0), Some(8.0));
    }

    #[test]
    fn holds_previous_value_when_not_interpolated() {
        let c = channel(false);
        assert_eq!(value_at(&c, 0.0), Some(1.0));
        assert_eq!(value_at(&c, 9.9), Some(1.0));
        assert_eq!(value_at(&c, 10.0), Some(2.0));
        assert_eq!(value_at(&c, 15.0), Some(2.0));
        assert_eq!(value_at(&c, 29.9), Some(4.0));
        assert_eq!(value_at(&c, 30.0), Some(8.0));
    }

    #[test]
    fn clamps_before_first_and_after_last_sample() {
        let c = channel(true);
        assert_eq!(value_at(&c, -100.0), Some(1.0));
        assert_eq!(value_at(&c, 1_000.0), Some(8.0));
    }

    #[test]
    fn empty_channel_returns_none() {
        let c = Channel {
            name: "Empty".into(),
            units: String::new(),
            dec_pts: 0,
            interpolate: true,
            timecodes: vec![],
            values: vec![],
        };
        assert_eq!(value_at(&c, 0.0), None);
    }

    #[test]
    fn build_plot_produces_move_and_line_commands() {
        let c = channel(true);
        let plot = build_plot(&c, 100.0, 100.0).unwrap();
        assert!(plot.commands.starts_with("M "));
        assert!(plot.commands.contains("L "));
        assert_eq!(plot.min_time, 0.0);
        assert_eq!(plot.max_time, 30.0);
    }

    #[test]
    fn build_plot_none_for_empty_channel() {
        let c = Channel {
            name: "Empty".into(),
            units: String::new(),
            dec_pts: 0,
            interpolate: true,
            timecodes: vec![],
            values: vec![],
        };
        assert!(build_plot(&c, 100.0, 100.0).is_none());
    }

    fn session_with(channels: Vec<Channel>) -> sde_core::Session {
        sde_core::Session {
            channels: channels.into_iter().map(|c| (c.name.clone(), c)).collect(),
            laps: vec![],
            metadata: std::collections::HashMap::new(),
            key_channel_map: sde_core::KeyChannelMap::default(),
            file_name: "test".into(),
        }
    }

    #[test]
    fn pick_default_channel_none_for_zero_channels() {
        let session = session_with(vec![]);
        assert!(pick_default_channel(&session).is_none());
    }

    #[test]
    fn pick_default_channel_falls_back_when_none_interpolate() {
        let a = Channel {
            name: "B".into(),
            units: "s".into(),
            dec_pts: 0,
            interpolate: false,
            timecodes: vec![0.0],
            values: vec![1.0],
        };
        let b = Channel {
            name: "A".into(),
            units: String::new(),
            dec_pts: 0,
            interpolate: false,
            timecodes: vec![0.0],
            values: vec![2.0],
        };
        let session = session_with(vec![a, b]);
        // No channel has interpolate == true, so falls back to the first
        // channel alphabetically by name ("A").
        let picked = pick_default_channel(&session).expect("has channels");
        assert_eq!(picked.name, "A");
    }

    #[test]
    fn pick_default_channel_prefers_first_interpolate_alphabetically() {
        let a = Channel {
            name: "A".into(),
            units: "s".into(), // interpolate == false
            dec_pts: 0,
            interpolate: false,
            timecodes: vec![0.0],
            values: vec![1.0],
        };
        let b = Channel {
            name: "B".into(),
            units: "km/h".into(),
            dec_pts: 0,
            interpolate: true,
            timecodes: vec![0.0],
            values: vec![2.0],
        };
        let session = session_with(vec![a, b]);
        let picked = pick_default_channel(&session).expect("has channels");
        assert_eq!(picked.name, "B");
    }

    /// End-to-end sanity check against the real synthetic fixture used by
    /// `sde-motec`/`sde-core`'s own tests, rather than only synthetic
    /// in-memory data.
    #[test]
    fn value_at_against_synthetic_fixture() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sde-formats/motec/tests/fixtures/synthetic.ld");
        let session = sde_core::Session::load_motec(&fixture)
            .unwrap_or_else(|e| panic!("failed to load {fixture:?}: {e}"));

        let channel = pick_default_channel(&session).expect("fixture has channels");
        assert!(!channel.timecodes.is_empty());

        // The value at the first sample's own timecode must equal that
        // sample exactly, regardless of interpolate mode.
        let t0 = channel.timecodes[0];
        let v0 = channel.values[0];
        assert_eq!(value_at(channel, t0), Some(v0));

        // Querying well past the end holds/returns the last sample.
        let t_last = *channel.timecodes.last().unwrap();
        let v_last = *channel.values.last().unwrap();
        assert_eq!(value_at(channel, t_last + 1_000_000.0), Some(v_last));
    }
}
