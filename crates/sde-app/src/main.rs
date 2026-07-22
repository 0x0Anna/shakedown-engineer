//! Slint GUI shell: load a MoTeC `.ld` file, search/pick channels to add
//! to a worksheet of stacked line-graph docks, optionally restrict the
//! view to a single lap (or overlay several laps for comparison), and
//! drag a shared vertical time cursor over the docks. See
//! PROJECT_PLAN.md for scope; see `graph.rs` for the pure plotting/lookup
//! logic this file wires up to the UI.

// clippy::pedantic/nursery notes (not part of the default lint set the
// project otherwise keeps clean):
// - doc_markdown fires repeatedly on plain-English mentions of
//   `PROJECT_PLAN.md`/MoTeC in prose above; not worth backtick-wrapping
//   every occurrence for a doc-only lint.
// - suboptimal_flops wants `mul_add` for the cursor-fraction lerp below;
//   for this tiny bit of UI-coordinate math, the plain form reads better
//   than the marginal FMA precision/perf gain is worth.
// - too_many_lines fires on `main`, which is just flat callback
//   registration (one closure per Slint callback) — splitting it up
//   would add indirection without making any single piece simpler.
#![allow(
    clippy::doc_markdown,
    clippy::suboptimal_flops,
    clippy::too_many_lines
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use slint::ComponentHandle;

use sde_app::graph;

slint::include_modules!();

/// The bits of a plotted channel the cursor-drag handler needs, kept
/// outside the `Session`/`Channel` so we don't have to clone the whole
/// session into the closure. One of these is cached per dock (keyed by
/// channel name) in `AppState::plotted`.
struct PlottedChannel {
    timecodes: Vec<f64>,
    values: Vec<f64>,
    interpolate: bool,
}

/// A small fixed palette cycled through for a dock's overlaid traces
/// (one per channel-x-lap combination it plots) and reused for the lap
/// legend swatches — distinct enough at a glance, in a
/// dark-graph-background-friendly range. Eight rather than four now that
/// a dock can overlay multiple channels as well as multiple laps.
const SERIES_COLORS: [(u8, u8, u8); 8] = [
    (78, 161, 255),
    (255, 158, 68),
    (120, 220, 130),
    (230, 100, 210),
    (240, 220, 90),
    (100, 220, 220),
    (200, 130, 255),
    (255, 120, 120),
];

const fn series_color(index: usize) -> slint::Color {
    let (r, g, b) = SERIES_COLORS[index % SERIES_COLORS.len()];
    slint::Color::from_rgb_u8(r, g, b)
}

/// Everything that needs to survive between callbacks: the loaded
/// session, the full (unfiltered) channel name list, the current channel
/// search text, the ordered set of channels shown as worksheet docks, the
/// lap selection state, and the cursor-lookup cache. Grouped into one
/// struct behind a single `Rc<RefCell<_>>` so each callback only needs to
/// clone one `Rc`.
#[derive(Default)]
struct AppState {
    session: Option<sde_core::Session>,
    all_channel_names: Vec<String>,
    filter_text: String,
    /// Worksheet docks, in display order. Each dock overlays one or more
    /// channels (see `overlay_pending`/`channel_overlay_toggled` for how
    /// a multi-channel dock gets created) — usually just one.
    dock_channels: Vec<Vec<String>>,
    /// Channels queued (via Ctrl+click in the sidebar) to become one new
    /// overlay dock once "Add overlay dock" is clicked; cleared after.
    overlay_pending: Vec<String>,
    /// `0` = whole session ("All"); `n >= 1` selects `session.laps[n - 1]`
    /// (mirrors `graph::lap_labels`' indexing). Only consulted when
    /// `compare_lap_indices` is empty.
    selected_lap_index: usize,
    /// 1-based lap numbers (`session.laps[n - 1]`) currently selected for
    /// comparison, sorted ascending. Non-empty means every dock overlays
    /// one trace per entry here instead of using `selected_lap_index`.
    compare_lap_indices: Vec<usize>,
    /// Names of channels created via the math-channel formula box, in
    /// creation order, so they can be listed with a remove button. Each
    /// one also lives in `session.channels` like any parsed channel.
    math_channel_names: Vec<String>,
    /// Non-linear-editor-style timeline zoom/pan window: fractions
    /// `(start, end)` of the current lap-selection's shared duration
    /// (see `graph::apply_zoom`). `None` means fully zoomed out — kept as
    /// `None` rather than `Some((0.0, 1.0))` so "is a zoom active" is a
    /// simple `is_some()` check.
    zoom: Option<(f64, f64)>,
    /// The axis (zoom vs. pan) the *current* scroll gesture is locked to,
    /// and when its last event arrived. Trackpads report a nonzero
    /// `delta_x`/`delta_y` on nearly every event even for an intended
    /// single-axis swipe, so reading both every event would make an
    /// intended pan also drift the zoom level (and vice versa). Locking
    /// to whichever axis dominated the *first* event of a gesture, and
    /// re-deciding only after a pause (see `SCROLL_GESTURE_TIMEOUT`),
    /// keeps the whole gesture on one axis the way a trackpad user
    /// expects.
    scroll_gesture: Option<(graph::ScrollAxis, std::time::Instant)>,
    plotted: HashMap<String, PlottedChannel>,
    /// Bumped every time `session.channels`' contents change (a new
    /// session loads, or a math channel is added/redefined/removed).
    /// Paired with `dock_channels` in `plotted_key` so `replot` can tell
    /// whether its cached `plotted` map is still valid without having to
    /// re-clone every channel's samples on every call (see `replot`).
    session_generation: u64,
    /// The `(session_generation, dock_channels)` that `plotted` was last
    /// built from, so a `replot` triggered purely by zoom/pan (which
    /// touches neither) can skip re-cloning every plotted channel's full
    /// sample arrays.
    plotted_key: Option<(u64, Vec<Vec<String>>)>,
}

/// How long a pause between scroll events ends the current gesture (so
/// the next event re-decides which axis to lock to, rather than being
/// stuck on whatever the last gesture used).
const SCROLL_GESTURE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);

const VIEW_WIDTH: f64 = 1000.0;
const VIEW_HEIGHT: f64 = 1000.0;
/// Fixed column count for the "Grid" worksheet layout (see `app.slint`'s
/// `DockData.grid-col`/`grid-row`).
const GRID_COLUMNS: i32 = 2;

fn main() -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    let state: Rc<RefCell<AppState>> = Rc::new(RefCell::new(AppState::default()));

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_open_file(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };

            let Some(path) = rfd::FileDialog::new()
                .add_filter("MoTeC log", &["ld"])
                .set_title("Open MoTeC .ld file")
                .pick_file()
            else {
                return; // user cancelled
            };

            load_file(&window, &state, &path);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_channel_filter_changed(move |query| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            state.borrow_mut().filter_text = query.to_string();
            refresh_channel_list(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_dock_selected(move |name| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            {
                let mut state = state.borrow_mut();
                let name = name.to_string();
                if let Some(pos) = state
                    .dock_channels
                    .iter()
                    .position(|g| g.len() == 1 && g[0] == name)
                {
                    // It's already its own standalone dock — remove it.
                    state.dock_channels.remove(pos);
                } else if !state.dock_channels.iter().any(|g| g.contains(&name)) {
                    // Not on the worksheet at all yet — add it as a new
                    // standalone dock. (If it's already part of a
                    // multi-channel overlay dock, a plain click does
                    // nothing — remove the whole dock via its "x" instead,
                    // since "which dock" would be ambiguous otherwise.)
                    state.dock_channels.push(vec![name]);
                }
            }
            refresh_channel_list(&window, &state);
            replot(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_dock_removed(move |index| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            if let Ok(index) = usize::try_from(index) {
                let mut state_mut = state.borrow_mut();
                if index < state_mut.dock_channels.len() {
                    state_mut.dock_channels.remove(index);
                }
                drop(state_mut);
                refresh_channel_list(&window, &state);
                replot(&window, &state);
            }
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_channel_overlay_toggled(move |name| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            {
                let mut state = state.borrow_mut();
                let name = name.to_string();
                if let Some(pos) = state.overlay_pending.iter().position(|n| *n == name) {
                    state.overlay_pending.remove(pos);
                } else {
                    state.overlay_pending.push(name);
                }
            }
            refresh_channel_list(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_overlay_add_requested(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            {
                let mut state_mut = state.borrow_mut();
                if !state_mut.overlay_pending.is_empty() {
                    let group = std::mem::take(&mut state_mut.overlay_pending);
                    state_mut.dock_channels.push(group);
                }
            }
            refresh_channel_list(&window, &state);
            replot(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_overlay_cleared(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            state.borrow_mut().overlay_pending.clear();
            refresh_channel_list(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_lap_selected(move |index| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            // `current-index` is `-1` before Slint's ComboBox has ever had
            // a selection made; treat that the same as "All" (index 0).
            let index = usize::try_from(index).unwrap_or(0);
            {
                let mut state_mut = state.borrow_mut();
                state_mut.selected_lap_index = index;
                // Picking from the plain lap dropdown is a fresh,
                // non-comparison selection — leaving stale "compare"
                // toggles active would otherwise silently override it.
                state_mut.compare_lap_indices.clear();
                // A zoom window is a fraction of the *previous* selection's
                // duration; carrying it over to a different lap selection
                // would silently show an unrelated, likely-confusing slice.
                state_mut.zoom = None;
            }
            refresh_compare_ui(&window, &state);
            refresh_zoom_ui(&window, &state);
            replot(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_compare_lap_toggled(move |index| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            if let Ok(index) = usize::try_from(index) {
                let lap_num = index + 1;
                let mut state_mut = state.borrow_mut();
                if let Some(pos) = state_mut
                    .compare_lap_indices
                    .iter()
                    .position(|&n| n == lap_num)
                {
                    state_mut.compare_lap_indices.remove(pos);
                } else {
                    state_mut.compare_lap_indices.push(lap_num);
                    state_mut.compare_lap_indices.sort_unstable();
                }
                // Same reasoning as the plain lap dropdown: the
                // comparison set just changed, so any existing zoom
                // window (a fraction of the *old* set's shared duration)
                // no longer means the same thing.
                state_mut.zoom = None;
                drop(state_mut);
                refresh_compare_ui(&window, &state);
                refresh_zoom_ui(&window, &state);
                replot(&window, &state);
            }
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_compare_cleared(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            {
                let mut state_mut = state.borrow_mut();
                state_mut.compare_lap_indices.clear();
                state_mut.zoom = None;
            }
            refresh_compare_ui(&window, &state);
            refresh_zoom_ui(&window, &state);
            replot(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_zoom_scrolled(move |delta_x, delta_y, fraction| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            {
                let mut state_mut = state.borrow_mut();
                let (delta_x, delta_y) = (f64::from(delta_x), f64::from(delta_y));

                let now = std::time::Instant::now();
                let axis = match state_mut.scroll_gesture {
                    // Continuing a gesture still in progress: keep its
                    // locked axis regardless of this event's own deltas.
                    Some((axis, last)) if now.duration_since(last) < SCROLL_GESTURE_TIMEOUT => axis,
                    // No gesture in progress (or the previous one timed
                    // out): this event starts a new one, locked to
                    // whichever axis it dominates.
                    _ => graph::dominant_scroll_axis(delta_x, delta_y),
                };
                state_mut.scroll_gesture = Some((axis, now));

                // Zero out the non-locked axis's delta entirely, rather
                // than just picking which effect to apply — trackpad
                // jitter on the "wrong" axis shouldn't leak through even
                // partially.
                let (delta_x, delta_y) = match axis {
                    graph::ScrollAxis::Zoom => (0.0, delta_y),
                    graph::ScrollAxis::Pan => (delta_x, 0.0),
                };

                let current = state_mut.zoom.unwrap_or((0.0, 1.0));
                let updated =
                    graph::zoom_scroll(current, delta_x, delta_y, f64::from(fraction));
                state_mut.zoom = (!graph::is_full_zoom(updated)).then_some(updated);
            }
            replot(&window, &state);
            refresh_zoom_ui(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_zoom_reset(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            state.borrow_mut().zoom = None;
            replot(&window, &state);
            refresh_zoom_ui(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_math_channel_add_requested(move |name, formula| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let name = name.to_string();
            if name.trim().is_empty() {
                window.set_math_channel_error("Name cannot be empty.".into());
                return;
            }

            let mut state_mut = state.borrow_mut();
            // Redefining an existing math channel (same name) is allowed
            // (it just overwrites in place); reusing the name of a
            // parsed/real channel is not, so math channels can never
            // shadow actual telemetry data. Computed before taking
            // `session` mutably below so the two borrows don't overlap.
            let is_existing_math = state_mut.math_channel_names.contains(&name);
            let Some(session) = state_mut.session.as_mut() else {
                window.set_math_channel_error("Load a file first.".into());
                return;
            };

            if session.channels.contains_key(&name) && !is_existing_math {
                drop(state_mut);
                window.set_math_channel_error(format!("A channel named \"{name}\" already exists.").into());
                return;
            }

            match sde_core::mathexpr::evaluate_math_channel(session, &name, &formula) {
                Ok(channel) => {
                    session.channels.insert(name.clone(), channel);
                    // Last use of `session` (and thus of the borrow of
                    // `state_mut.session` it came from) — `state_mut` can
                    // be touched mutably again after this.
                    let all_channel_names = graph::channel_names(session);

                    if !is_existing_math {
                        state_mut.math_channel_names.push(name.clone());
                    }
                    state_mut.all_channel_names = all_channel_names;
                    state_mut.session_generation += 1;
                    let math_channel_names = state_mut.math_channel_names.clone();
                    drop(state_mut);

                    window.set_math_channel_error(String::new().into());
                    window.set_math_name_text(String::new().into());
                    window.set_math_formula_text(String::new().into());
                    window.set_math_channel_names(to_model(math_channel_names));
                    refresh_channel_list(&window, &state);
                }
                Err(e) => {
                    drop(state_mut);
                    window.set_math_channel_error(e.to_string().into());
                }
            }
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_math_channel_removed(move |index| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let Ok(index) = usize::try_from(index) else {
                return;
            };

            {
                let mut state_mut = state.borrow_mut();
                if index >= state_mut.math_channel_names.len() {
                    return;
                }
                let name = state_mut.math_channel_names.remove(index);
                for group in &mut state_mut.dock_channels {
                    group.retain(|n| *n != name);
                }
                state_mut.dock_channels.retain(|g| !g.is_empty());
                state_mut.overlay_pending.retain(|n| *n != name);
                if let Some(session) = state_mut.session.as_mut() {
                    session.channels.remove(&name);
                    state_mut.all_channel_names = graph::channel_names(session);
                    state_mut.session_generation += 1;
                }
            }

            let math_channel_names = state.borrow().math_channel_names.clone();
            window.set_math_channel_names(to_model(math_channel_names));
            refresh_channel_list(&window, &state);
            replot(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        // Last use of the outer `state` Rc in `main`, so it can just be
        // moved into this closure instead of cloned.
        window.on_cursor_moved(move |fraction| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let state = state.borrow();
            let Some(session) = state.session.as_ref() else {
                return;
            };
            let ranges = current_ranges(session, &state);
            let (ranges, time_span) = graph::apply_zoom(&ranges, state.zoom);
            if ranges.is_empty() {
                return;
            }
            let t_rel = f64::from(fraction) * time_span;

            let cursor_values: Vec<slint::SharedString> = state
                .dock_channels
                .iter()
                .map(|group| cursor_text_for_group(&state, group, &ranges, t_rel).into())
                .collect();

            window.set_cursor_visible(true);
            window.set_cursor_fraction(fraction);
            window.set_cursor_time_text(format!("{t_rel:.1}").into());
            window.set_cursor_values(slint::ModelRc::new(slint::VecModel::from(cursor_values)));
        });
    }

    window.run()
}

/// The cursor readout text for one dock: each channel in `group` gets its
/// pipe-separated per-`ranges` values (`"12.3 | 15.0"` when comparing
/// laps); a single-channel dock keeps that bare (matching the original
/// format), while a multi-channel overlay dock prefixes each with its
/// channel name (`"BRAKE: 80.0   THROTTLE: 0.0"`) so the values stay
/// identifiable once more than one channel shares the graph.
fn cursor_text_for_group(
    state: &AppState,
    group: &[String],
    ranges: &[(f64, f64)],
    t_rel: f64,
) -> String {
    let parts: Vec<String> = group
        .iter()
        .map(|name| {
            let values = state.plotted.get(name).map_or_else(String::new, |c| {
                ranges
                    .iter()
                    .filter_map(|&(start, end)| {
                        let abs_t = (start + t_rel).min(end);
                        graph::value_at_raw(&c.timecodes, &c.values, c.interpolate, abs_t)
                            .map(|v| format!("{v:.3}"))
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            });
            let values = if values.is_empty() { "n/a".to_string() } else { values };
            if group.len() == 1 {
                values
            } else {
                format!("{name}: {values}")
            }
        })
        .collect();
    parts.join("   ")
}

/// Turn a `Vec<String>` into the `ModelRc<SharedString>` Slint's
/// generated `[string]` properties expect.
fn to_model(items: Vec<String>) -> slint::ModelRc<slint::SharedString> {
    let items: Vec<slint::SharedString> = items.into_iter().map(Into::into).collect();
    slint::ModelRc::new(slint::VecModel::from(items))
}

fn to_bool_model(items: Vec<bool>) -> slint::ModelRc<bool> {
    slint::ModelRc::new(slint::VecModel::from(items))
}

fn to_dock_model(items: Vec<DockData>) -> slint::ModelRc<DockData> {
    slint::ModelRc::new(slint::VecModel::from(items))
}

/// The time windows every dock should plot: normally a single range (the
/// selected lap's `(start_time, end_time)`, or the whole session's for
/// "All"); when comparing laps, one range per entry in
/// `compare_lap_indices`, each becoming one overlaid trace per dock.
fn current_ranges(session: &sde_core::Session, state: &AppState) -> Vec<(f64, f64)> {
    if state.compare_lap_indices.is_empty() {
        current_range(session, state.selected_lap_index)
            .into_iter()
            .collect()
    } else {
        state
            .compare_lap_indices
            .iter()
            .filter_map(|&i| session.laps.get(i - 1))
            .map(|lap| (lap.start_time, lap.end_time))
            .collect()
    }
}

/// The single-range case of [`current_ranges`]: a selected lap's
/// `(start_time, end_time)`, or the whole session's for the "All" entry
/// (index `0`).
fn current_range(session: &sde_core::Session, lap_index: usize) -> Option<(f64, f64)> {
    if lap_index > 0 {
        session
            .laps
            .get(lap_index - 1)
            .map(|lap| (lap.start_time, lap.end_time))
    } else {
        Some(graph::session_time_range(session))
    }
}

/// Recompute the (filtered) channel list and which of those channels are
/// currently on the worksheet, and push both into the window. Called
/// whenever the search text or the dock set changes.
fn refresh_channel_list(window: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();
    let filtered = graph::filter_channel_names(&state.all_channel_names, &state.filter_text);
    let active: Vec<bool> = filtered
        .iter()
        .map(|n| state.dock_channels.iter().any(|g| g.contains(n)))
        .collect();
    let pending: Vec<bool> = filtered
        .iter()
        .map(|n| state.overlay_pending.contains(n))
        .collect();
    window.set_channel_names(to_model(filtered));
    window.set_channel_active(to_bool_model(active));
    window.set_channel_overlay_pending(to_bool_model(pending));
    window.set_overlay_pending_names(to_model(state.overlay_pending.clone()));
}

/// Recompute which "compare" chips are toggled on and the legend (lap
/// number -> series color), and push both into the window. Called
/// whenever the compare-lap selection or the plain lap dropdown changes.
fn refresh_compare_ui(window: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();
    let Some(session) = state.session.as_ref() else {
        return;
    };
    let active: Vec<bool> = (1..=session.laps.len())
        .map(|i| state.compare_lap_indices.contains(&i))
        .collect();
    window.set_compare_lap_active(to_bool_model(active));

    let legend: Vec<LegendEntry> = state
        .compare_lap_indices
        .iter()
        .enumerate()
        .map(|(i, &lap_num)| LegendEntry {
            label: format!("Lap {lap_num}").into(),
            color: series_color(i),
        })
        .collect();
    window.set_legend(slint::ModelRc::new(slint::VecModel::from(legend)));

    // Drives `app.slint`'s swap between the plain lap `ComboBox` and a
    // "Comparing N laps / Clear" indicator: whenever the compare chips
    // have any lap selected, they override the dropdown's own selection
    // entirely (see `current_ranges`), so showing both at once would
    // leave it ambiguous which one is actually in effect.
    let status = if state.compare_lap_indices.is_empty() {
        String::new()
    } else {
        let n = state.compare_lap_indices.len();
        let plural = if n == 1 { "" } else { "s" };
        format!("Comparing {n} lap{plural}")
    };
    window.set_compare_status_text(status.into());
}

/// Push the current zoom window's human-readable range (e.g. `"12.0s -
/// 48.0s of 120.7s"`) into the window, or clear it when fully zoomed out
/// (see `app.slint`'s `zoom-range-text`, which hides the zoom readout/
/// reset button whenever this is empty). Called whenever the zoom
/// window, lap selection, or comparison set changes.
fn refresh_zoom_ui(window: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();
    let Some(session) = state.session.as_ref() else {
        window.set_zoom_range_text(String::new().into());
        return;
    };
    let ranges = current_ranges(session, &state);
    let Some(zoom) = state.zoom else {
        window.set_zoom_range_text(String::new().into());
        return;
    };
    if ranges.is_empty() || graph::is_full_zoom(zoom) {
        window.set_zoom_range_text(String::new().into());
        return;
    }

    let full_span_s = graph::shared_duration(&ranges) / 1000.0;
    let start_s = zoom.0 * full_span_s;
    let end_s = zoom.1 * full_span_s;
    window.set_zoom_range_text(format!("{start_s:.1}s - {end_s:.1}s of {full_span_s:.1}s").into());
}

fn load_file(window: &AppWindow, state: &Rc<RefCell<AppState>>, path: &Path) {
    let file_name = path.file_name().map_or_else(
        || path.to_string_lossy().to_string(),
        |n| n.to_string_lossy().to_string(),
    );

    let session = match sde_core::Session::load_motec(path) {
        Ok(s) => s,
        Err(e) => {
            window.set_window_title(format!("sde-app — failed to load {file_name}").into());
            window.set_status_text(format!("Error loading {file_name}: {e}").into());
            *state.borrow_mut() = AppState::default();
            window.set_docks(to_dock_model(vec![]));
            window.set_channel_names(to_model(vec![]));
            window.set_channel_active(to_bool_model(vec![]));
            window.set_channel_overlay_pending(to_bool_model(vec![]));
            window.set_overlay_pending_names(to_model(vec![]));
            window.set_lap_labels(to_model(vec![]));
            window.set_compare_lap_labels(to_model(vec![]));
            window.set_compare_lap_active(to_bool_model(vec![]));
            window.set_compare_status_text(String::new().into());
            window.set_legend(slint::ModelRc::new(slint::VecModel::from(Vec::<
                LegendEntry,
            >::new(
            ))));
            window.set_math_channel_names(to_model(vec![]));
            window.set_math_channel_error(String::new().into());
            window.set_math_name_text(String::new().into());
            window.set_math_formula_text(String::new().into());
            window.set_zoom_range_text(String::new().into());
            return;
        }
    };

    let all_channel_names = graph::channel_names(&session);
    let lap_labels = graph::lap_labels(&session);
    let compare_lap_labels: Vec<String> = (1..=session.laps.len()).map(|n| n.to_string()).collect();
    let default_channel = graph::pick_default_channel(&session).map(|c| c.name.clone());

    {
        let mut state = state.borrow_mut();
        state.session = Some(session);
        state.all_channel_names.clone_from(&all_channel_names);
        state.filter_text.clear();
        state.dock_channels = default_channel.into_iter().map(|n| vec![n]).collect();
        state.overlay_pending.clear();
        state.selected_lap_index = 0;
        state.compare_lap_indices.clear();
        state.math_channel_names.clear();
        state.zoom = None;
        state.session_generation += 1;
    }

    window.set_window_title(format!("sde-app — {file_name}").into());
    window.set_status_text("No channels added to the worksheet yet — click one on the left.".into());
    window.set_lap_labels(to_model(lap_labels));
    window.set_current_lap_index(0);
    window.set_compare_lap_labels(to_model(compare_lap_labels));
    window.set_channel_filter_text(String::new().into());
    window.set_math_channel_names(to_model(vec![]));
    window.set_math_channel_error(String::new().into());
    window.set_math_name_text(String::new().into());
    window.set_math_formula_text(String::new().into());
    window.set_zoom_range_text(String::new().into());

    refresh_channel_list(window, state);
    refresh_compare_ui(window, state);
    refresh_zoom_ui(window, state);
    replot(window, state);
}

/// Rebuild every dock's plot from `state`'s current session/worksheet
/// channels/lap selection, and push the result into the window's
/// properties. Called after loading a file, adding/removing a dock, or
/// changing the lap selection/comparison.
fn replot(window: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let mut state = state.borrow_mut();
    let Some(session) = state.session.as_ref() else {
        return;
    };

    if state.dock_channels.is_empty() {
        state.plotted.clear();
        state.plotted_key = None;
        window.set_docks(to_dock_model(vec![]));
        window.set_cursor_visible(false);
        return;
    }

    let ranges = current_ranges(session, &state);
    let (ranges, time_span) = graph::apply_zoom(&ranges, state.zoom);
    if ranges.is_empty() {
        state.plotted.clear();
        state.plotted_key = None;
        window.set_docks(to_dock_model(vec![]));
        window.set_status_text("Selected lap has no valid time range.".into());
        window.set_cursor_visible(false);
        return;
    }

    // `plotted` only depends on `session.channels` and `dock_channels`
    // (which channels are plotted), not on `ranges`/`time_span` (the
    // zoom/lap window) — so a `replot` triggered purely by a zoom/pan
    // scroll event (see `on_zoom_scrolled`) can reuse the existing cache
    // instead of re-cloning every plotted channel's full sample arrays,
    // which matters since those scroll unboundedly fast on a trackpad.
    let rebuild_plotted =
        state.plotted_key.as_ref() != Some(&(state.session_generation, state.dock_channels.clone()));
    let mut plotted = rebuild_plotted.then(HashMap::new);

    let mut docks = Vec::with_capacity(state.dock_channels.len());

    for (i, group) in state.dock_channels.iter().enumerate() {
        // Fixed `GRID_COLUMNS`-wide grid position for the "Grid" layout
        // mode; ignored by the other layout modes. Computed here so
        // `app.slint` never needs modulo/division inside markup.
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        let (grid_row, grid_col) = ((i as i32) / GRID_COLUMNS, (i as i32) % GRID_COLUMNS);

        let mut series: Vec<SeriesData> = Vec::new();
        let mut any_data = false;
        // One color per (channel, lap-range) combination generated,
        // sequential in that order — for the common single-channel case
        // this is exactly the old per-lap coloring; overlaying channels
        // just extends the same sequence.
        let mut color_index = 0usize;

        for name in group {
            let Some(channel) = session.channels.get(name) else {
                continue;
            };

            if let Some(plotted) = plotted.as_mut() {
                plotted.insert(
                    name.clone(),
                    PlottedChannel {
                        timecodes: channel.timecodes.clone(),
                        values: channel.values.clone(),
                        interpolate: channel.interpolate,
                    },
                );
            }

            if let Some(plot) =
                graph::build_lap_comparison_plot(channel, VIEW_WIDTH, VIEW_HEIGHT, &ranges, time_span)
            {
                any_data = true;
                // Only label traces when the dock overlays more than one
                // channel — with a single channel the dock header already
                // names it, and lap comparison already has its own
                // top-level legend (see `DockPanel`'s per-series legend).
                let label = if group.len() > 1 { channel.name.clone() } else { String::new() };
                for s in plot.series {
                    series.push(SeriesData {
                        commands: s.commands.into(),
                        color: series_color(color_index),
                        label: label.clone().into(),
                    });
                    color_index += 1;
                }
            }
        }

        let channel_name = group.join(" + ");
        let channel_units = match group.as_slice() {
            [only] => session
                .channels
                .get(only)
                .map(|c| c.units.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };

        if any_data {
            docks.push(DockData {
                channel_name: channel_name.into(),
                channel_units: channel_units.into(),
                series: slint::ModelRc::new(slint::VecModel::from(series)),
                // `VIEW_WIDTH`/`VIEW_HEIGHT` (1000.0) are small, exactly
                // f32-representable UI coordinates, so this narrowing
                // cast never actually loses precision in practice.
                #[allow(clippy::cast_possible_truncation)]
                view_width: VIEW_WIDTH as f32,
                #[allow(clippy::cast_possible_truncation)]
                view_height: VIEW_HEIGHT as f32,
                has_data: true,
                status_text: String::new().into(),
                grid_row,
                grid_col,
            });
        } else {
            #[allow(clippy::cast_possible_truncation)]
            docks.push(DockData {
                channel_name: channel_name.into(),
                channel_units: channel_units.into(),
                series: slint::ModelRc::new(slint::VecModel::from(Vec::<SeriesData>::new())),
                view_width: VIEW_WIDTH as f32,
                view_height: VIEW_HEIGHT as f32,
                has_data: false,
                status_text: "No samples in this range.".into(),
                grid_row,
                grid_col,
            });
        }
    }

    if let Some(plotted) = plotted {
        state.plotted = plotted;
        state.plotted_key = Some((state.session_generation, state.dock_channels.clone()));
    }
    window.set_docks(to_dock_model(docks));
    window.set_cursor_visible(false);
}
