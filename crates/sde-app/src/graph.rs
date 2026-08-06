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

/// Fraction of the value range reserved as blank margin above and below
/// the plotted trace. Without this, a channel that sits flat at its own
/// min or max for long stretches (e.g. `BRAKE` at 0 outside a braking
/// zone, or `THROTTLE` at 100 on a straight) plots exactly on the dock's
/// top/bottom border pixel and becomes visually indistinguishable from
/// it — looking like "no data" there instead of a flat line.
const VALUE_MARGIN_FRACTION: f64 = 0.05;

/// The `(min, span)` to scale values against, given the raw `(min, max)`
/// across the samples being plotted: pads both ends by
/// [`VALUE_MARGIN_FRACTION`] of the raw span so a value at the true
/// min/max still plots a few pixels shy of the dock's border.
fn value_scale(min_val: f64, max_val: f64) -> (f64, f64) {
    let raw_span = (max_val - min_val).max(f64::EPSILON);
    let margin = raw_span * VALUE_MARGIN_FRACTION;
    (min_val - margin, raw_span + 2.0 * margin)
}

/// `channel`'s samples within `[start, end]`, plus a synthetic point at
/// each edge the real samples don't already reach exactly (via
/// [`value_at`]'s usual interpolate-or-hold lookup) — so the plotted line
/// always touches both edges of the window instead of stopping wherever
/// the nearest real sample happens to fall.
///
/// This matters most when zoomed into a narrow window (see
/// `graph::zoom_scroll`/`apply_zoom`): without it, a window that doesn't
/// happen to land exactly on sample boundaries would leave a visible gap
/// between the trace and the dock's edges, even though real data exists
/// right up to (and past) the boundary.
///
/// Returns an empty `Vec` if `channel` has no samples inside `[start,
/// end]` at all — deliberately doesn't synthesize a flat line across a
/// window the channel has no real overlap with (see `value_at`'s
/// clamp-to-nearest-sample behavior, which would otherwise make an
/// entirely-out-of-range window look like it has data).
fn windowed_samples(channel: &Channel, start: f64, end: f64) -> Vec<(f64, f64)> {
    let mut samples: Vec<(f64, f64)> = channel
        .timecodes
        .iter()
        .zip(channel.values.iter())
        .filter_map(|(&t, &v)| (t >= start && t <= end).then_some((t, v)))
        .collect();

    if samples.is_empty() {
        return samples;
    }

    if samples.first().is_some_and(|&(t, _)| t > start) {
        if let Some(v) = value_at(channel, start) {
            samples.insert(0, (start, v));
        }
    }
    if samples.last().is_some_and(|&(t, _)| t < end) {
        if let Some(v) = value_at(channel, end) {
            samples.push((end, v));
        }
    }

    samples
}

/// Cap on plotted points per pixel column: two lets a bucket's min and max
/// both survive (see [`decimate_for_display`]), which a single "one point
/// per column" reduction wouldn't — a brief brake spike or curb hit inside
/// a bucket would otherwise vanish into whichever sample happened to land
/// closest to the column's timestamp.
const MAX_POINTS_PER_PIXEL: usize = 2;

/// Reduce `samples` to roughly `MAX_POINTS_PER_PIXEL * view_width` points
/// by min/max decimation, when there are enough samples for it to matter.
///
/// A high-rate capture (ACR's `acr_telemetry` export runs ~333 Hz) zoomed
/// out to a multi-minute stage can carry tens of thousands of samples
/// while the SVG `Path` it's plotted into is `view_width` pixels wide
/// (1000, see `VIEW_WIDTH`) — every one of those samples becoming its own
/// `L x y ` command was both wasted `write!` work building the string and
/// wasted work for Slint tessellating/rendering a path with ~100x more
/// vertices than pixels. Bucketing by time (not by index) and keeping
/// each bucket's min and max preserves visual spikes a naive stride
/// (every Nth sample) would alias away.
///
/// A no-op below the threshold, so normal zoomed-in windows (already
/// close to one sample per pixel or fewer) pay nothing extra. Always
/// keeps `samples`' exact first/last point, since [`windowed_samples`]'s
/// synthesized boundary points (or the window's own real edge samples)
/// are what let the plotted line touch both edges of the window — a
/// bucket's min/max pick could otherwise silently drop them.
fn decimate_for_display(samples: Vec<(f64, f64)>, view_width: f64) -> Vec<(f64, f64)> {
    let buckets = view_width.max(1.0) as usize;
    let max_points = buckets * MAX_POINTS_PER_PIXEL;
    if samples.len() <= max_points {
        return samples;
    }

    let t0 = samples[0].0;
    let t1 = samples[samples.len() - 1].0;
    let bucket_width = ((t1 - t0) / buckets as f64).max(f64::EPSILON);

    let mut out = Vec::with_capacity(max_points + 2);
    let mut i = 0;
    while i < samples.len() {
        let bucket_end = t0 + (((samples[i].0 - t0) / bucket_width).floor() + 1.0) * bucket_width;
        let mut min_s = samples[i];
        let mut max_s = samples[i];
        let mut j = i;
        while j < samples.len() && samples[j].0 < bucket_end {
            if samples[j].1 < min_s.1 {
                min_s = samples[j];
            }
            if samples[j].1 > max_s.1 {
                max_s = samples[j];
            }
            j += 1;
        }
        // Keep time-ascending order within the bucket regardless of which
        // extreme (min or max) occurred first.
        if min_s.0 <= max_s.0 {
            out.push(min_s);
            out.push(max_s);
        } else {
            out.push(max_s);
            out.push(min_s);
        }
        i = j;
    }

    let first = samples[0];
    let last = samples[samples.len() - 1];
    if out.first() != Some(&first) {
        out.insert(0, first);
    }
    if out.last() != Some(&last) {
        out.push(last);
    }
    out
}

/// Build an SVG path (`M x y L x y L x y ...`) plotting `channel.values`
/// against `channel.timecodes`, normalized into a `view_width` x
/// `view_height` box. Y is flipped (SVG/Slint y grows downward, but a
/// larger channel value should plot higher on screen).
///
/// `range`, if given, restricts the plot to samples within
/// `(start_ms, end_ms)` inclusive (e.g. a selected lap's time window) and
/// both the time axis and the value (vertical) scaling are computed from
/// that window alone, not the whole channel. `None` plots the full
/// channel, as before.
///
/// Returns `None` if the channel has no samples, or if `range` excludes
/// every sample.
///
/// # Panics
///
/// Never panics in practice: the `.last().unwrap()` below is only
/// reached after the empty-channel check above, so `timecodes` is
/// guaranteed non-empty at that point.
#[must_use]
pub fn build_plot(
    channel: &Channel,
    view_width: f64,
    view_height: f64,
    range: Option<(f64, f64)>,
) -> Option<PlotData> {
    if channel.timecodes.is_empty() {
        return None;
    }

    let (min_time, max_time) =
        range.unwrap_or_else(|| (channel.timecodes[0], *channel.timecodes.last().unwrap()));

    let samples = windowed_samples(channel, min_time, max_time);
    if samples.is_empty() {
        return None;
    }
    let samples = decimate_for_display(samples, view_width);

    let time_span = (max_time - min_time).max(f64::EPSILON);
    let raw_min = samples
        .iter()
        .map(|&(_, v)| v)
        .fold(f64::INFINITY, f64::min);
    let raw_max = samples
        .iter()
        .map(|&(_, v)| v)
        .fold(f64::NEG_INFINITY, f64::max);
    let (min_val, val_span) = value_scale(raw_min, raw_max);

    let mut commands = String::new();
    for (i, &(t, v)) in samples.iter().enumerate() {
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

/// Which quantity a dock's graphs use for the horizontal axis. See the
/// "UI/UX direction: telemetry dashboard, reimagined" design note in
/// PROJECT_PLAN.md (principle 3): distance is the primary axis for
/// rally/hillclimb content — stage position is comparable across runs
/// even when pace (and therefore elapsed time at a given point) differs,
/// unlike time. Lap/zoom *selection* stays entirely time-based
/// (unchanged); only the plotted x-coordinate and cursor-position lookup
/// change.
///
/// `Distance` silently behaves like `Time` whenever no distance channel
/// is available (see `sde_core::KeyChannelMap::distance`) — callers don't
/// need to branch on availability themselves, see
/// [`build_lap_comparison_plot`].
///
/// One known limitation: a distance channel like iRacing's `LapDist`
/// resets to `0` at each lap start, so it's only monotonic *within* one
/// lap. This is exactly the shape [`build_lap_comparison_plot`] already
/// slices data into (one range per lap, each rebased independently), so
/// per-lap and per-comparison views are unaffected — the one case that
/// doesn't work is a single "All" range spanning *multiple* laps, where
/// the distance axis would sawtooth. Not guarded against here; out of
/// scope for the rally/hillclimb single-stage content this was built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisMode {
    #[default]
    Time,
    Distance,
}

/// The shared distance-axis span for a set of `(start_ms, end_ms)` time
/// ranges, analogous to [`shared_duration`] for the time axis: the
/// largest `distance_channel` delta across any one range, so the range
/// covering the most ground fills the full plot width. Always at least
/// [`f64::EPSILON`]`, so it's safe to divide by directly.
///
/// A range whose start/end both fall outside `distance_channel`'s data
/// contributes nothing (silently skipped, matching [`windowed_samples`]'s
/// "no overlap" handling elsewhere in this module).
#[must_use]
pub fn distance_axis_span(distance_channel: &Channel, ranges: &[(f64, f64)]) -> f64 {
    ranges
        .iter()
        .filter_map(|&(start, end)| {
            let d0 = value_at(distance_channel, start)?;
            let d1 = value_at(distance_channel, end)?;
            Some((d1 - d0).max(0.0))
        })
        .fold(0.0_f64, f64::max)
        .max(f64::EPSILON)
}

/// Inverse of [`value_at`] for a distance channel, restricted to one
/// `[start, end]` time range (typically one lap): the time within that
/// range at which `distance_channel` reaches `target_distance`. Windows
/// to `[start, end]` first (via [`windowed_samples`]) rather than
/// searching the whole channel, because a channel like iRacing's
/// `LapDist` resets to `0` at every lap start and so is only monotonic
/// *within* one lap — searching the unwindowed channel would feed
/// [`value_at_raw`]'s bracket search a non-sorted array and silently
/// return a bracket from the wrong lap. Returns `None` if `distance_channel`
/// has no samples overlapping `[start, end]`.
#[must_use]
pub fn time_at_distance(
    distance_channel: &Channel,
    start: f64,
    end: f64,
    target_distance: f64,
) -> Option<f64> {
    let samples = windowed_samples(distance_channel, start, end);
    if samples.is_empty() {
        return None;
    }
    let (timecodes, values): (Vec<f64>, Vec<f64>) = samples.into_iter().unzip();
    value_at_raw(&values, &timecodes, true, target_distance)
}

/// One overlaid trace within a [`MultiPlotData`] — a single lap's slice of
/// a channel, rebased so it starts at `t = 0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesPlot {
    pub commands: String,
}

/// Multiple laps' worth of one channel, overlaid for comparison: each
/// [`SeriesPlot`] is rebased to start at `t = 0` (lap-relative time) and
/// all series share the same time and value scaling, so the traces are
/// directly comparable. Built by [`build_lap_comparison_plot`].
#[derive(Debug, Clone, PartialEq)]
pub struct MultiPlotData {
    pub series: Vec<SeriesPlot>,
    pub view_width: f64,
    pub view_height: f64,
}

/// The shared time-axis span for a set of `(start_ms, end_ms)` lap
/// ranges: the longest range's duration (so the longest lap fills the
/// full plot width and shorter laps' traces simply end partway across).
/// Always at least `f64::EPSILON`, so it's safe to divide by directly.
#[must_use]
pub fn shared_duration(ranges: &[(f64, f64)]) -> f64 {
    ranges
        .iter()
        .map(|&(start, end)| end - start)
        .fold(0.0_f64, f64::max)
        .max(f64::EPSILON)
}

/// A timeline zoom window is never allowed to shrink below this fraction
/// of the full (unzoomed) range — avoids a degenerate/inverted or
/// division-by-near-zero window from runaway wheel-zooming.
pub const MIN_ZOOM_WIDTH: f64 = 0.01;

/// One mouse-wheel notch, in the scroll-delta units Slint reports. The
/// platform convention every desktop toolkit follows; a trackpad reports
/// fractions of it for a continuous swipe, which is exactly why
/// [`zoom_scroll`] scales by `delta / WHEEL_NOTCH` rather than treating
/// every event as one step.
pub const WHEEL_NOTCH: f64 = 120.0;

/// How much one full wheel notch zooms out (and its reciprocal, in).
/// ~11% per notch, unchanged from the original flat per-event factor —
/// what changed is that a partial notch now zooms proportionally less.
pub const ZOOM_PER_NOTCH: f64 = 1.0 / 0.9;

/// How far one full wheel notch pans, as a fraction of the visible
/// window.
pub const PAN_PER_NOTCH: f64 = 0.15;

/// How much accumulated scroll movement a gesture needs before it locks
/// to an axis (see [`ScrollGesture`]). One wheel notch (120) clears this
/// immediately, so a mouse wheel still zooms on its very first event; a
/// trackpad has to express a direction first.
pub const AXIS_LOCK_THRESHOLD: f64 = 12.0;

/// Accumulates scroll events into an axis-locked gesture.
///
/// Trackpads report a nonzero delta on *both* axes for nearly every
/// event, even during an intentional single-axis swipe, so applying both
/// would make a pan drift the zoom (and vice versa). Locking fixes that,
/// but deciding the lock from the very first event — which is often a
/// couple of jittery pixels — locks the wrong axis often enough to feel
/// broken: an intended horizontal pan whose first event happened to
/// favour vertical would zoom for the rest of the gesture instead.
///
/// So the decision waits until the gesture has accumulated
/// [`AXIS_LOCK_THRESHOLD`] of movement, and the accumulated delta is then
/// applied rather than discarded, so nothing is lost to the wait.
#[derive(Debug, Clone, Default)]
pub struct ScrollGesture {
    axis: Option<ScrollAxis>,
    pending_x: f64,
    pending_y: f64,
}

impl ScrollGesture {
    /// The axis this gesture is locked to, or `None` while still
    /// undecided.
    #[must_use]
    pub fn axis(&self) -> Option<ScrollAxis> {
        self.axis
    }

    /// End the current gesture, so the next event starts a fresh one.
    /// Callers drive this from a quiet-period timeout — scroll events
    /// carry no gesture start/end markers of their own.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feed one scroll event; returns the `(delta_x, delta_y)` to apply,
    /// with the off-axis component zeroed. `(0.0, 0.0)` while the gesture
    /// is still below the lock threshold.
    pub fn feed(&mut self, delta_x: f64, delta_y: f64) -> (f64, f64) {
        self.pending_x += delta_x;
        self.pending_y += delta_y;

        let axis = match self.axis {
            Some(axis) => axis,
            None => {
                if self.pending_x.abs().max(self.pending_y.abs()) < AXIS_LOCK_THRESHOLD {
                    return (0.0, 0.0);
                }
                let axis = dominant_scroll_axis(self.pending_x, self.pending_y);
                self.axis = Some(axis);
                axis
            }
        };

        let (x, y) = (self.pending_x, self.pending_y);
        self.pending_x = 0.0;
        self.pending_y = 0.0;
        match axis {
            ScrollAxis::Pan => (x, 0.0),
            ScrollAxis::Zoom => (0.0, y),
        }
    }
}

/// Fully zoomed out is represented as `(0.0, 1.0)`; anything tighter is
/// "zoomed in". Used to decide whether a zoom window is worth storing at
/// all (vs. just clearing it back to "no zoom").
#[must_use]
pub fn is_full_zoom(zoom: (f64, f64)) -> bool {
    zoom.0 <= 1e-9 && zoom.1 >= 1.0 - 1e-9
}

/// Which effect a scroll gesture drives: zooming (vertical scroll) or
/// panning (horizontal scroll). A gesture is locked to one axis for its
/// whole duration (see `main.rs`'s `AppState::scroll_gesture`) so
/// that trackpad diagonal jitter during an intended pan doesn't also
/// nudge the zoom level, and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxis {
    Zoom,
    Pan,
}

/// Which axis a single scroll event's `(delta_x, delta_y)` is dominantly
/// along — the axis a *new* gesture should lock to. Ties (equal
/// magnitude) favor zoom, matching a plain vertical mouse wheel (which
/// only ever reports `delta_y`, so `delta_x == delta_y == 0.0` should
/// still zoom rather than pan).
#[must_use]
pub fn dominant_scroll_axis(delta_x: f64, delta_y: f64) -> ScrollAxis {
    if delta_x.abs() > delta_y.abs() {
        ScrollAxis::Pan
    } else {
        ScrollAxis::Zoom
    }
}

/// Apply one wheel/trackpad scroll event to a `(start, end)` zoom window
/// (both fractions of the full, unzoomed time span — `(0.0, 1.0)` is
/// "fully zoomed out"), non-linear-video-editor style: vertical scroll
/// (`delta_y`) zooms in/out centered on `cursor_fraction` (the pointer's
/// position within the *current* window, so the point under the cursor
/// stays put as the window scales); horizontal scroll (`delta_x`, e.g.
/// shift-scroll or a trackpad swipe) pans the window instead. The result
/// is always a valid, clamped-to-`[0, 1]` window at least
/// [`MIN_ZOOM_WIDTH`] wide.
#[must_use]
pub fn zoom_scroll(
    current: (f64, f64),
    delta_x: f64,
    delta_y: f64,
    cursor_fraction: f64,
) -> (f64, f64) {
    let (mut start, mut end) = current;
    let cursor_fraction = cursor_fraction.clamp(0.0, 1.0);

    if delta_y != 0.0 {
        let width = end - start;
        // Positive `delta_y` is a scroll-down/away-from-user notch, which
        // by convention zooms out here; scroll-up/toward-user zooms in.
        //
        // Scaled by how far the wheel/trackpad actually moved, not a flat
        // per-event step: one mouse-wheel notch reports a full
        // `WHEEL_NOTCH` of delta and should zoom one step, but a trackpad
        // reports a stream of much smaller deltas for the same physical
        // gesture. Stepping a flat factor per *event* made a trackpad
        // swipe compound to an enormous zoom change (0.9^20 for a
        // twenty-event flick) while a wheel notch moved one step — the
        // same gesture meaning wildly different things per device.
        // Exponentiating keeps it continuous: two half-notches zoom
        // exactly as far as one whole one.
        let factor = ZOOM_PER_NOTCH.powf(delta_y / WHEEL_NOTCH);
        let cursor_point = start + cursor_fraction * width;
        let new_width = (width * factor).clamp(MIN_ZOOM_WIDTH, 1.0);
        start = cursor_point - cursor_fraction * new_width;
        end = start + new_width;
        (start, end) = clamp_window(start, end);
    }

    if delta_x != 0.0 {
        let width = end - start;
        // Same per-notch treatment as the zoom above, for the same
        // device-consistency reason. Numerically identical to the previous
        // `delta_x / 800.0` at one full notch; the difference is only that
        // a trackpad's fractional deltas now pan proportionally.
        let shift = PAN_PER_NOTCH * (delta_x / WHEEL_NOTCH) * width;
        (start, end) = clamp_window(start + shift, end + shift);
    }

    (start, end)
}

/// Clamp a `(start, end)` window into `[0.0, 1.0]` without changing its
/// width, sliding it inward if it would otherwise run off either edge
/// (rather than just truncating, which would silently narrow it).
fn clamp_window(start: f64, end: f64) -> (f64, f64) {
    let width = (end - start).max(MIN_ZOOM_WIDTH);
    let (mut start, mut end) = (start, start + width);
    if start < 0.0 {
        end -= start;
        start = 0.0;
    }
    if end > 1.0 {
        start -= end - 1.0;
        end = 1.0;
    }
    (start.max(0.0), end.min(1.0))
}

/// Narrow a set of `(start_ms, end_ms)` ranges (see [`shared_duration`])
/// to a `zoom` sub-window (fractions of the full shared duration, as
/// produced by [`zoom_scroll`]; `None` means fully zoomed out). Returns
/// the narrowed ranges plus the time span they should now be plotted
/// against, both ready to hand to [`build_lap_comparison_plot`]/
/// [`build_plot`]'s `range`/`time_span` parameters.
///
/// Each original range is intersected with the zoom window independently
/// (rather than the zoom window being one absolute time slice), so
/// zooming still means the same *lap-relative* moment across every
/// compared lap, matching how [`build_lap_comparison_plot`] rebases each
/// range to start at `t = 0`.
#[must_use]
pub fn apply_zoom(ranges: &[(f64, f64)], zoom: Option<(f64, f64)>) -> (Vec<(f64, f64)>, f64) {
    let full_span = shared_duration(ranges);
    let Some((z0, z1)) = zoom else {
        return (ranges.to_vec(), full_span);
    };

    let narrowed: Vec<(f64, f64)> = ranges
        .iter()
        .map(|&(start, end)| {
            let lo = (start + z0 * full_span).clamp(start, end);
            let hi = (start + z1 * full_span).clamp(lo, end);
            (lo, hi)
        })
        .collect();

    let zoomed_span = ((z1 - z0) * full_span).max(f64::EPSILON);
    (narrowed, zoomed_span)
}

/// Build one overlaid trace per `(start_ms, end_ms)` range in `ranges`
/// (typically one per lap being compared, or a single range for the
/// normal "All"/one-lap case), each rebased to start at lap-relative
/// `t = 0`. All traces share `time_span` (see [`shared_duration`]) for
/// the x-axis and a value scale computed across every sample in every
/// range, so the overlay is a fair visual comparison rather than each
/// lap being independently auto-scaled.
///
/// A range that contains no samples for `channel` is silently omitted
/// from `series` (e.g. a lap the channel wasn't recording during).
/// Returns `None` if `ranges` is empty or every range excludes all of
/// the channel's samples.
///
/// `axis`/`distance_channel` select the horizontal axis — see
/// [`AxisMode`]. Passing `AxisMode::Distance` with `distance_channel:
/// None` behaves exactly like `AxisMode::Time` (transparent fallback);
/// callers don't need to check availability themselves.
#[must_use]
pub fn build_lap_comparison_plot(
    channel: &Channel,
    view_width: f64,
    view_height: f64,
    ranges: &[(f64, f64)],
    time_span: f64,
    axis: AxisMode,
    distance_channel: Option<&Channel>,
) -> Option<MultiPlotData> {
    if ranges.is_empty() {
        return None;
    }

    let distance_channel = match axis {
        AxisMode::Distance => distance_channel,
        AxisMode::Time => None,
    };

    let per_range_samples: Vec<Vec<(f64, f64)>> = ranges
        .iter()
        .map(|&(start, end)| {
            let raw = decimate_for_display(windowed_samples(channel, start, end), view_width);
            match distance_channel {
                Some(dist_ch) => {
                    let start_dist = value_at(dist_ch, start).unwrap_or(0.0);
                    raw.into_iter()
                        .filter_map(|(t, v)| value_at(dist_ch, t).map(|d| (d - start_dist, v)))
                        .collect()
                }
                None => raw.into_iter().map(|(t, v)| (t - start, v)).collect(),
            }
        })
        .collect();

    if per_range_samples.iter().all(Vec::is_empty) {
        return None;
    }

    let axis_span = match distance_channel {
        Some(dist_ch) => distance_axis_span(dist_ch, ranges),
        None => time_span.max(f64::EPSILON),
    };
    let (raw_min, raw_max) = per_range_samples
        .iter()
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &(_, v)| {
            (lo.min(v), hi.max(v))
        });
    let (min_val, val_span) = value_scale(raw_min, raw_max);

    let series = per_range_samples
        .into_iter()
        .filter(|samples| !samples.is_empty())
        .map(|samples| {
            let mut commands = String::new();
            for (i, &(t, v)) in samples.iter().enumerate() {
                let x = t / axis_span * view_width;
                let y = view_height - (v - min_val) / val_span * view_height;
                let _ = if i == 0 {
                    write!(commands, "M {x} {y} ")
                } else {
                    write!(commands, "L {x} {y} ")
                };
            }
            SeriesPlot { commands }
        })
        .collect();

    Some(MultiPlotData {
        series,
        view_width,
        view_height,
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

/// Whether `channel` carries any real signal, as opposed to a column a
/// format/sim simply never populates. A capability the exporting sim
/// doesn't support (e.g. SimHub's `CarPosX_raw` when the active title
/// doesn't expose world-space car position) still shows up as a full-width
/// column, just one that's either entirely empty (see the `shtep` TSV
/// parser's empty-field handling) or filled with the same `0.0` sentinel
/// on every row — indistinguishable, from a UI's point of view, from
/// "no data" and not worth a dock or a distance-axis mode built on it.
///
/// Deliberately checks for *all-zero*, not merely *flat* — a channel that
/// legitimately sits at one nonzero value for a whole run (e.g. `AirTemp_C`
/// on a session with no weather change) is real data and shouldn't be
/// suppressed just because it isn't varying right now.
#[must_use]
pub fn channel_has_data(channel: &Channel) -> bool {
    !channel.values.is_empty() && channel.values.iter().any(|&v| v != 0.0)
}

/// Pick which channel to plot by default when a file is first loaded (the
/// user can then pick a different one via the channel search/list): the
/// first `interpolate == true` channel in alphabetical order by name, or
/// if none qualify, simply the first channel alphabetically.
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

/// Candidate names for each "role" in the default worksheet, checked in
/// order, one list per role — the same channel means different things by
/// name across formats (e.g. speed is `"Ground Speed"` in MoTeC,
/// `"Speed"` in IBT, `"Speed_kmh"` in `shtep`, `"speed"` in RSF/NGP's own
/// dotted-field-name `.ld`/`.tsv` exports), so this can't be a single flat
/// name list the way [`sde_core::KeyChannelMap`]'s `distance` lookup is.
/// One dock is created per role that has *any* match in the loaded
/// session; a role with no match is simply skipped, not left as an empty
/// dock. Order here is the on-screen order, top to bottom.
///
/// The RSF/NGP names (`speed`, `engineRotation`, `throttle`, `brake`,
/// `gear`, `steering`) are confirmed against a real capture's channel list
/// (`.sample-data/RBR/…/Run1/motec/*.ld`, see PROJECT_PLAN.md's "RSF
/// real-capture validation" section) — before these were added, a loaded
/// RSF session matched none of the other formats' names and fell all the
/// way through to [`pick_default_channel`]'s single-channel fallback,
/// which is the "only shows one panel" bug this list closes for that
/// format.
const DEFAULT_DOCK_ROLES: &[&[&str]] = &[
    &["Ground Speed", "Speed", "Speed_kmh", "speed"],
    &["RPM", "Engine0_RPM", "engineRotation"],
    &[
        "Throttle",
        "THROTTLE",
        "Throttle_pct",
        "ThrottleRaw",
        "throttle",
    ],
    &["Brake", "BRAKE", "Brake_pct", "BrakeRaw", "brake"],
    &["Gear", "GEAR", "gear"],
    &[
        "SteeringWheelAngle",
        "STEERANGLE",
        "SteerAngle_deg",
        "steering",
    ],
];

/// A sensible default worksheet for a freshly loaded session — one dock
/// per [`DEFAULT_DOCK_ROLES`] entry the session actually has a channel
/// for — rather than starting from either a single bare channel or a
/// fully empty worksheet. Matches the "glance-able default view" every
/// established telemetry tool (MoTeC i2, Pi Toolbox) opens with, instead
/// of making every session start from a blank canvas.
///
/// Falls back to [`pick_default_channel`]'s single-channel pick (as one
/// dock) if *none* of the roles match anything — an unfamiliar channel
/// naming scheme shouldn't leave the worksheet empty either.
#[must_use]
pub fn default_dock_channels(session: &sde_core::Session) -> Vec<Vec<String>> {
    let docks: Vec<Vec<String>> = DEFAULT_DOCK_ROLES
        .iter()
        .filter_map(|candidates| {
            candidates
                .iter()
                .find(|name| session.channels.contains_key(**name))
                .map(|name| vec![(*name).to_string()])
        })
        .collect();

    if docks.is_empty() {
        pick_default_channel(session)
            .into_iter()
            .map(|c| vec![c.name.clone()])
            .collect()
    } else {
        docks
    }
}

/// How the worksheet is arranging its docks. Mirrors `app.slint`'s
/// `layout-mode` integer, which is what [`DockLayout::from_index`]
/// converts — the markup keeps it as an int because it's plain UI state
/// set by three toggle chips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockLayout {
    Stacked,
    SideBySide,
    Grid,
}

impl DockLayout {
    /// Anything unrecognized reads as [`Stacked`](DockLayout::Stacked),
    /// which is the worksheet's default.
    #[must_use]
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::SideBySide,
            2 => Self::Grid,
            _ => Self::Stacked,
        }
    }
}

/// Columns in the "Grid" layout mode. Must match the `GRID_COLUMNS` that
/// `main.rs` precomputes each dock's `grid-row`/`grid-col` against.
pub const DOCK_GRID_COLUMNS: usize = 2;

/// Which dock index a drag that started on dock `source` and has moved
/// `(dx, dy)` pixels is currently over, or `None` if it's over the source
/// itself, over empty space, or outside the worksheet.
///
/// Derived from the layout's own regular geometry rather than from hit
/// testing in markup: every layout mode places docks on a fixed pitch, so
/// the dragged panel's own `(cell_width, cell_height)` is enough to turn a
/// pixel delta into a whole number of cells moved. That keeps the
/// arithmetic here — pure and testable — instead of in `app.slint`, which
/// is the same split the rest of the layout math already uses (see
/// `DockData::grid-row`/`grid-col`).
///
/// Rounding to the nearest cell also gives the click/drag distinction for
/// free: a drag too short to reach a neighbouring cell rounds to zero
/// cells moved, lands back on `source`, and yields `None`.
///
/// Deliberately says nothing about *what* the drop does — the same target
/// index feeds either [`reorder_docks`] (plain drag) or [`merge_docks`]
/// (Ctrl held), which is exactly why a modifier can distinguish the two
/// without needing a second, differently-shaped drop geometry.
#[must_use]
pub fn drag_drop_target(
    layout: DockLayout,
    source: usize,
    dock_count: usize,
    dx: f64,
    dy: f64,
    cell_width: f64,
    cell_height: f64,
) -> Option<usize> {
    if source >= dock_count || cell_width <= 0.0 || cell_height <= 0.0 {
        return None;
    }

    // The delta is bounded by the window size and the cell by the dock
    // size, so the quotient can't approach i64's range.
    #[allow(clippy::cast_possible_truncation)]
    let steps = |delta: f64, cell: f64| (delta / cell).round() as i64;

    let source_index = i64::try_from(source).ok()?;
    let count = i64::try_from(dock_count).ok()?;
    let columns = i64::try_from(DOCK_GRID_COLUMNS).ok()?;

    let target = match layout {
        DockLayout::Stacked => source_index + steps(dy, cell_height),
        DockLayout::SideBySide => source_index + steps(dx, cell_width),
        DockLayout::Grid => {
            // A grid drag has to track both axes, and a sideways drag off
            // either edge is *not* a wrap onto the neighbouring row — the
            // user dragged into empty space, so nothing is targeted.
            let column = source_index % columns + steps(dx, cell_width);
            let row = source_index / columns + steps(dy, cell_height);
            if column < 0 || column >= columns || row < 0 {
                return None;
            }
            row * columns + column
        }
    };

    if target == source_index || target < 0 || target >= count {
        return None;
    }
    usize::try_from(target).ok()
}

/// Move dock `source` to position `target`, shifting the docks in between
/// along, and report whether anything changed. The plain (unmodified)
/// header drag — see [`merge_docks`] for the Ctrl-held variant.
///
/// "Move to index" semantics: afterwards the moved dock *is* at `target`,
/// whichever direction it came from.
pub fn reorder_docks(docks: &mut Vec<Vec<String>>, source: usize, target: usize) -> bool {
    if source == target || source >= docks.len() || target >= docks.len() {
        return false;
    }

    let moved = docks.remove(source);
    docks.insert(target, moved);
    true
}

/// Fold dock `source`'s channels into dock `target` (an overlay group)
/// and drop the now-empty source dock, reporting whether anything
/// changed. The Ctrl-held header drag — see [`reorder_docks`] for the
/// plain one.
///
/// Channels `target` already plots are skipped rather than duplicated —
/// dropping a dock onto one that overlaps it shouldn't plot the same
/// trace twice — and the merged dock keeps `target`'s channel order with
/// the new ones appended, so the drop target stays recognizably itself.
pub fn merge_docks(docks: &mut Vec<Vec<String>>, source: usize, target: usize) -> bool {
    if source == target || source >= docks.len() || target >= docks.len() {
        return false;
    }

    let moved = docks.remove(source);
    // Removing the source shifts every later dock down one.
    let target = if target > source { target - 1 } else { target };
    for name in moved {
        if !docks[target].contains(&name) {
            docks[target].push(name);
        }
    }
    true
}

/// All channel names in `session`, sorted alphabetically — the unfiltered
/// list backing the channel search/picker.
#[must_use]
pub fn channel_names(session: &sde_core::Session) -> Vec<String> {
    let mut names: Vec<String> = session.channels.keys().cloned().collect();
    names.sort();
    names
}

/// Case-insensitive substring filter over `names`, for the channel search
/// box. An empty `query` matches everything.
#[must_use]
pub fn filter_channel_names(names: &[String], query: &str) -> Vec<String> {
    let query = query.to_lowercase();
    names
        .iter()
        .filter(|n| query.is_empty() || n.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

/// Display labels for the lap/run picker: `"All"` (the whole session,
/// index `0`) followed by one label per lap in session order (index
/// `lap_num + 1`), so `lap_labels(session)[i]` and `session.laps[i - 1]`
/// always correspond for `i >= 1`.
///
/// A session with exactly one lap — a single continuous stage/hillclimb
/// run, not a lap-timed circuit (see PROJECT_PLAN.md's "UI/UX direction"
/// design note, principle 4: "Run", not "Lap", as the primary unit) —
/// gets one label, `"Full Run (…s)"`, instead of the redundant `["All",
/// "Lap 1 (…s)"]` pair a lap-assuming picker would show for the exact
/// same range twice. `index 0` still means "the whole session" either
/// way, so callers indexing into `session.laps` (`i - 1` for `i >= 1`)
/// need no special-casing — a single-lap session's index `0` already
/// spans that one lap.
#[must_use]
pub fn lap_labels(session: &sde_core::Session) -> Vec<String> {
    if let [lap] = session.laps.as_slice() {
        let dur_s = (lap.end_time - lap.start_time) / 1000.0;
        return vec![format!("Full Run ({dur_s:.1}s)")];
    }

    let mut labels = vec!["All".to_string()];
    labels.extend(session.laps.iter().map(|lap| {
        let dur_s = (lap.end_time - lap.start_time) / 1000.0;
        format!("Lap {} ({dur_s:.1}s)", lap.num + 1)
    }));
    labels
}

/// The whole-session time window `(0.0, end_time)`, i.e. the same range
/// `"All"` in the lap picker should plot. Derived from the last lap's
/// `end_time` rather than re-scanning every channel, since lap-splitting
/// (see `sde-core`'s `laps_from_beacon`/`laps_from_markers`) always
/// appends the session's overall end time as the final lap boundary, and
/// a `Session` always has at least one lap (the whole-session fallback
/// when no beacon/marker data is present). Returns `(0.0, 0.0)` only for
/// a degenerate session with zero channels and thus zero laps.
#[must_use]
pub fn session_time_range(session: &sde_core::Session) -> (f64, f64) {
    session
        .laps
        .last()
        .map_or((0.0, 0.0), |lap| (0.0, lap.end_time))
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
        let plot = build_plot(&c, 100.0, 100.0, None).unwrap();
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
        assert!(build_plot(&c, 100.0, 100.0, None).is_none());
    }

    #[test]
    fn build_plot_windowed_to_range_rescales_axes() {
        let c = channel(true);
        // Window to the middle two samples only: time/value scaling
        // should derive from just that window, not the whole channel.
        let plot = build_plot(&c, 100.0, 100.0, Some((10.0, 20.0))).unwrap();
        assert_eq!(plot.min_time, 10.0);
        assert_eq!(plot.max_time, 20.0);
        // Values 2.0..4.0 span the window; first sample (v=2.0, the min)
        // should plot near the bottom, but not exactly at y = view_height
        // (see `value_scale`'s margin, which keeps a flat-at-min/max trace
        // off the dock's border).
        assert!(plot.commands.starts_with("M 0 "));
        assert!(!plot.commands.starts_with("M 0 100 "));
    }

    #[test]
    fn decimate_for_display_is_a_no_op_below_the_point_budget() {
        let samples: Vec<(f64, f64)> = (0..500).map(|i| (f64::from(i), f64::from(i))).collect();
        let view_width = 1000.0; // budget 2000 points, well above 500
        assert_eq!(decimate_for_display(samples.clone(), view_width), samples);
    }

    #[test]
    fn decimate_for_display_caps_point_count_for_a_dense_channel() {
        // 100k samples over a 1000px-wide view (budget 2000 points) is the
        // exact shape of a full-zoom view of a high-rate ACR capture.
        let samples: Vec<(f64, f64)> = (0..100_000)
            .map(|i| (f64::from(i), (f64::from(i) * 0.01).sin()))
            .collect();
        let out = decimate_for_display(samples, 1000.0);
        assert!(
            out.len() <= 2002,
            "expected roughly 2000 points, got {}",
            out.len()
        );
        assert!(out.len() > 100, "shouldn't collapse to almost nothing");
    }

    #[test]
    fn decimate_for_display_preserves_a_spike_inside_one_bucket() {
        // A single-sample spike buried in an otherwise flat run must
        // survive decimation — min/max-per-bucket exists precisely so a
        // brake-pressure or curb-strike transient isn't averaged away.
        let mut samples: Vec<(f64, f64)> = (0..10_000).map(|i| (f64::from(i), 0.0)).collect();
        samples[4_242].1 = 999.0;
        let out = decimate_for_display(samples, 1000.0);
        assert!(
            out.iter().any(|&(_, v)| v == 999.0),
            "spike value was lost during decimation"
        );
    }

    #[test]
    fn decimate_for_display_keeps_exact_first_and_last_points() {
        let mut samples: Vec<(f64, f64)> = (0..50_000).map(|i| (f64::from(i), 0.0)).collect();
        samples[0] = (0.0, 1.5);
        let last = samples.len() - 1;
        samples[last] = (49_999.0, -2.5);
        let out = decimate_for_display(samples.clone(), 1000.0);
        assert_eq!(out.first(), Some(&samples[0]));
        assert_eq!(out.last(), Some(&samples[last]));
    }

    #[test]
    fn decimate_for_display_keeps_time_ascending_order() {
        let samples: Vec<(f64, f64)> = (0..50_000)
            .map(|i| (f64::from(i), (f64::from(i) * 0.037).cos()))
            .collect();
        let out = decimate_for_display(samples, 1000.0);
        assert!(
            out.windows(2).all(|w| w[0].0 <= w[1].0),
            "decimated output must stay time-ascending for value_at-style bracket search"
        );
    }

    #[test]
    fn windowed_samples_synthesizes_boundary_points() {
        let c = channel(true); // timecodes 0/10/20/30, values 1/2/4/8
                               // (5, 25) doesn't land on any real sample, so both edges should
                               // be filled in via interpolation.
        let samples = windowed_samples(&c, 5.0, 25.0);
        assert_eq!(samples.first(), Some(&(5.0, 1.5))); // halfway between 1.0@0 and 2.0@10
        assert_eq!(samples.last(), Some(&(25.0, 6.0))); // halfway between 4.0@20 and 8.0@30
    }

    #[test]
    fn windowed_samples_empty_when_channel_has_no_overlap() {
        let c = channel(true); // covers 0..30
        assert!(windowed_samples(&c, 1000.0, 2000.0).is_empty());
    }

    #[test]
    fn build_plot_extends_to_the_window_edges_when_samples_dont_land_exactly_there() {
        let c = channel(true);
        // A window that doesn't align with any real sample boundary
        // (previously the trace would stop wherever the nearest sample
        // fell, leaving a visible gap at each edge when zoomed in).
        let plot = build_plot(&c, 100.0, 100.0, Some((5.0, 25.0))).unwrap();
        assert!(plot.commands.starts_with("M 0 "));
        let last_command = plot.commands.trim_end().rsplit("L ").next().unwrap();
        let x: f64 = last_command
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!((x - 100.0).abs() < 1e-9);
    }

    #[test]
    fn value_scale_pads_both_ends_of_the_raw_range() {
        let (min, span) = value_scale(0.0, 10.0);
        assert!(min < 0.0);
        assert!(span > 10.0);
        // Symmetric padding: the raw max should land strictly inside
        // (min, min + span), not right at either edge.
        assert!(10.0 > min && 10.0 < min + span);
    }

    #[test]
    fn value_scale_never_divides_by_zero_for_a_flat_signal() {
        let (_, span) = value_scale(5.0, 5.0);
        assert!(span > 0.0);
    }

    #[test]
    fn build_plot_none_when_range_excludes_all_samples() {
        let c = channel(true);
        assert!(build_plot(&c, 100.0, 100.0, Some((100.0, 200.0))).is_none());
    }

    #[test]
    fn filter_channel_names_is_case_insensitive_substring() {
        let names = vec![
            "Ground Speed".to_string(),
            "RPM".to_string(),
            "Gear".to_string(),
        ];
        assert_eq!(filter_channel_names(&names, ""), names);
        assert_eq!(filter_channel_names(&names, "gr"), vec!["Ground Speed"]);
        assert_eq!(filter_channel_names(&names, "ea"), vec!["Gear"]);
        assert_eq!(filter_channel_names(&names, "RPM"), vec!["RPM"]);
        assert!(filter_channel_names(&names, "zzz").is_empty());
    }

    #[test]
    fn lap_labels_prefixes_all_and_numbers_laps_from_one() {
        let session = sde_core::Session {
            channels: std::collections::HashMap::new(),
            laps: vec![
                sde_core::Lap {
                    num: 0,
                    start_time: 0.0,
                    end_time: 30_000.0,
                },
                sde_core::Lap {
                    num: 1,
                    start_time: 30_000.0,
                    end_time: 65_500.0,
                },
            ],
            metadata: std::collections::HashMap::new(),
            key_channel_map: sde_core::KeyChannelMap::default(),
            file_name: "test".into(),
            time_penalties: Vec::new(),
        };
        let labels = lap_labels(&session);
        assert_eq!(labels, vec!["All", "Lap 1 (30.0s)", "Lap 2 (35.5s)"]);
    }

    #[test]
    fn lap_labels_single_lap_session_is_full_run_not_all_plus_lap_one() {
        // A single continuous stage/hillclimb run (e.g. RBR/RSF, iRacing
        // hillclimb) has exactly one "lap" spanning the whole session —
        // showing both "All" and "Lap 1" for the identical range would be
        // redundant and assumes a lap-timed circuit that isn't there.
        let session = sde_core::Session {
            channels: std::collections::HashMap::new(),
            laps: vec![sde_core::Lap {
                num: 0,
                start_time: 0.0,
                end_time: 466_000.0,
            }],
            metadata: std::collections::HashMap::new(),
            key_channel_map: sde_core::KeyChannelMap::default(),
            file_name: "test".into(),
            time_penalties: Vec::new(),
        };
        let labels = lap_labels(&session);
        assert_eq!(labels, vec!["Full Run (466.0s)"]);
    }

    #[test]
    fn session_time_range_is_zero_to_last_lap_end() {
        let session = sde_core::Session {
            channels: std::collections::HashMap::new(),
            laps: vec![
                sde_core::Lap {
                    num: 0,
                    start_time: 0.0,
                    end_time: 30_000.0,
                },
                sde_core::Lap {
                    num: 1,
                    start_time: 30_000.0,
                    end_time: 65_500.0,
                },
            ],
            metadata: std::collections::HashMap::new(),
            key_channel_map: sde_core::KeyChannelMap::default(),
            file_name: "test".into(),
            time_penalties: Vec::new(),
        };
        assert_eq!(session_time_range(&session), (0.0, 65_500.0));
    }

    #[test]
    fn shared_duration_is_the_longest_range() {
        assert_eq!(shared_duration(&[(0.0, 10.0), (5.0, 25.0)]), 20.0);
        assert_eq!(shared_duration(&[]), f64::EPSILON);
    }

    #[test]
    fn is_full_zoom_detects_the_unzoomed_window() {
        assert!(is_full_zoom((0.0, 1.0)));
        assert!(!is_full_zoom((0.1, 1.0)));
        assert!(!is_full_zoom((0.0, 0.9)));
    }

    #[test]
    fn dominant_scroll_axis_picks_the_larger_magnitude_delta() {
        assert_eq!(dominant_scroll_axis(1.0, 5.0), ScrollAxis::Zoom);
        assert_eq!(dominant_scroll_axis(5.0, 1.0), ScrollAxis::Pan);
        assert_eq!(dominant_scroll_axis(-5.0, 1.0), ScrollAxis::Pan); // sign shouldn't matter
        assert_eq!(dominant_scroll_axis(1.0, -5.0), ScrollAxis::Zoom);
    }

    #[test]
    fn dominant_scroll_axis_ties_favor_zoom() {
        // Covers a plain vertical mouse wheel, which only ever reports
        // `delta_y` — `(0.0, 0.0)` (no scroll at all) shouldn't default
        // to locking a gesture onto "pan".
        assert_eq!(dominant_scroll_axis(0.0, 0.0), ScrollAxis::Zoom);
        assert_eq!(dominant_scroll_axis(3.0, 3.0), ScrollAxis::Zoom);
    }

    #[test]
    fn zoom_scroll_in_narrows_the_window_around_the_cursor() {
        let zoomed = zoom_scroll((0.0, 1.0), 0.0, -1.0, 0.5);
        assert!(zoomed.1 - zoomed.0 < 1.0);
        // Zooming in centered on the middle should stay roughly centered.
        let mid = f64::midpoint(zoomed.0, zoomed.1);
        assert!((mid - 0.5).abs() < 1e-6);
    }

    #[test]
    fn zoom_scroll_in_keeps_the_point_under_the_cursor_fixed() {
        // Zoom in centered on a point 3/4 of the way across a window
        // that's already zoomed to [0.2, 0.6]: the absolute point under
        // the cursor (0.2 + 0.75 * 0.4 == 0.5) should land at the same
        // fraction of the *new* (narrower) window.
        let before = (0.2, 0.6);
        let cursor_fraction = 0.75;
        let after = zoom_scroll(before, 0.0, -1.0, cursor_fraction);
        let point_before = before.0 + cursor_fraction * (before.1 - before.0);
        let point_after = after.0 + cursor_fraction * (after.1 - after.0);
        assert!((point_before - point_after).abs() < 1e-9);
    }

    #[test]
    fn zoom_scroll_out_widens_and_clamps_to_full_range() {
        let zoomed = zoom_scroll((0.4, 0.6), 0.0, WHEEL_NOTCH, 0.5);
        assert!(zoomed.1 - zoomed.0 > 0.2);
        let very_zoomed_out = zoom_scroll((0.0, 1.0), 0.0, WHEEL_NOTCH, 0.5);
        assert_eq!(very_zoomed_out, (0.0, 1.0));
    }

    #[test]
    fn zoom_scroll_never_narrows_past_the_minimum_width() {
        // Full notches, not a delta of 1.0: now that a step is
        // proportional to the delta, 200 one-unit events barely zoom at
        // all and wouldn't reach the clamp this test exists to check.
        let mut window = (0.0, 1.0);
        for _ in 0..200 {
            window = zoom_scroll(window, 0.0, -WHEEL_NOTCH, 0.5);
        }
        assert!(window.1 - window.0 >= MIN_ZOOM_WIDTH - 1e-9);
    }

    #[test]
    fn zoom_scroll_pan_shifts_without_changing_width() {
        let before = (0.4, 0.6);
        let after = zoom_scroll(before, 50.0, 0.0, 0.5);
        assert!((after.1 - after.0 - (before.1 - before.0)).abs() < 1e-9);
        assert_ne!(before, after);
    }

    #[test]
    fn zoom_scroll_pan_stops_at_the_edges_instead_of_truncating() {
        let after = zoom_scroll((0.0, 0.2), -10_000.0, 0.0, 0.5);
        assert!((after.0 - 0.0).abs() < 1e-6 && (after.1 - 0.2).abs() < 1e-6); // left edge
        let after = zoom_scroll((0.8, 1.0), 10_000.0, 0.0, 0.5);
        assert!((after.0 - 0.8).abs() < 1e-6 && (after.1 - 1.0).abs() < 1e-6); // right edge
    }

    #[test]
    fn zoom_step_is_proportional_to_how_far_the_wheel_moved() {
        // The bug this guards: a flat per-event factor meant a trackpad,
        // which reports many small deltas for one physical swipe, zoomed
        // vastly further than a mouse wheel doing "the same" gesture.
        let one_notch = zoom_scroll((0.0, 1.0), 0.0, -WHEEL_NOTCH, 0.5);
        let mut ten_tenths = (0.0, 1.0);
        for _ in 0..10 {
            ten_tenths = zoom_scroll(ten_tenths, 0.0, -WHEEL_NOTCH / 10.0, 0.5);
        }
        let (a, b) = (one_notch.1 - one_notch.0, ten_tenths.1 - ten_tenths.0);
        assert!(
            (a - b).abs() < 1e-9,
            "one notch ({a}) should zoom as far as ten tenths of one ({b})"
        );
        // And one notch is the ~11% step it always was.
        assert!(
            (a - 0.9).abs() < 1e-9,
            "one notch should zoom in ~10%, got {a}"
        );
    }

    #[test]
    fn pan_step_is_proportional_to_how_far_the_wheel_moved() {
        let before = (0.4, 0.6);
        let one_notch = zoom_scroll(before, WHEEL_NOTCH, 0.0, 0.5);
        let mut ten_tenths = before;
        for _ in 0..10 {
            ten_tenths = zoom_scroll(ten_tenths, WHEEL_NOTCH / 10.0, 0.0, 0.5);
        }
        assert!((one_notch.0 - ten_tenths.0).abs() < 1e-9);
    }

    #[test]
    fn a_gesture_waits_for_a_clear_direction_before_locking_an_axis() {
        let mut gesture = ScrollGesture::default();
        // Trackpad jitter: tiny, and vertical-dominant at first, but the
        // user is swiping horizontally. Nothing is applied yet...
        assert_eq!(gesture.feed(1.0, 2.0), (0.0, 0.0));
        assert_eq!(gesture.axis(), None);
        // ...and once the direction is clear, the gesture locks to pan,
        // *including* the movement accumulated during the wait.
        let (dx, dy) = gesture.feed(11.0, 1.0);
        assert_eq!(gesture.axis(), Some(ScrollAxis::Pan));
        assert_eq!((dx, dy), (12.0, 0.0));
    }

    #[test]
    fn a_locked_gesture_ignores_off_axis_jitter_for_its_whole_duration() {
        let mut gesture = ScrollGesture::default();
        gesture.feed(0.0, -WHEEL_NOTCH); // one wheel notch: locks to zoom at once
        assert_eq!(gesture.axis(), Some(ScrollAxis::Zoom));
        // A later event that's *mostly horizontal* still can't pan.
        assert_eq!(gesture.feed(50.0, -2.0), (0.0, -2.0));
    }

    #[test]
    fn a_wheel_notch_locks_and_applies_immediately() {
        // A mouse wheel reports one big delta per notch; waiting for a
        // second event before acting would feel like a dropped input.
        let mut gesture = ScrollGesture::default();
        assert_eq!(gesture.feed(0.0, WHEEL_NOTCH), (0.0, WHEEL_NOTCH));
    }

    #[test]
    fn resetting_a_gesture_lets_the_next_one_pick_a_different_axis() {
        let mut gesture = ScrollGesture::default();
        gesture.feed(0.0, WHEEL_NOTCH);
        assert_eq!(gesture.axis(), Some(ScrollAxis::Zoom));
        gesture.reset();
        assert_eq!(gesture.axis(), None);
        gesture.feed(WHEEL_NOTCH, 0.0);
        assert_eq!(gesture.axis(), Some(ScrollAxis::Pan));
    }

    #[test]
    fn accumulated_jitter_that_never_gets_anywhere_stays_unapplied() {
        // Idle hand resting on a trackpad: sub-threshold noise in both
        // directions must not eventually add up into a zoom.
        let mut gesture = ScrollGesture::default();
        for _ in 0..20 {
            assert_eq!(gesture.feed(0.3, -0.3), (0.0, 0.0));
            assert_eq!(gesture.feed(-0.3, 0.3), (0.0, 0.0));
        }
        assert_eq!(gesture.axis(), None);
    }

    #[test]
    fn apply_zoom_none_returns_ranges_unchanged() {
        let ranges = [(0.0, 10.0), (5.0, 25.0)];
        let (out, span) = apply_zoom(&ranges, None);
        assert_eq!(out, ranges.to_vec());
        assert_eq!(span, 20.0);
    }

    #[test]
    fn apply_zoom_narrows_each_range_independently_in_lap_relative_terms() {
        // Full span is 20ms (from the second range). Zooming to the
        // second half (fraction 0.5..1.0) should keep each range's own
        // *second half*, lap-relative.
        let ranges = [(0.0, 10.0), (5.0, 25.0)];
        let (out, span) = apply_zoom(&ranges, Some((0.5, 1.0)));
        assert_eq!(span, 10.0);
        // First range (0..10): zoom window in absolute terms is
        // (0 + 0.5*20, 0 + 1.0*20) = (10, 20), clamped into (0, 10) -> (10, 10).
        assert_eq!(out[0], (10.0, 10.0));
        // Second range (5..25): zoom window is (5 + 10, 5 + 20) = (15, 25).
        assert_eq!(out[1], (15.0, 25.0));
    }

    #[test]
    fn apply_zoom_clamps_a_short_range_that_the_window_fully_exceeds() {
        let ranges = [(0.0, 100.0)];
        let (out, _) = apply_zoom(&ranges, Some((0.0, 1.0)));
        assert_eq!(out[0], (0.0, 100.0));
    }

    #[test]
    fn comparison_plot_rebases_each_range_to_start_at_zero() {
        // Two "laps" over the same channel: [0,10] and [20,30]. Both
        // cover the same two samples relative to their own start (v=1.0
        // at t=0, v=2.0 at t=10), so their traces should be identical.
        let c = Channel {
            name: "Test".into(),
            units: "u".into(),
            dec_pts: 2,
            interpolate: true,
            timecodes: vec![0.0, 10.0, 20.0, 30.0],
            values: vec![1.0, 2.0, 1.0, 2.0],
        };
        let ranges = [(0.0, 10.0), (20.0, 30.0)];
        let span = shared_duration(&ranges);
        let plot = build_lap_comparison_plot(&c, 100.0, 100.0, &ranges, span, AxisMode::Time, None)
            .unwrap();
        assert_eq!(plot.series.len(), 2);
        assert_eq!(plot.series[0].commands, plot.series[1].commands);
    }

    #[test]
    fn comparison_plot_omits_ranges_with_no_samples() {
        let c = channel(true);
        // Second range (1000..2000) is well past the channel's data.
        let ranges = [(0.0, 30.0), (1000.0, 2000.0)];
        let span = shared_duration(&ranges);
        let plot = build_lap_comparison_plot(&c, 100.0, 100.0, &ranges, span, AxisMode::Time, None)
            .unwrap();
        assert_eq!(plot.series.len(), 1);
    }

    #[test]
    fn comparison_plot_none_when_no_range_has_samples() {
        let c = channel(true);
        let ranges = [(1000.0, 2000.0)];
        let span = shared_duration(&ranges);
        assert!(
            build_lap_comparison_plot(&c, 100.0, 100.0, &ranges, span, AxisMode::Time, None)
                .is_none()
        );
    }

    #[test]
    fn comparison_plot_none_for_empty_ranges() {
        let c = channel(true);
        assert!(
            build_lap_comparison_plot(&c, 100.0, 100.0, &[], 1.0, AxisMode::Time, None).is_none()
        );
    }

    /// A distance channel resembling iRacing's `LapDist`: resets to `0` at
    /// t=0 and t=20 (two lap starts), climbing to 100 within each lap.
    fn distance_channel() -> Channel {
        Channel {
            name: "LapDist".into(),
            units: "m".into(),
            dec_pts: 1,
            interpolate: true,
            timecodes: vec![0.0, 10.0, 20.0, 30.0],
            values: vec![0.0, 100.0, 0.0, 100.0],
        }
    }

    #[test]
    fn distance_axis_span_is_the_largest_per_range_delta() {
        let dist = distance_channel();
        // First range covers half a lap's worth of distance (0..50 at
        // t=5), second covers a full lap's worth (0..100).
        let span = distance_axis_span(&dist, &[(0.0, 5.0), (20.0, 30.0)]);
        assert_eq!(span, 100.0);
    }

    #[test]
    fn time_at_distance_inverts_value_at_within_the_given_range() {
        let dist = distance_channel();
        // Within the first lap (0..10, distance 0..100), distance 50
        // should map back to t=5 (halfway).
        assert_eq!(time_at_distance(&dist, 0.0, 10.0, 50.0), Some(5.0));
        assert_eq!(time_at_distance(&dist, 0.0, 10.0, 0.0), Some(0.0));
    }

    #[test]
    fn time_at_distance_stays_within_its_own_lap_despite_the_reset() {
        // Same distance (50) but looked up within the *second* lap's
        // window (20..30) should map to that lap's own t=25, not get
        // confused by the first lap's identical distance value at t=5 —
        // the whole reason this function windows to `[start, end]` first
        // instead of searching the unwindowed (non-monotonic) channel.
        let dist = distance_channel();
        assert_eq!(time_at_distance(&dist, 20.0, 30.0, 50.0), Some(25.0));
    }

    #[test]
    fn comparison_plot_distance_axis_rebases_each_range_to_zero_distance() {
        let dist = distance_channel();
        // Channel value climbs 1.0 -> 8.0 across the first lap's samples;
        // plot it against distance instead of time.
        let c = Channel {
            name: "Test".into(),
            units: "u".into(),
            dec_pts: 2,
            interpolate: true,
            timecodes: vec![0.0, 5.0, 10.0],
            values: vec![1.0, 2.0, 4.0],
        };
        let ranges = [(0.0, 10.0)];
        let plot = build_lap_comparison_plot(
            &c,
            100.0,
            100.0,
            &ranges,
            10.0,
            AxisMode::Distance,
            Some(&dist),
        )
        .unwrap();
        assert_eq!(plot.series.len(), 1);
        // t=0 (distance 0) should plot at x=0; the trace should reach the
        // full view width by the last sample (distance 100, the axis span).
        assert!(plot.series[0].commands.starts_with("M 0 "));
    }

    #[test]
    fn comparison_plot_distance_axis_falls_back_to_time_when_channel_missing() {
        let c = channel(true);
        let ranges = [(0.0, 30.0)];
        let with_distance =
            build_lap_comparison_plot(&c, 100.0, 100.0, &ranges, 30.0, AxisMode::Distance, None);
        let time_mode =
            build_lap_comparison_plot(&c, 100.0, 100.0, &ranges, 30.0, AxisMode::Time, None);
        assert_eq!(with_distance, time_mode);
    }

    #[test]
    fn session_time_range_is_zero_zero_with_no_laps() {
        let session = sde_core::Session {
            channels: std::collections::HashMap::new(),
            laps: vec![],
            metadata: std::collections::HashMap::new(),
            key_channel_map: sde_core::KeyChannelMap::default(),
            file_name: "test".into(),
            time_penalties: Vec::new(),
        };
        assert_eq!(session_time_range(&session), (0.0, 0.0));
    }

    fn session_with(channels: Vec<Channel>) -> sde_core::Session {
        sde_core::Session {
            channels: channels.into_iter().map(|c| (c.name.clone(), c)).collect(),
            laps: vec![],
            metadata: std::collections::HashMap::new(),
            key_channel_map: sde_core::KeyChannelMap::default(),
            file_name: "test".into(),
            time_penalties: Vec::new(),
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

    fn stub_channel(name: &str) -> Channel {
        Channel {
            name: name.to_string(),
            units: String::new(),
            dec_pts: 0,
            interpolate: true,
            timecodes: vec![0.0],
            values: vec![0.0],
        }
    }

    #[test]
    fn default_dock_channels_picks_one_match_per_role_ibt_style_names() {
        // IBT-shaped session: "Speed" not "Ground Speed", "Engine0_RPM"
        // not "RPM", no steering channel present at all.
        let session = session_with(vec![
            stub_channel("Speed"),
            stub_channel("Engine0_RPM"),
            stub_channel("Throttle"),
            stub_channel("BrakeRaw"),
            stub_channel("Gear"),
            stub_channel("SomeUnrelatedChannel"),
        ]);
        let docks = default_dock_channels(&session);
        assert_eq!(
            docks,
            vec![
                vec!["Speed".to_string()],
                vec!["Engine0_RPM".to_string()],
                vec!["Throttle".to_string()],
                vec!["BrakeRaw".to_string()],
                vec!["Gear".to_string()],
            ]
        );
    }

    #[test]
    fn default_dock_channels_picks_one_match_per_role_rsf_ngp_style_names() {
        // RSF/NGP-shaped session: the exact lowercase dotted field names
        // confirmed against a real capture (see the DEFAULT_DOCK_ROLES doc
        // comment) — this used to match none of the other formats' names
        // and fall through to a single-channel worksheet.
        let session = session_with(vec![
            stub_channel("speed"),
            stub_channel("engineRotation"),
            stub_channel("throttle"),
            stub_channel("brake"),
            stub_channel("gear"),
            stub_channel("steering"),
            stub_channel("LF.brakeDiskTemp"),
        ]);
        let docks = default_dock_channels(&session);
        assert_eq!(
            docks,
            vec![
                vec!["speed".to_string()],
                vec!["engineRotation".to_string()],
                vec!["throttle".to_string()],
                vec!["brake".to_string()],
                vec!["gear".to_string()],
                vec!["steering".to_string()],
            ]
        );
    }

    #[test]
    fn default_dock_channels_falls_back_to_pick_default_channel_when_no_role_matches() {
        let session = session_with(vec![stub_channel("SomeWeirdChannel")]);
        let docks = default_dock_channels(&session);
        assert_eq!(docks, vec![vec!["SomeWeirdChannel".to_string()]]);
    }

    #[test]
    fn default_dock_channels_empty_for_a_session_with_no_channels_at_all() {
        let session = session_with(vec![]);
        assert!(default_dock_channels(&session).is_empty());
    }

    #[test]
    fn drag_drop_target_stacked_counts_whole_rows_moved() {
        // 4 docks stacked, 160px rows, dragging dock 1.
        let target = |dy| drag_drop_target(DockLayout::Stacked, 1, 4, 0.0, dy, 400.0, 160.0);
        assert_eq!(target(160.0), Some(2));
        assert_eq!(target(-160.0), Some(0));
        assert_eq!(target(330.0), Some(3), "two rows down, plus a bit");
        // Horizontal movement is meaningless in a single-column layout.
        assert_eq!(
            drag_drop_target(DockLayout::Stacked, 1, 4, 900.0, 0.0, 400.0, 160.0),
            None
        );
    }

    #[test]
    fn drag_drop_target_is_none_for_a_drag_too_short_to_leave_its_own_cell() {
        // This is what makes a plain click on the drag handle harmless.
        assert_eq!(
            drag_drop_target(DockLayout::Stacked, 1, 4, 0.0, 3.0, 400.0, 160.0),
            None
        );
        assert_eq!(
            drag_drop_target(DockLayout::SideBySide, 1, 4, -5.0, 0.0, 200.0, 400.0),
            None
        );
    }

    #[test]
    fn drag_drop_target_clamps_to_the_docks_that_exist() {
        assert_eq!(
            drag_drop_target(DockLayout::Stacked, 2, 3, 0.0, 800.0, 400.0, 160.0),
            None,
            "dragging below the last dock targets nothing"
        );
        assert_eq!(
            drag_drop_target(DockLayout::Stacked, 0, 3, 0.0, -800.0, 400.0, 160.0),
            None,
            "dragging above the first dock targets nothing"
        );
    }

    #[test]
    fn drag_drop_target_side_by_side_counts_whole_columns_moved() {
        let target = |dx| drag_drop_target(DockLayout::SideBySide, 0, 3, dx, 0.0, 200.0, 400.0);
        assert_eq!(target(200.0), Some(1));
        assert_eq!(target(420.0), Some(2));
        assert_eq!(target(-200.0), None);
    }

    #[test]
    fn drag_drop_target_grid_tracks_both_axes() {
        // 5 docks in a 2-wide grid (rows 0,1,2), 300x200 cells, dragging
        // dock 1 (row 0, col 1).
        let target = |dx, dy| drag_drop_target(DockLayout::Grid, 1, 5, dx, dy, 300.0, 200.0);
        assert_eq!(target(-300.0, 0.0), Some(0));
        assert_eq!(target(0.0, 200.0), Some(3));
        assert_eq!(target(-300.0, 400.0), Some(4));
    }

    #[test]
    fn drag_drop_target_grid_does_not_wrap_off_the_edge_onto_another_row() {
        // Dock 1 is in the right-hand column; dragging further right is
        // empty space, not dock 2 (which is the *next row's* left cell).
        assert_eq!(
            drag_drop_target(DockLayout::Grid, 1, 5, 300.0, 0.0, 300.0, 200.0),
            None
        );
        assert_eq!(
            drag_drop_target(DockLayout::Grid, 0, 5, -300.0, 0.0, 300.0, 200.0),
            None
        );
    }

    #[test]
    fn drag_drop_target_rejects_a_degenerate_cell_size() {
        // Before the first layout pass a dock can report zero size.
        assert_eq!(
            drag_drop_target(DockLayout::Stacked, 0, 3, 0.0, 100.0, 0.0, 0.0),
            None
        );
    }

    /// Three single-channel docks, for the reorder/merge tests below.
    fn stub_docks() -> Vec<Vec<String>> {
        vec![
            vec!["Speed".to_string()],
            vec!["Throttle".to_string()],
            vec!["Brake".to_string()],
        ]
    }

    #[test]
    fn reorder_docks_moves_a_dock_to_the_target_index_from_either_direction() {
        let mut docks = stub_docks();
        assert!(reorder_docks(&mut docks, 0, 2));
        assert_eq!(
            docks,
            vec![
                vec!["Throttle".to_string()],
                vec!["Brake".to_string()],
                vec!["Speed".to_string()],
            ],
            "dragging down lands on the target index, not past it"
        );

        let mut docks = stub_docks();
        assert!(reorder_docks(&mut docks, 2, 0));
        assert_eq!(
            docks,
            vec![
                vec!["Brake".to_string()],
                vec!["Speed".to_string()],
                vec!["Throttle".to_string()],
            ]
        );
    }

    #[test]
    fn reorder_docks_keeps_every_dock_and_never_merges_them() {
        // The whole point of the plain drag: nothing is combined.
        let mut docks = stub_docks();
        assert!(reorder_docks(&mut docks, 1, 0));
        assert_eq!(docks.len(), 3);
        assert!(docks.iter().all(|d| d.len() == 1));
    }

    #[test]
    fn reorder_docks_is_a_no_op_for_a_self_move_or_an_out_of_range_index() {
        let mut docks = stub_docks();
        let before = docks.clone();
        assert!(!reorder_docks(&mut docks, 1, 1));
        assert!(!reorder_docks(&mut docks, 9, 0));
        assert!(!reorder_docks(&mut docks, 0, 9));
        assert_eq!(docks, before);
    }

    #[test]
    fn merge_docks_appends_the_source_channels_and_removes_the_source() {
        let mut docks = vec![
            vec!["Speed".to_string()],
            vec!["Throttle".to_string()],
            vec!["Brake".to_string()],
        ];
        assert!(merge_docks(&mut docks, 2, 0));
        assert_eq!(
            docks,
            vec![
                vec!["Speed".to_string(), "Brake".to_string()],
                vec!["Throttle".to_string()],
            ]
        );
    }

    #[test]
    fn merge_docks_targets_the_right_dock_when_the_source_precedes_it() {
        // The removal shifts later docks down one — the target has to
        // follow it, or the channels land on the wrong dock.
        let mut docks = vec![
            vec!["Speed".to_string()],
            vec!["Throttle".to_string()],
            vec!["Brake".to_string()],
        ];
        assert!(merge_docks(&mut docks, 0, 2));
        assert_eq!(
            docks,
            vec![
                vec!["Throttle".to_string()],
                vec!["Brake".to_string(), "Speed".to_string()],
            ]
        );
    }

    #[test]
    fn merge_docks_does_not_duplicate_a_channel_the_target_already_plots() {
        let mut docks = vec![
            vec!["Speed".to_string(), "Throttle".to_string()],
            vec!["Throttle".to_string(), "Brake".to_string()],
        ];
        assert!(merge_docks(&mut docks, 1, 0));
        assert_eq!(
            docks,
            vec![vec![
                "Speed".to_string(),
                "Throttle".to_string(),
                "Brake".to_string()
            ]]
        );
    }

    #[test]
    fn merge_docks_is_a_no_op_for_a_self_merge_or_an_out_of_range_index() {
        let mut docks = vec![vec!["Speed".to_string()], vec!["Throttle".to_string()]];
        let before = docks.clone();
        assert!(!merge_docks(&mut docks, 1, 1));
        assert!(!merge_docks(&mut docks, 5, 0));
        assert!(!merge_docks(&mut docks, 0, 5));
        assert_eq!(docks, before);
    }

    #[test]
    fn dock_layout_from_index_maps_the_slint_layout_mode_ints() {
        assert_eq!(DockLayout::from_index(0), DockLayout::Stacked);
        assert_eq!(DockLayout::from_index(1), DockLayout::SideBySide);
        assert_eq!(DockLayout::from_index(2), DockLayout::Grid);
        assert_eq!(DockLayout::from_index(-1), DockLayout::Stacked);
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

    #[test]
    fn channel_has_data_is_false_for_empty_or_all_zero_channels() {
        let empty = Channel {
            name: "Empty".into(),
            units: String::new(),
            dec_pts: 0,
            interpolate: true,
            timecodes: vec![],
            values: vec![],
        };
        assert!(!channel_has_data(&empty));

        let all_zero = Channel {
            name: "CarPosX_raw".into(),
            units: String::new(),
            dec_pts: 2,
            interpolate: true,
            timecodes: vec![0.0, 10.0, 20.0],
            values: vec![0.0, 0.0, 0.0],
        };
        assert!(!channel_has_data(&all_zero));
    }

    #[test]
    fn channel_has_data_is_true_for_a_nonzero_flat_or_varying_channel() {
        let flat_nonzero = Channel {
            name: "AirTemp_C".into(),
            units: "\u{b0}C".into(),
            dec_pts: 1,
            interpolate: true,
            timecodes: vec![0.0, 10.0],
            values: vec![12.51, 12.51],
        };
        assert!(channel_has_data(&flat_nonzero));

        let varying = channel(true);
        assert!(channel_has_data(&varying));
    }
}
