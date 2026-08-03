//! Slint GUI shell: load a MoTeC `.ld`, iRacing `.ibt`, or `shtep` `.tsv` file, search/pick channels to add
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
#![allow(clippy::doc_markdown, clippy::suboptimal_flops, clippy::too_many_lines)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use slint::ComponentHandle;

use sde_app::{graph, replay_check, setup_view};

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
    /// Resolved once the user points the app at an RBR/RSF install root
    /// (see `on_set_install_root`). Used to default the "Open file..."/
    /// "Open replay info..." dialogs to the right folders; `None` means no
    /// root has been set yet, so those dialogs just open wherever the OS
    /// defaults to.
    install_paths: Option<sde_rbr::InstallPaths>,
    /// The replay `.rpl`/`.ini` sidecar describing the currently loaded
    /// telemetry file, if any — either picked manually (see
    /// `on_open_replay_info`) or auto-matched by modification time against
    /// `install_paths.replays_dir` when a new file loads (see `load_file`,
    /// `sde_rbr::find_matching_replay_ini`). Reset on every *successful*
    /// new file load (whether or not a match was found — a previous
    /// file's replay info would otherwise silently describe the wrong
    /// run) but preserved across a *failed* load, same as `install_paths`
    /// — see `load_file`'s failure-path comment.
    replay_info: Option<sde_rbr::ReplayInfo>,
    /// How far apart (in time) the auto-matched replay's modification time
    /// was from the loaded telemetry file's, for display in
    /// `refresh_replay_status` — `None` when `replay_info` came from a
    /// manual "Open replay info..." pick instead, or wasn't auto-matched
    /// at all.
    replay_auto_match_gap: Option<std::time::Duration>,
    /// The `.lsp` setup sheet the loaded run used — auto-resolved from
    /// `replay_info`'s `SetupName` against `install_paths` on every load
    /// (see `refresh_setup_panel`), or picked manually via "Open
    /// setup...". Reset alongside `replay_info` for the same reason: a
    /// previous run's setup would silently describe the wrong car.
    setup: Option<sde_setup::Setup>,
    /// A second setup picked via "Compare...", diffed against `setup`.
    /// `None` means the panel shows the single sheet instead.
    setup_compare: Option<sde_setup::Setup>,
    all_channel_names: Vec<String>,
    filter_text: String,
    /// Worksheet docks, in display order. Each dock overlays one or more
    /// channels (see `overlay_pending`/`channel_overlay_toggled` for how
    /// a multi-channel dock gets created) — usually just one.
    dock_channels: Vec<Vec<String>>,
    /// Channels queued (via Ctrl+click in the sidebar) to become one new
    /// overlay dock once "Add overlay dock" is clicked; cleared after.
    overlay_pending: Vec<String>,
    /// The dock whose header is currently being dragged, and the dock a
    /// release would act on (`graph::drag_drop_target`'s answer for the
    /// latest drag offset). Both index `dock_channels`; both `None` when
    /// no drag is in progress or the drag is over no other dock.
    dock_drag_source: Option<usize>,
    dock_drag_target: Option<usize>,
    /// Whether Ctrl was held as of the last movement of the current drag,
    /// which picks between the two things a drop can do: merge the source
    /// into the target as one overlay dock (held) or move the source to
    /// the target's position (not held). Sampled per movement rather than
    /// read at release so the highlight the user is looking at and the
    /// action they get are always the same thing.
    dock_drag_merges: bool,
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
    /// The in-progress scroll gesture: which axis (zoom vs. pan) it has
    /// locked to, and any movement accumulated while that was still
    /// undecided. See `graph::ScrollGesture` for why both the lock and
    /// the accumulation are needed on a trackpad.
    scroll_gesture: graph::ScrollGesture,
    /// When the last scroll event arrived, so a quiet period can end the
    /// current gesture (see `SCROLL_GESTURE_TIMEOUT`). Kept next to
    /// `scroll_gesture` rather than inside it so the gesture logic itself
    /// stays pure and unit-testable, with no clock in it.
    scroll_last_event: Option<std::time::Instant>,
    /// Time (default) or distance x-axis — see `graph::AxisMode`. Reset to
    /// `Time` on every file load; toggled independently of lap
    /// selection/zoom, both of which stay time-based regardless.
    axis_mode: graph::AxisMode,
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
/// `DockData.grid-col`/`grid-row`). Derived from `graph`'s copy rather
/// than spelled out twice, since `graph::drag_drop_target` has to assume
/// the exact same grid pitch to work out drop targets in that layout.
#[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
const GRID_COLUMNS: i32 = graph::DOCK_GRID_COLUMNS as i32;

/// Where the install-root config file lives: `%APPDATA%\sde-app\` on
/// Windows (this app's only target platform today — see `Cargo.toml`).
/// `None` if `%APPDATA%` isn't set, in which case the install root simply
/// isn't persisted rather than the app failing to start.
fn config_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| std::path::PathBuf::from(appdata).join("sde-app"))
}

/// Resolve `root` into [`sde_rbr::InstallPaths`], validate it, read the
/// `[NGP]` recording settings, and push all three into `state`/the window
/// as `install_status_text`. Shared by the "RBR install root..." button
/// (a freshly picked root) and startup (a root loaded from the persisted
/// config file via `config_dir`/`sde_app::config::load_install_root`) so
/// both paths report identically.
fn apply_install_root(window: &AppWindow, state: &Rc<RefCell<AppState>>, root: std::path::PathBuf) {
    let install_paths = sde_rbr::InstallConfig::new(root).resolve();
    let report = sde_rbr::validate(&install_paths);
    let ngp_settings = sde_rbr::read_ngp_settings(&install_paths).ok();

    let mut status = if report.root_looks_valid() {
        format!("RBR install root: {}", install_paths.root.display())
    } else {
        format!(
            "Warning: {} doesn't look like an RBR install (missing RichardBurnsRally.ini and/or Plugins\\NGP\\Telemetry.ini).",
            install_paths.root.display()
        )
    };
    if !report.missing.is_empty() {
        status.push_str(&format!(
            " {} expected path(s) not found.",
            report.missing.len()
        ));
    }
    match ngp_settings.and_then(|s| s.telemetry_recording) {
        Some(true) => status.push_str(" Telemetry recording: ON."),
        Some(false) => status.push_str(" Telemetry recording: OFF."),
        None => {}
    }

    state.borrow_mut().install_paths = Some(install_paths);
    window.set_install_status_text(status.into());
}

fn main() -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    let state: Rc<RefCell<AppState>> = Rc::new(RefCell::new(AppState::default()));

    if let Some(root) = config_dir().and_then(|dir| sde_app::config::load_install_root(&dir)) {
        apply_install_root(&window, &state, root);
    }

    // So the (hidden by default) setup panel opens onto its empty-state
    // guidance rather than a blank column before any file is loaded.
    refresh_setup_panel(&window, &state);

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_open_file(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };

            let mut dialog = rfd::FileDialog::new()
                .add_filter("Telemetry log", &["ld", "ibt", "tsv"])
                .add_filter("MoTeC log", &["ld"])
                .add_filter("iRacing telemetry", &["ibt"])
                .add_filter("shtep TSV export", &["tsv"])
                .set_title("Open a .ld, .ibt, or .tsv telemetry log");
            if let Some(dir) = state
                .borrow()
                .install_paths
                .as_ref()
                .map(|p| p.ngp_telemetry_dir.clone())
            {
                dialog = dialog.set_directory(dir);
            }
            let Some(path) = dialog.pick_file() else {
                return; // user cancelled
            };

            load_file(&window, &state, &path);
            refresh_replay_status(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_set_install_root(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let Some(root) = rfd::FileDialog::new()
                .set_title("Select the RBR install root (e.g. C:\\Richard Burns Rally)")
                .pick_folder()
            else {
                return; // user cancelled
            };

            apply_install_root(&window, &state, root.clone());

            // Best-effort: failing to persist just means this root has to
            // be re-picked next launch, not that it stops working now.
            if let Some(dir) = config_dir() {
                let _ = sde_app::config::save_install_root(&dir, &root);
            }
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_open_replay_info(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };

            let mut dialog = rfd::FileDialog::new()
                .add_filter("Replay metadata", &["ini"])
                .set_title("Open a replay metadata (.ini) sidecar");
            if let Some(dir) = state
                .borrow()
                .install_paths
                .as_ref()
                .map(|p| p.replays_dir.clone())
            {
                dialog = dialog.set_directory(dir);
            }
            let Some(path) = dialog.pick_file() else {
                return; // user cancelled
            };

            match sde_rbr::parse_replay_ini(&path) {
                Ok(replay) => {
                    let mut state_mut = state.borrow_mut();
                    state_mut.replay_info = Some(replay);
                    // A manual pick, not an auto-match.
                    state_mut.replay_auto_match_gap = None;
                }
                Err(e) => {
                    window.set_replay_status_text(format!("Error loading replay info: {e}").into());
                    return;
                }
            }
            refresh_replay_status(&window, &state);

            // A different replay means a different run, so its setup takes
            // over — but only if one actually resolves. Failing to find it
            // leaves whatever the panel had (possibly a manually opened
            // sheet), rather than clearing the panel as a side effect of
            // picking a replay.
            if let Some(setup) = auto_resolve_setup(&state) {
                let mut state_mut = state.borrow_mut();
                state_mut.setup = Some(setup);
                state_mut.setup_compare = None;
            }
            refresh_setup_panel(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_open_setup(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let Some(setup) = pick_setup(&state, "Open a car setup (.lsp)") else {
                return;
            };
            {
                let mut state_mut = state.borrow_mut();
                state_mut.setup = Some(setup);
                // A newly picked *primary* setup invalidates any active
                // comparison — the pair the user set up was between two
                // specific sheets, and silently re-pointing one half of it
                // would show a diff they never asked for.
                state_mut.setup_compare = None;
            }
            refresh_setup_panel(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_compare_setup(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let Some(setup) = pick_setup(&state, "Compare against a car setup (.lsp)") else {
                return;
            };
            state.borrow_mut().setup_compare = Some(setup);
            refresh_setup_panel(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_setup_comparison_cleared(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            state.borrow_mut().setup_compare = None;
            refresh_setup_panel(&window, &state);
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

    // -- header drag-and-drop: reorder docks, or merge them with Ctrl --
    //
    // The three handlers are deliberately thin: all the geometry lives in
    // `graph::drag_drop_target` and all the list surgery in
    // `graph::reorder_docks`/`graph::merge_docks`, all pure and unit
    // tested. The same drag geometry feeds both actions, which is what
    // lets a modifier pick between them (see the interaction notes under
    // milestone 5 in PROJECT_PLAN.md).
    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_dock_drag_started(move |index| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let mut state_mut = state.borrow_mut();
            state_mut.dock_drag_source = usize::try_from(index)
                .ok()
                .filter(|i| *i < state_mut.dock_channels.len());
            state_mut.dock_drag_target = None;
            state_mut.dock_drag_merges = false;
            drop(state_mut);
            refresh_dock_drag_ui(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_dock_drag_moved(move |index, dx, dy, cell_width, cell_height, ctrl_held| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let layout = graph::DockLayout::from_index(window.get_layout_mode());
            let mut state_mut = state.borrow_mut();
            // A `moved` can arrive without a preceding `down` having been
            // recorded (a press that started before the dock existed);
            // trust the index the event carries either way.
            let Ok(source) = usize::try_from(index) else {
                return;
            };
            state_mut.dock_drag_source = Some(source);
            state_mut.dock_drag_target = graph::drag_drop_target(
                layout,
                source,
                state_mut.dock_channels.len(),
                f64::from(dx),
                f64::from(dy),
                f64::from(cell_width),
                f64::from(cell_height),
            );
            state_mut.dock_drag_merges = ctrl_held;
            drop(state_mut);
            refresh_dock_drag_ui(&window, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = state.clone();
        window.on_dock_drag_ended(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let mut state_mut = state.borrow_mut();
            let changed = match (state_mut.dock_drag_source, state_mut.dock_drag_target) {
                (Some(source), Some(target)) => {
                    if state_mut.dock_drag_merges {
                        graph::merge_docks(&mut state_mut.dock_channels, source, target)
                    } else {
                        graph::reorder_docks(&mut state_mut.dock_channels, source, target)
                    }
                }
                // A press with no drag (or a drag that ended over nothing)
                // just ends, leaving the worksheet as it was.
                _ => false,
            };
            state_mut.dock_drag_source = None;
            state_mut.dock_drag_target = None;
            state_mut.dock_drag_merges = false;
            drop(state_mut);
            refresh_dock_drag_ui(&window, &state);
            if changed {
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

                // Scroll events carry no gesture start/end markers, so a
                // quiet period is what ends one — after which the next
                // event starts a fresh gesture free to lock to a different
                // axis.
                let now = std::time::Instant::now();
                if state_mut
                    .scroll_last_event
                    .is_none_or(|last| now.duration_since(last) >= SCROLL_GESTURE_TIMEOUT)
                {
                    state_mut.scroll_gesture.reset();
                }
                state_mut.scroll_last_event = Some(now);

                // The gesture zeroes the off-axis delta entirely, rather
                // than just picking which effect to apply — trackpad
                // jitter on the "wrong" axis shouldn't leak through even
                // partially — and returns (0, 0) until it has seen enough
                // movement to know which axis the user means.
                let (delta_x, delta_y) = state_mut.scroll_gesture.feed(delta_x, delta_y);
                if delta_x == 0.0 && delta_y == 0.0 {
                    return;
                }

                let current = state_mut.zoom.unwrap_or((0.0, 1.0));
                let updated = graph::zoom_scroll(current, delta_x, delta_y, f64::from(fraction));
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
        window.on_axis_mode_toggled(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            {
                let mut state_mut = state.borrow_mut();
                state_mut.axis_mode = match state_mut.axis_mode {
                    graph::AxisMode::Time => graph::AxisMode::Distance,
                    graph::AxisMode::Distance => graph::AxisMode::Time,
                };
            }
            refresh_axis_ui(&window, &state);
            replot(&window, &state);
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
                window.set_math_channel_error(
                    format!("A channel named \"{name}\" already exists.").into(),
                );
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

            let distance_channel = session
                .key_channel_map
                .distance
                .as_deref()
                .and_then(|name| session.channels.get(name));
            let abs_times = cursor_abs_times(
                &ranges,
                time_span,
                f64::from(fraction),
                state.axis_mode,
                distance_channel,
            );

            let cursor_values: Vec<slint::SharedString> = state
                .dock_channels
                .iter()
                .map(|group| cursor_text_for_group(&state, group, &abs_times).into())
                .collect();

            let readout = match (state.axis_mode, distance_channel) {
                (graph::AxisMode::Distance, Some(dist_ch)) => {
                    let axis_span = graph::distance_axis_span(dist_ch, &ranges);
                    format!("d = {:.1} m", f64::from(fraction) * axis_span)
                }
                _ => format!("t = {:.1} ms", f64::from(fraction) * time_span),
            };

            window.set_cursor_visible(true);
            window.set_cursor_fraction(fraction);
            window.set_cursor_time_text(readout.into());
            window.set_cursor_values(slint::ModelRc::new(slint::VecModel::from(cursor_values)));
        });
    }

    window.run()
}

/// The cursor readout text for one dock: each channel in `group` gets its
/// pipe-separated per-range values (`"12.3 | 15.0"` when comparing laps),
/// one absolute sample time per entry in `abs_times` (parallel to
/// `ranges`/whatever range list produced them — see [`cursor_abs_times`]);
/// a single-channel dock keeps that bare (matching the original format),
/// while a multi-channel overlay dock prefixes each with its channel name
/// (`"BRAKE: 80.0   THROTTLE: 0.0"`) so the values stay identifiable once
/// more than one channel shares the graph.
fn cursor_text_for_group(state: &AppState, group: &[String], abs_times: &[f64]) -> String {
    let parts: Vec<String> = group
        .iter()
        .map(|name| {
            let values = state.plotted.get(name).map_or_else(String::new, |c| {
                abs_times
                    .iter()
                    .filter_map(|&abs_t| {
                        graph::value_at_raw(&c.timecodes, &c.values, c.interpolate, abs_t)
                            .map(|v| format!("{v:.3}"))
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            });
            let values = if values.is_empty() {
                "n/a".to_string()
            } else {
                values
            };
            if group.len() == 1 {
                values
            } else {
                format!("{name}: {values}")
            }
        })
        .collect();
    parts.join("   ")
}

/// The absolute sample time (one per entry in `ranges`) the cursor
/// currently points at, for a cursor drag at `fraction` (0..1 across the
/// dock's plotted width). In `Time` mode this is the same offset
/// (`fraction * time_span`) applied to every range, since the x-axis is
/// linear in time; in `Distance` mode each range gets its own absolute
/// time, found by inverting `distance_channel` (see
/// `graph::time_at_distance`) — necessary because two compared runs at
/// the same distance along the stage generally aren't at the same
/// elapsed time, which is the entire point of a distance-based
/// comparison. Falls back to `Time`-mode behavior (and `None` for a range
/// distance lookup fails) whenever no distance channel is available,
/// matching `build_lap_comparison_plot`'s own fallback.
fn cursor_abs_times(
    ranges: &[(f64, f64)],
    time_span: f64,
    fraction: f64,
    axis_mode: graph::AxisMode,
    distance_channel: Option<&sde_core::Channel>,
) -> Vec<f64> {
    match (axis_mode, distance_channel) {
        (graph::AxisMode::Distance, Some(dist_ch)) => {
            let axis_span = graph::distance_axis_span(dist_ch, ranges);
            ranges
                .iter()
                .filter_map(|&(start, end)| {
                    let start_dist = graph::value_at(dist_ch, start)?;
                    let target_dist = start_dist + fraction * axis_span;
                    let abs_t = graph::time_at_distance(dist_ch, start, end, target_dist)?;
                    Some(abs_t.clamp(start, end))
                })
                .collect()
        }
        _ => {
            let t_rel = fraction * time_span;
            ranges
                .iter()
                .map(|&(start, end)| (start + t_rel).min(end))
                .collect()
        }
    }
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

/// Push the in-progress dock drag (which dock is being dragged, which one
/// a release would act on, and whether that action is a merge) into the
/// window, so the markup can fade the source and highlight the target in
/// the style matching what a release would do. `-1` for "none", since
/// Slint has no optional int.
fn refresh_dock_drag_ui(window: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();
    let as_index = |slot: Option<usize>| slot.and_then(|i| i32::try_from(i).ok()).unwrap_or(-1);
    window.set_dock_drag_source(as_index(state.dock_drag_source));
    window.set_dock_drag_target(as_index(state.dock_drag_target));
    window.set_dock_drag_merges(state.dock_drag_merges);
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

/// Push the current axis-mode label/availability into the window. Called
/// after loading a file and whenever the axis toggle is clicked.
///
/// `axis-distance-available` drives `app.slint`'s toggle: distance mode
/// can be selected regardless (falls back to time transparently — see
/// `graph::build_lap_comparison_plot`), but greying out an unavailable
/// toggle communicates *why* the axis didn't visibly change, rather than
/// silently doing nothing.
fn refresh_axis_ui(window: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();
    let distance_available = state
        .session
        .as_ref()
        .is_some_and(|s| s.key_channel_map.distance.is_some());
    window.set_axis_mode_label(
        match state.axis_mode {
            graph::AxisMode::Time => "Time",
            graph::AxisMode::Distance => "Distance",
        }
        .into(),
    );
    window.set_axis_distance_available(distance_available);
}

/// Best-effort last path segment of a Windows-style path string. RSF's own
/// `.ini` values (e.g. `car.setup_name`) always use `\` regardless of the
/// host OS, so this can't just be `Path::file_name`. Falls back to the
/// whole string if there's no separator.
fn path_basename(value: &str) -> &str {
    value.rsplit(['\\', '/']).next().unwrap_or(value)
}

/// Push a summary of the currently loaded replay `.rpl`/`.ini` sidecar
/// into the window — stage, car, setup used, surface/weather conditions,
/// NGP physics version, and driving time, plus a recovery-spot cross-check
/// against the currently loaded telemetry session if one is loaded too
/// (see `replay_check::cross_check_recoveries`). Called after loading a
/// replay `.ini` (manually, or auto-matched by `load_file`) and after
/// loading a telemetry file (since either one changing affects
/// whether/what cross-check runs).
///
/// These particular fields were picked because they're what decides
/// whether two runs are actually comparable (setup, conditions, physics
/// version) or just look like they are (same stage/car) — see
/// `PROJECT_PLAN.md`'s "UI/UX direction" design note, principle 6 (curated
/// metadata over a flat property dump) and principle 8 (setup-diff-aware
/// comparison, not yet built, but this is the raw material for it).
/// `ReplayInfo` has more fields than are shown here (map length, rally
/// type/name, sky type, surface age, RSF version, ...) — left out for now
/// as less immediately actionable than what's here; nothing stops adding
/// them later, `ReplayInfo::extra` isn't needed since these are all
/// already-modeled fields.
fn refresh_replay_status(window: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();
    let Some(replay) = state.replay_info.as_ref() else {
        window.set_replay_status_text(String::new().into());
        return;
    };

    let mut parts = Vec::new();
    if let Some(name) = &replay.stage.name {
        parts.push(format!("Stage: {name}"));
    }
    if let Some(model) = &replay.car.model {
        parts.push(format!("Car: {model}"));
    }
    if let Some(setup) = &replay.car.setup_name {
        parts.push(format!("Setup: {}", path_basename(setup)));
    }
    let conditions: Vec<&str> = [
        replay.conditions.tyre_type.as_deref(),
        replay.conditions.weather_type.as_deref(),
        replay.conditions.surface_wetness.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !conditions.is_empty() {
        parts.push(format!("Conditions: {}", conditions.join("/")));
    }
    if let Some(ngp) = &replay.versions.ngp {
        parts.push(format!("NGP {ngp}"));
    }
    if let Some(driving_time) = replay.driving_time_secs() {
        parts.push(format!("Driving time: {driving_time:.1}s"));
    }

    // Distinguishes an auto-matched pairing (a heuristic — see
    // `sde_rbr::find_matching_replay_ini`) from a manual "Open replay
    // info..." pick, and shows how tight the match was so the user can
    // judge how much to trust it.
    let prefix = state.replay_auto_match_gap.map_or_else(
        || "Replay".to_string(),
        |gap| format!("Replay (auto-matched, Δ{}s)", gap.as_secs()),
    );
    let mut text = format!("{prefix}: {}", parts.join(" | "));

    if let Some(session) = state.session.as_ref() {
        let check = replay_check::cross_check_recoveries(session, replay);
        let agreement = if check.looks_consistent() {
            "consistent"
        } else {
            "MISMATCH"
        };
        text.push_str(&format!(
            " — recoveries: replay {} / telemetry {} ({agreement})",
            check.replay_count, check.telemetry_count
        ));
    }

    window.set_replay_status_text(text.into());
}

/// Show a file picker for a `.lsp` setup and load it, defaulting to the
/// install's `SavedGames\` folder when a root is set. `None` if the user
/// cancelled *or* the file didn't load — a bad pick leaves the panel
/// showing whatever it already had rather than blanking it.
fn pick_setup(state: &Rc<RefCell<AppState>>, title: &str) -> Option<sde_setup::Setup> {
    let mut dialog = rfd::FileDialog::new()
        .add_filter("RBR car setup", &["lsp"])
        .set_title(title);
    if let Some(dir) = state
        .borrow()
        .install_paths
        .as_ref()
        .map(|p| p.saved_games_dir.clone())
    {
        dialog = dialog.set_directory(dir);
    }
    let path = dialog.pick_file()?;
    sde_setup::rbr::load_lsp(&path).ok()
}

/// Push the setup panel's contents into the window: the whole sheet, or
/// only what differs once a comparison setup is picked.
///
/// Per PROJECT_PLAN.md's UI/UX design note, principle 8 — a performance
/// comparison next to *what actually changed between the two setups* is
/// the differentiated feature here, so the diff view is the one the panel
/// switches to as soon as there's a second setup to show.
fn refresh_setup_panel(window: &AppWindow, state: &Rc<RefCell<AppState>>) {
    let state = state.borrow();

    let (title, subtitle, rows, comparing) = match (&state.setup, &state.setup_compare) {
        (Some(left), Some(right)) => {
            let diff = sde_setup::diff(left, right);
            let subtitle = if diff.is_empty() {
                "Identical — no values differ.".to_string()
            } else {
                format!("{} values differ", diff.change_count())
            };
            (
                format!("{} → {}", left.name, right.name),
                subtitle,
                setup_view::rows_for_diff(&diff),
                true,
            )
        }
        (Some(setup), None) => (
            setup.name.clone(),
            format!(
                "{} values in {} groups — {}",
                setup.entry_count(),
                setup.groups.len(),
                setup.source
            ),
            setup_view::rows_for_setup(setup),
            false,
        ),
        (None, _) => (
            "Setup".to_string(),
            "No setup loaded. Set an RBR install root and load a run to \
             resolve its setup automatically, or open one directly."
                .to_string(),
            Vec::new(),
            false,
        ),
    };

    window.set_setup_panel_title(title.into());
    window.set_setup_panel_subtitle(subtitle.into());
    window.set_setup_comparing(comparing);
    window.set_setup_rows(slint::ModelRc::new(slint::VecModel::from(
        rows.into_iter()
            .map(|row| SetupRowData {
                is_group: row.is_group,
                label: row.label.into(),
                value: row.value.into(),
                detail: row.detail.into(),
            })
            .collect::<Vec<_>>(),
    )));
}

/// Resolve the `.lsp` the currently loaded replay names, if any. Needs
/// both an install root (to resolve RSF's install-relative `SetupName`)
/// and a loaded replay `.ini`; a missing or unreadable file just means no
/// setup, not an error worth surfacing over the telemetry itself.
fn auto_resolve_setup(state: &Rc<RefCell<AppState>>) -> Option<sde_setup::Setup> {
    let state = state.borrow();
    let paths = state.install_paths.as_ref()?;
    let setup_name = state.replay_info.as_ref()?.car.setup_name.as_ref()?;
    let path = setup_view::resolve_setup_path(paths, setup_name)?;
    let mut setup = sde_setup::rbr::load_lsp(&path).ok()?;
    // The `.lsp` itself carries no car identity (it's implied by the
    // folder it lives in) — the replay does, so fill it in here where
    // both are in hand.
    setup.car.clone_from(&state.replay_info.as_ref()?.car.model);
    Some(setup)
}

fn load_file(window: &AppWindow, state: &Rc<RefCell<AppState>>, path: &Path) {
    let file_name = path.file_name().map_or_else(
        || path.to_string_lossy().to_string(),
        |n| n.to_string_lossy().to_string(),
    );

    // Dispatch on extension, matching `sde-cli`'s `dump_channels` — all
    // format loaders return their own error type, so map to `String`
    // early to keep this match's arms the same type.
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase());
    let load_result = match ext.as_deref() {
        Some("ibt") => sde_core::Session::load_ibt(path).map_err(|e| e.to_string()),
        Some("tsv") => sde_core::Session::load_shtep(path).map_err(|e| e.to_string()),
        _ => sde_core::Session::load_motec(path).map_err(|e| e.to_string()),
    };

    let session = match load_result {
        Ok(s) => s,
        Err(e) => {
            window.set_window_title(format!("sde-app — failed to load {file_name}").into());
            window.set_status_text(format!("Error loading {file_name}: {e}").into());
            {
                // Preserve the install root / loaded replay info across a
                // failed load — a bad file pick shouldn't cost the user a
                // config they already set up.
                let mut state_mut = state.borrow_mut();
                let install_paths = state_mut.install_paths.take();
                let replay_info = state_mut.replay_info.take();
                let replay_auto_match_gap = state_mut.replay_auto_match_gap.take();
                // The setup belongs to the replay, so it's preserved on
                // exactly the same terms.
                let setup = state_mut.setup.take();
                let setup_compare = state_mut.setup_compare.take();
                *state_mut = AppState {
                    install_paths,
                    replay_info,
                    replay_auto_match_gap,
                    setup,
                    setup_compare,
                    ..AppState::default()
                };
            }
            window.set_docks(to_dock_model(vec![]));
            window.set_channel_names(to_model(vec![]));
            window.set_channel_active(to_bool_model(vec![]));
            window.set_channel_overlay_pending(to_bool_model(vec![]));
            window.set_overlay_pending_names(to_model(vec![]));
            window.set_lap_labels(to_model(vec![]));
            window.set_compare_lap_labels(to_model(vec![]));
            window.set_compare_lap_active(to_bool_model(vec![]));
            window.set_compare_status_text(String::new().into());
            window.set_legend(slint::ModelRc::new(slint::VecModel::from(
                Vec::<LegendEntry>::new(),
            )));
            window.set_math_channel_names(to_model(vec![]));
            window.set_math_channel_error(String::new().into());
            window.set_math_name_text(String::new().into());
            window.set_math_formula_text(String::new().into());
            window.set_zoom_range_text(String::new().into());
            window.set_axis_mode_label("Time".into());
            window.set_axis_distance_available(false);
            return;
        }
    };

    let all_channel_names = graph::channel_names(&session);
    let lap_labels = graph::lap_labels(&session);
    let compare_lap_labels: Vec<String> = (1..=session.laps.len()).map(|n| n.to_string()).collect();
    let default_dock_channels = graph::default_dock_channels(&session);

    // Best-effort auto-pair with the replay `.ini` describing this same
    // run (see `sde_rbr::find_matching_replay_ini` — matched by
    // modification-time proximity, since RSF/NGP has no shared filename
    // convention between telemetry and replay files). Computed before the
    // state borrow below so this immutable borrow doesn't overlap with
    // the mutable one that follows. `None` (no install root set, or no
    // `.ini` within tolerance) just means the worksheet loads without
    // replay context, not an error.
    let auto_replay = state
        .borrow()
        .install_paths
        .as_ref()
        .and_then(|paths| sde_rbr::find_matching_replay_ini(path, &paths.replays_dir))
        .and_then(|(ini_path, gap)| {
            sde_rbr::parse_replay_ini(&ini_path)
                .ok()
                .map(|info| (info, gap))
        });

    {
        let mut state = state.borrow_mut();
        state.session = Some(session);
        state.all_channel_names.clone_from(&all_channel_names);
        state.filter_text.clear();
        state.dock_channels = default_dock_channels;
        state.overlay_pending.clear();
        state.selected_lap_index = 0;
        state.compare_lap_indices.clear();
        state.math_channel_names.clear();
        state.zoom = None;
        // Replay info describes exactly one telemetry file, so it's reset
        // on every new (successful) load — see the field's doc comment —
        // then repopulated here if auto-pairing found a match.
        state.replay_auto_match_gap = auto_replay.as_ref().map(|(_, gap)| *gap);
        state.replay_info = auto_replay.map(|(info, _)| info);
        state.axis_mode = graph::AxisMode::Time;
        state.session_generation += 1;
    }

    // Resolve this run's setup sheet from the replay info just loaded.
    // Separate borrow because `auto_resolve_setup` reads the state it
    // depends on (the borrow above is still held at that point).
    let auto_setup = auto_resolve_setup(state);
    {
        let mut state = state.borrow_mut();
        state.setup = auto_setup;
        // A comparison pairs two specific sheets; the left one just
        // changed, so the pairing no longer means what the user set up.
        state.setup_compare = None;
    }

    window.set_window_title(format!("sde-app — {file_name}").into());
    window
        .set_status_text("No channels added to the worksheet yet — click one on the left.".into());
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
    refresh_axis_ui(window, state);
    refresh_setup_panel(window, state);
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

    let distance_channel = session
        .key_channel_map
        .distance
        .as_deref()
        .and_then(|name| session.channels.get(name));

    // `plotted` only depends on `session.channels` and `dock_channels`
    // (which channels are plotted), not on `ranges`/`time_span` (the
    // zoom/lap window) — so a `replot` triggered purely by a zoom/pan
    // scroll event (see `on_zoom_scrolled`) can reuse the existing cache
    // instead of re-cloning every plotted channel's full sample arrays,
    // which matters since those scroll unboundedly fast on a trackpad.
    let rebuild_plotted = state.plotted_key.as_ref()
        != Some(&(state.session_generation, state.dock_channels.clone()));
    let mut plotted = rebuild_plotted.then(HashMap::new);

    let mut docks = Vec::with_capacity(state.dock_channels.len());

    // Comparing laps gives color a *different* meaning (which lap, shared
    // across every dock via `refresh_compare_ui`'s own top-level legend —
    // see `series_color(i)` there, indexed by position in
    // `compare_lap_indices`) than the plain case (which channel). Mixing
    // the two would desync a dock's trace color from what the shared
    // legend says it means, so only the plain case rotates a color per
    // dock/channel here; comparison mode keeps starting each dock fresh at
    // color 0 to line up with that legend.
    //
    // Deliberately keyed on `compare_lap_indices` being non-empty, *not*
    // `ranges.len() > 1`: a single lap can be "compared" (the compare-chip
    // UI has nothing stopping a user from toggling on just one, and
    // `refresh_compare_ui`'s status text already special-cases "Comparing
    // 1 lap"), which yields exactly one range — `ranges.len() > 1` would
    // read that as the plain case and let the color rotation advance past
    // dock 0, desyncing every dock after the first from the legend's
    // single swatch (which always starts at color 0 for that one lap).
    let comparing = !state.compare_lap_indices.is_empty();
    // Oscilloscope-style: each successive dock/channel gets the next
    // color in the palette rather than every single-channel dock
    // restarting at color 0 (which made every dock the same blue). Only
    // advances outside comparison mode — see above.
    let mut worksheet_color_index = 0usize;

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
        // just extends the same sequence. Starts from the running
        // worksheet-wide index (plain mode) or fresh at 0 (comparison
        // mode, to match the shared legend) — see above.
        let mut color_index = if comparing { 0 } else { worksheet_color_index };

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

            if let Some(plot) = graph::build_lap_comparison_plot(
                channel,
                VIEW_WIDTH,
                VIEW_HEIGHT,
                &ranges,
                time_span,
                state.axis_mode,
                distance_channel,
            ) {
                any_data = true;
                // Only label traces when the dock overlays more than one
                // channel — with a single channel the dock header already
                // names it, and lap comparison already has its own
                // top-level legend (see `DockPanel`'s per-series legend).
                let label = if group.len() > 1 {
                    channel.name.clone()
                } else {
                    String::new()
                };
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

        if !comparing {
            worksheet_color_index = color_index;
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
