//! Milestone 3 minimal Slint GUI shell: load a MoTeC `.ld` file, plot one
//! (hardcoded-selection) channel, and drag a vertical time cursor over
//! it. See PROJECT_PLAN.md for scope; see `graph.rs` for the pure
//! plotting/lookup logic this file wires up to the UI.

// clippy::pedantic/nursery notes (not part of the default lint set the
// project otherwise keeps clean):
// - doc_markdown fires repeatedly on plain-English mentions of
//   `PROJECT_PLAN.md`/MoTeC in prose above; not worth backtick-wrapping
//   every occurrence for a doc-only lint.
// - suboptimal_flops wants `mul_add` for the cursor-fraction lerp below;
//   for this tiny bit of UI-coordinate math, the plain form reads better
//   than the marginal FMA precision/perf gain is worth.
#![allow(clippy::doc_markdown, clippy::suboptimal_flops)]

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use slint::ComponentHandle;

use tda_app::graph;

slint::include_modules!();

/// The bits of the currently-plotted channel the cursor-drag handler
/// needs, kept outside the `Session`/`Channel` so we don't have to clone
/// the whole session into the closure.
struct PlottedChannel {
    timecodes: Vec<f64>,
    values: Vec<f64>,
    interpolate: bool,
    min_time: f64,
    max_time: f64,
}

const VIEW_WIDTH: f64 = 1000.0;
const VIEW_HEIGHT: f64 = 1000.0;

fn main() -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;

    // Shared with the cursor-moved callback so it can look up the value
    // under the cursor without re-touching the whole Session.
    let plotted: Rc<RefCell<Option<PlottedChannel>>> = Rc::new(RefCell::new(None));

    {
        let window_weak = window.as_weak();
        let plotted = plotted.clone();
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

            load_and_display(&window, &plotted, &path);
        });
    }

    {
        let window_weak = window.as_weak();
        // Last use of the outer `plotted` Rc in `main`, so it can just be
        // moved into this closure instead of cloned.
        window.on_cursor_moved(move |fraction| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let Some(chan) = plotted.borrow().as_ref().map(|c| {
                let t = c.min_time + f64::from(fraction) * (c.max_time - c.min_time);
                let value = graph::value_at_raw(&c.timecodes, &c.values, c.interpolate, t);
                (t, value)
            }) else {
                return;
            };
            let (t, value) = chan;
            window.set_cursor_visible(true);
            window.set_cursor_fraction(fraction);
            window.set_cursor_time_text(format!("{t:.1}").into());
            window.set_cursor_value_text(
                value.map_or_else(|| "n/a".into(), |v| format!("{v:.3}").into()),
            );
        });
    }

    window.run()
}

fn load_and_display(
    window: &AppWindow,
    plotted: &Rc<RefCell<Option<PlottedChannel>>>,
    path: &Path,
) {
    let file_name = path.file_name().map_or_else(
        || path.to_string_lossy().to_string(),
        |n| n.to_string_lossy().to_string(),
    );

    let session = match tda_core::Session::load_motec(path) {
        Ok(s) => s,
        Err(e) => {
            window.set_window_title(format!("tda-app — failed to load {file_name}").into());
            window.set_status_text(format!("Error loading {file_name}: {e}").into());
            window.set_has_data(false);
            *plotted.borrow_mut() = None;
            return;
        }
    };

    // Milestone-3 shortcut: no channel-picker UI yet, so just plot a
    // single hardcoded selection (first `interpolate == true` channel
    // alphabetically, falling back to the first channel). A proper
    // channel search/picker is deferred to milestone 5.
    let Some(channel) = graph::pick_default_channel(&session) else {
        window.set_window_title(format!("tda-app — {file_name}").into());
        window.set_status_text("File loaded, but it has no channels to plot.".into());
        window.set_has_data(false);
        *plotted.borrow_mut() = None;
        return;
    };

    let Some(plot) = graph::build_plot(channel, VIEW_WIDTH, VIEW_HEIGHT) else {
        window.set_status_text("Selected channel has no samples.".into());
        return;
    };

    *plotted.borrow_mut() = Some(PlottedChannel {
        timecodes: channel.timecodes.clone(),
        values: channel.values.clone(),
        interpolate: channel.interpolate,
        min_time: plot.min_time,
        max_time: plot.max_time,
    });

    window.set_window_title(format!("tda-app — {file_name}").into());
    window.set_has_data(true);
    window.set_channel_name(channel.name.clone().into());
    window.set_channel_units(channel.units.clone().into());
    window.set_path_commands(plot.commands.into());
    // `plot.view_width`/`view_height` are always `VIEW_WIDTH`/`VIEW_HEIGHT`
    // (1000.0) — small, exactly f32-representable UI coordinates, so this
    // narrowing cast to Slint's f32 property type never actually loses
    // precision in practice.
    #[allow(clippy::cast_possible_truncation)]
    {
        window.set_view_width(plot.view_width as f32);
        window.set_view_height(plot.view_height as f32);
    }
    window.set_cursor_visible(false);
}
