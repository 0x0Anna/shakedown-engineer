# Shakedown Engineer (Project Plan)

## Overview

**Shakedown Engineer** (crate prefix `sde-`) is a rally **race engineer tool**:
creating and analyzing vehicle setups per stage (suspension, ABS/TCS/aero
electronics, not just driver performance), for use with Richard Burns Rally (NGP
physics / rallysimfans), Dirt Rally, Assetto Corsa Rally (limited telemetry), and
EA WRC.

**Primary motivation:** the setup/analysis features (`sde-setup`, `sde-analysis`,
per-sim setup adapters) are rally-specific and are the reason this project exists —
that's where new development effort is prioritized.

**Foundation (general-purpose, not rally-specific):** its telemetry log parsing and
core data model (`sde-formats`, `sde-core`) are a Rust + Slint port of
[racer-coder/TrackDataAnalysis](https://github.com/racer-coder/TrackDataAnalysis), a
PySide6-based racing telemetry viewer (Python/Cython, supports AiM XRK, MoTeC LD,
ECUMaster ADULOG, RaceLogic VBOX, Race Technology RUN, MegaLogViewer MLG, iRacing
IBT). This layer is deliberately kept sim-agnostic and useful for circuit racing too
— it's shared infrastructure, not the differentiator. New parser work here follows
the port faithfully; new *feature* work goes into the rally-focused crates above.

**Secondary goals:**
- Learn Rust more deeply through this project.
- Build a modern, clean UX (Slint UI, not just parity with the dated original).
- Structure the code so core parts (parsers, data model) are modular and reusable
  in other projects, independent of the GUI.

**Explicitly out of scope (for now):**
- Full vehicle dynamics simulation (predictive "what-if" setup modeling). Too large
  a project on its own (this is what tools like ChassisSim / OptimumG do professionally).
- 3D visualization — deferred to a future phase (see below). Only a *replay/animation*
  of logged data was ever in scope, never a physics simulator.
- Godot was considered for the 3D visualizer and explicitly rejected in favor of staying
  native Rust (`three-d` or `wgpu`), to avoid breaking the modular crate architecture and
  avoid a second UI paradigm/process alongside Slint.

## Tech stack decisions

- **Language:** Rust (workspace of multiple crates).
- **GUI:** Slint (chosen over egui/iced for declarative markup separate from logic,
  GPU-accelerated rendering, good custom-drawn component support for graphs/maps).
- **Video sync:** `libmpv-rs` bindings (original used libmpv too — directly reusable concept).
- **Binary parsing:** `binrw` recommended for the fixed-record binary log formats.
- **3D (future, deferred):** `three-d` crate, embedded into Slint via off-screen render
  to texture → `slint::Image`. Not started yet.

## Workspace / crate layout

```
crates/
├── sde-formats/          # binary parsers — no UI/GUI deps, dependency-light
│   ├── motec/            # MoTeC LD parser — FIRST TARGET, in progress
│   ├── xrk/              # AiM
│   ├── ibt/               # iRacing
│   ├── vbox/              # RaceLogic
│   ├── adulog/            # ECUMaster
│   ├── run/                # Race Technology
│   ├── mlg/                # MegaLogViewer
│   ├── rbr/               # RBR/RSF companion files — STARTED (`sde-rbr`).
│   │                      #   ini.rs     — shared minimal INI reader
│   │                      #   replay.rs  — replay metadata .ini sidecar (DONE)
│   │                      #   todo: .lsp setup sheet, pacenote .ini, .rpl frames
│   └── dirt_rally/, acr/, ea_wrc/         # per-sim SETUP FILE adapters
│                                            # (read each sim's own install-dir car/track/
│                                            #  setup files, map into sde-setup's model —
│                                            #  same adapter pattern as telemetry parsers,
│                                            #  but for setup data instead of channel data)
│                                            # NOTE: RBR telemetry itself needs no adapter —
│                                            #  RSF exports MoTeC LD, so sde-motec handles it.
│
├── sde-core/              # Session/Channel/Lap data model, math channel expression
│                          # engine, interpolation/resampling, metadata DB cache (sqlite)
│                          # — depends on sde-formats, UI-free
│
├── sde-setup/             # NEW (race-engineer feature): sim-agnostic setup sheet model —
│                          # springs, dampers, ARBs, ride height, diff, gearing, tire
│                          # pressures/compounds. Versioned and diffable per stage.
│                          # Populated either manually or via sde-formats::<sim> adapters.
│
├── sde-analysis/          # NEW (race-engineer feature): derived-channel analysis —
│                          # damper velocity histograms, ABS/TC intervention markers +
│                          # frequency/duration stats, ride-height/roll estimates from
│                          # suspension travel differentials, brake balance effectiveness.
│                          # This is the "analyze the vehicle, not just the driver" piece.
│
├── sde-gis/               # GPS/map-tile logic, UI-free
├── sde-video/             # libmpv wrapper + video/data sync
│
└── sde-app/                # Slint GUI. ONLY crate allowed to depend on Slint.
                            # Everything else must remain usable standalone
                            # (CLI tools, other projects, WASM builds, etc.)

(future, not started)
└── sde-viz/                # 2D then optional 3D pose animation from logged data
                            # (damper travel, steering angle, yaw rate) — NOT a physics
                            # simulator, just data-driven visualization/replay.
```

### Modularity principles
- `sde-formats` and `sde-core` must stay UI-free and dependency-light so they're
  reusable outside this app (e.g. as a CLI tool or in someone else's project).
- Common `LogFormat` trait for telemetry parsers so new formats plug in without
  touching GUI code:
  ```rust
  pub trait LogFormat {
      fn parse(data: &[u8]) -> Result<RawSession, FormatError>;
      fn detect(data: &[u8]) -> bool; // magic bytes sniffing
  }
  ```
- Same adapter pattern applies to per-sim setup file readers in `sde-formats::<sim>`,
  feeding into `sde-setup`'s shared data model.
- Feature-gate heavier optional deps (e.g. `sqlite` feature for the DB cache).
- Consider publishing `sde-formats` / `sde-core` to crates.io early (even 0.1.0)
  once 1–2 formats work, as a forcing function for a clean public API.

## Reference repos / prior art

- **Original Python tool:** https://github.com/racer-coder/TrackDataAnalysis
  (PySide6, the thing being ported) (Local reference repo: `../TrackDataAnalysis`).
  **This is the PRIMARY oracle for the MoTeC LD format**, not `ldparser` (see below) —
  it ships its own `data/motec.py`, MIT-licensed, 2024, actively maintained, and it's
  literally the parser belonging to the tool this project ports. Its `data/base.py`
  also defines the exact `Channel`/`Lap`/`LogFile` dataclasses to mirror in
  `sde-core`'s Rust data model:
  - `Channel { timecodes: array, values: array, name, units, dec_pts, interpolate }`
    (`interpolate=False` means "hold previous value until next timecode" — needed for
    discrete/event channels vs continuous ones)
  - `Lap { num, start_time, end_time }`
  - `LogFile { channels: Dict[str, Channel], laps: List[Lap], metadata: Dict[str,str],
    key_channel_map: [speed, lat, long, alt], file_name }`
  - Lap splitting is derived from a `Beacon` channel via a small state machine
    (see `MOTEC()` in `data/motec.py`) — worth porting almost verbatim into
    `sde-core`'s session-building step in milestone 2.
- **MoTeC LD format cross-reference:** `gotzl/ldparser` (Python, GPL-3.0) — older,
  reverse-engineered LD parser. Still useful as a second independent implementation to
  diff against, and its `ldData.frompd()/write()` round-trip is a handy way to generate
  synthetic `.ld` fixtures (see Validation findings below). Do **not** treat it as the
  primary oracle — see the conversion-formula discrepancy noted below.
  (Local reference repo: `../ldparser`)
- **RBR setup/domain reference:** `durandom/race-engineer` — currently **unimplemented**
  (README states "Status: Not yet implemented"). Different tech stack (Tauri + SvelteKit,
  not directly reusable). What's useful: the concept of reading RBR's own installation-
  directory JSON/ini files for car/track/setup data, and its
  `docs/data-model-rbr.md` domain model doc, which documents `CarGroup`, `CarModel`,
  `Car`, `CarData`, `CarGroupMap`, `Stage`, `CarPersonalOptions`, `StagePersonalOptions`
  — the field-level shape to target for the `sde-formats::rbr` adapter in milestone 9.
  (Local reference repo: `../race-engineer`)
- **iRacing telemetry reference:** `ethansheffield/iracing-telemetry-tool` (Python,
  MIT) — a *live* SDK capture tool (`pyirsdk` shared-memory polling), not an `.ibt`
  file parser, so its code is **not directly portable** to `sde-formats::ibt`
  (milestone 4). Useful only as a channel/metadata reference:
  - Confirms the iRacing channel set and units to target: `lap`, `time`, `distance`,
    `distance_pct`, `throttle`, `brake`, `steering`, `gear`, `rpm`, `speed` (m/s),
    `lat_accel`/`long_accel` (G), `yaw_rate`, `steering_wheel_angle` (rad).
  - Session metadata shape (`track_name`, `track_config`, `car_name`, `session_type`
    enum 0–4 Testing/Practice/Qualifying/Warmup/Race, `driver_name`) mirrors what
    IBT's YAML session-info header carries — cross-check for `sde-core::Session`
    metadata fields once the iRacing adapter lands.
  - Lap splitting is trivial for IBT vs. MoTeC: iRacing's SDK/IBT exposes an
    explicit `Lap` variable directly, so milestone 4 shouldn't need a beacon-style
    state machine for iRacing files.
  (Local reference repo: `../iracing-telemetry-tool`)

### Validation findings (2026-07-20)

Cross-checked `TrackDataAnalysis/data/motec.py` against `ldparser/ldparser.py` on both
code reading and a generated synthetic `.ld` file (see below). Findings:

- **Channel record layout confirmed** (offsets relative to each channel meta record's
  base address `addr`, little-endian):
  - `addr+0`: prev record addr (u32, unused by either parser)
  - `addr+4`: next record addr (u32) — walk this linked list from the file header's
    `channel_meta_addr` to enumerate all channels
  - `addr+8`: `data_addr` (u32) — absolute file offset of this channel's sample data
  - `addr+12`: `data_count` (u32) — number of samples
  - `addr+16`: counter/unknown (u16, skip)
  - `addr+18`: `elem_type` (u16) — `0|3|5` = integer, `7` = float
  - `addr+20`: `elem_size` (u16) — byte width directly (2 or 4); TDA reads this as a
    literal size, not an enum-index lookup table like `ldparser` does (`ldparser`'s
    `[None, np.int16, None, np.int32]` indexing is fragile/harder to port — prefer
    TDA's direct-size approach for the Rust `binrw` parser)
  - `addr+22`: `sample_rate` (u16, Hz)
  - `addr+24`: `offset`/`shift` (i16)
  - `addr+26`: `mul` (i16)
  - `addr+28`: `scale` (i16)
  - `addr+30`: `dec_pts` (i16)
  - `addr+32`: `name` (32 bytes, ASCII, NUL-padded)
  - `addr+64`: `short_name` (8 bytes, ASCII) — TDA doesn't bother reading this
  - `addr+72`: `unit` (12 bytes, ASCII)
  - record continues to `addr+124` total (40 bytes trailing padding per `ldparser`'s fmt)
- **⚠️ Conversion formula discrepancy** — the two parsers compute physical value from
  raw sample differently:
  - `ldparser`: `value = (raw / scale * 10^-dec + shift) * mul`
  - TDA `motec.py`: `value = raw * mul / (scale * 10^dec_pts) + offset`
  - These are only equivalent when `shift == 0` (they diverge whenever
    `shift/offset != 0`, since `shift*mul != offset` in general). **Use TDA's formula**
    — it's the authoritative/current implementation. This could not be fully exercised
    against a real file with nonzero shift (see below), so add a unit test with a
    deliberately nonzero shift value once real fixture data is available, and treat
    this as a known risk area during parser implementation.
- **No real SimHub-exported `.ld` file was available locally.** Rather than block on
  that, generated a synthetic-but-format-valid fixture using `ldparser`'s
  `ldData.frompd(df).write(path)` round-trip (3 float32 channels, 50 samples, 10 Hz).
  Verified it parses identically through **both** `ldparser.fromfile()` and TDA's
  `motec._decode()` (values match to float32 precision). Committed to
  `crates/sde-formats/motec/tests/fixtures/synthetic.ld` with expected values dumped to
  `synthetic_expected.json` alongside it — use this as the first Rust parser unit test
  fixture. Getting a real SimHub `.ld` file remains a **manual follow-up** (needs
  SimHub + an actual sim session) to catch any real-world quirks the synthetic file
  can't (e.g. nonzero shift, non-float32 channels, `Beacon`/lap-marker channels,
  populated venue/vehicle sub-records).

## Milestone sequence

1. **MoTeC LD parser** *(done, 2026-07-21 — see `crates/sde-formats/motec`)* — using `binrw` for the
   fixed-record binary layout (file header with meta/data/event pointers → channel header
   linked list → sample data blocks with mul/scale/shift/decimals conversion). Validate
   incrementally (header offsets → channel list → sample values) against TDA's
   `data/motec.py` (primary oracle) using the committed synthetic `.ld` fixture at
   `crates/sde-formats/motec/tests/fixtures/synthetic.ld` (+ `synthetic_expected.json`);
   swap in a real SimHub-generated `.ld` file once available. See "Validation findings"
   above for the exact byte layout and the shift/offset conversion-formula gotcha.
   *(Hardening pass, 2026-07-21):* fixed a panic-on-malformed-input bug where a corrupt
   `channel_meta_addr`/linked-list pointer past EOF indexed the file buffer directly
   instead of going through a bounds-checked `LdError::Truncated`; added unit tests
   in `crates/sde-formats/motec/src/lib.rs` exercising every `LdError` variant via
   deliberately-malformed in-memory buffers (no fixture files needed).
2. **`sde-core::Session` + CLI example** (`dump_channels`) — proves the parser →
   data-model boundary works before any UI exists. **Done** (2026-07-21): `Session`
   mirrors TDA's `base.py` dataclasses, `dump_channels` prints metadata/laps/channels
   for a given `.ld` file, and lap-splitting is ported verbatim from `MOTEC()`'s
   Beacon-channel state machine (including its narrow `v == 100 || v == 2` trigger
   check and the 14-bit sign-corrected value decode) — see
   `crates/sde-core/src/lib.rs`'s `laps_from_beacon`, tested against
   `crates/sde-formats/motec/tests/fixtures/synthetic_with_laps.ld`.
3. **Slint shell + first graph** *(done, 2026-07-21 — see `crates/sde-app`)* — minimal
   window: load a log, plot one channel on a time/distance graph with cursor drag. First
   end-to-end vertical slice. Implemented: `sde-app` (the only crate depending on Slint,
   per the modularity principle above), a "load file" button opening a native file picker
   (`rfd`, filtered to `.ld`) that loads via `sde_core::Session::load_motec`; a hand-drawn
   line graph (Slint `Path` element fed an SVG-style `commands` string built in Rust,
   markup kept in `ui/app.slint` separate from logic in `src/`); a draggable vertical
   cursor (`TouchArea` over the graph) showing time (ms) and the channel's value at the
   cursor, correctly respecting `Channel::interpolate` (linear interpolation vs.
   hold-previous-sample); window title reflects the loaded file name or "No file loaded".
   Pure logic (channel selection, path building, cursor value lookup/interpolation) lives
   in `src/graph.rs`, unit tested independent of Slint/display (including one test against
   the real `synthetic.ld` fixture). **Deferred to milestone 5** ("core UI parity"),
   intentionally out of scope here: channel-picker UI (currently hardcodes the first
   `interpolate == true` channel alphabetically, falling back to the first channel by
   name), distance-axis mode (time-only for now), multiple channels/worksheets/docks,
   lap selection/comparison.
4. **Remaining telemetry format parsers** — XRK, IBT, VBOX, ADULOG, RUN, MLG, each
   behind the shared `LogFormat` trait.
5. **Core UI parity** — worksheets/docks, channel search, lap selection/comparison,
   math channels (matching the original tool's baseline usefulness).
   *(In progress, 2026-07-21)* Channel search + lap selection landed first, as the
   most natural extension of milestone 3's single-graph view — see `crates/sde-app`:
   - `graph.rs` gained `channel_names`/`filter_channel_names` (case-insensitive
     substring match) and `lap_labels` (`"All"` at index 0, `"Lap N (duration)"`
     per `session.laps` entry thereafter). `build_plot` now takes an
     `Option<(f64, f64)>` time-range parameter — when `Some`, both the time axis
     and the vertical value scaling are computed from that window alone (e.g. a
     selected lap), not the whole channel; `None` preserves the old full-channel
     behavior.
   - `main.rs` now keeps the loaded `Session` alive in an `AppState` behind one
     `Rc<RefCell<_>>` (previously only the plotted channel's raw arrays were kept
     around; picking a different channel required reloading the file). A search
     box filters a channel list sidebar (click to plot); a lap `ComboBox` in the
     header restricts the graph to that lap's time window via the same
     `build_plot` range parameter.
   - `app.slint`: added a left sidebar (`LineEdit` search + `ListView` of channel
     names, highlighting the active one) and a lap `ComboBox` in the title bar.
   *(Worksheets/docks, 2026-07-21):* multiple channels can now be plotted at
   once. `AppState.dock_channels: Vec<String>` replaced the single
   `selected_channel`; clicking a channel in the sidebar toggles it in/out of
   the worksheet (highlighted via a `channel-active: [bool]` array parallel to
   the filtered `channel-names` list), each dock has a "x" remove button, and
   `app.slint` renders one stacked graph row per dock (a new `DockData` Slint
   struct — name/units/path-commands/dims/has-data/status-text — via a
   `ListView` over `[DockData]`). All docks share one draggable cursor (each
   dock keeps its own `TouchArea`, but since they're all the same width and
   plotted over the same `current_range`, a fraction means the same timecode
   in every dock); per-dock cursor readouts live in a separate parallel
   `cursor-values: [string]` property so dragging the cursor only updates that
   small array, not every dock's path string. `graph::build_plot`'s `range`
   parameter (added for milestone 5's lap-selection work above) is now always
   `Some(...)` — either the selected lap's window or `graph::session_time_range`
   (new: `(0.0, last_lap.end_time)`) for "All" — so every dock's x-axis lines
   up regardless of which channels are on the worksheet.
   *(Lap comparison, 2026-07-21):* laps can now be overlaid per-dock instead
   of only viewed one at a time. `graph.rs` gained `build_lap_comparison_plot`
   (replaces `build_plot` for all dock rendering, including the plain
   "All"/one-lap case, which is now just a 1-element `ranges` list): given a
   list of `(start, end)` ranges it rebases each to lap-relative `t = 0`,
   scales every trace off one shared time span (`shared_duration` — the
   longest range) and one shared value range across *all* the ranges'
   samples, so overlaid traces are a fair visual comparison, not
   independently auto-scaled. A range with no samples for that channel is
   silently omitted from the output rather than erroring. In `main.rs`,
   `AppState.compare_lap_indices: Vec<usize>` (1-based lap numbers, sorted)
   holds the active comparison set; non-empty overrides the plain
   `selected_lap_index`. `current_ranges` picks between the two. The UI
   (`app.slint`) grew a row of small numbered toggle chips ("Compare:") next
   to the lap `ComboBox` — picking from the `ComboBox` clears any active
   comparison, and vice versa isn't needed since comparison chips are
   independent toggles. `DockData.path-commands` (one string) became
   `DockData.series: [SeriesData]` (`{ commands, color }` — a new Slint
   struct), rendered as one `Path` per series via `for s in dock.series`; a
   4-color palette (`SERIES_COLORS`/`series_color`) is cycled through and
   also drives a legend row (new `LegendEntry`/`legend` property, shown only
   while comparing). Per-dock cursor readouts now show one value per active
   range, pipe-separated (e.g. `"12.3 | 15.0"`), each looked up by clamping
   the lap-relative cursor time into that specific lap's own
   `(start, end)` before calling `value_at_raw` — necessary because a
   channel's raw timecodes run continuously across the whole session, so an
   unclamped lookup near a short lap's end could silently read into the
   next lap's data.
   *(Math channels, 2026-07-21 — milestone complete):* new `sde_core::mathexpr`
   module (UI-free, in `sde-core` rather than `sde-app`, per the workspace's
   modularity principles) adds a small hand-written recursive-descent
   parser/evaluator for arithmetic expressions over existing channels — e.g.
   `[Ground Speed] * 3.6`, `abs(RPM - 6000)`. Grammar: standard `+ - * /`
   precedence, right-associative `^`, unary minus, parens, and four
   functions (`abs`, `sqrt` — 1 arg; `min`, `max` — 2 args). Channel names
   are referenced either as a bare identifier (`RPM`) or bracketed
   (`[Ground Speed]`, required for names containing spaces). Evaluation
   (`evaluate_math_channel(session, name, expr) -> Result<Channel, MathError>`)
   picks the *first*-referenced channel's timecodes as the output's sample
   base and resamples every other referenced channel onto those timecodes
   via each channel's own `interpolate` semantics — so an expression mixing
   channels recorded at different rates follows whichever channel it names
   first. Division by zero yields `±inf`/`NaN`, not an error (only syntax
   errors, unknown channels/functions, wrong arity, and "expression
   references no channel at all" are `MathError`s). In `sde-app`, a
   name/formula input pair in the sidebar (wired via
   `math-channel-add-requested`/`math-channel-removed` callbacks) calls this
   and, on success, inserts the resulting `Channel` directly into the live
   `Session.channels` map — so a math channel behaves exactly like any
   parsed one afterward (searchable, dockable, comparable across laps).
   Redefining an existing *math* channel by reusing its name overwrites it
   in place; reusing the name of a real/parsed channel is rejected, so math
   channels can never shadow actual telemetry data.

   All four milestone-5 sub-features (worksheets/docks, channel search, lap
   selection/comparison, math channels) are now implemented — see
   `crates/sde-app` and `crates/sde-core/src/mathexpr.rs`.

   *(Post-milestone polish, 2026-07-21, prompted by reviewing a screenshot of
   the worksheet):* a real screenshot of the lap-comparison view (against
   `.sample-data/CDA2 AMG EVO PAN RunData.ld`, a 5-lap ACC session) surfaced
   one bug and two feature gaps:
   - **Bug fix:** `TC`/`THROTTLE`/`BRAKE` traces appeared to vanish outside a
     narrow middle band. Verified via `dump_channels` (now also prints each
     channel's `t=[first..last]` timecode range and every lap's
     start/end/duration) that this wasn't a data or lap-boundary problem —
     those channels have continuous samples across the full session, and lap
     boundaries land where expected. The real cause: a channel that sits flat
     at its own min or max for long stretches (e.g. `BRAKE` at 0 outside a
     braking zone) plotted exactly on the dock's top/bottom border pixel and
     became visually indistinguishable from it. Fixed with a new
     `graph::value_scale` helper (used by both `build_plot` and
     `build_lap_comparison_plot`) that pads the value axis by 5% top and
     bottom (`VALUE_MARGIN_FRACTION`) so a flat trace at the extreme always
     plots a few pixels shy of the border.
   - **Timeline zoom** (non-linear-video-editor style): `graph.rs` gained
     `zoom_scroll` (pure, updates a `(start, end)` fraction-of-full-range
     window from one wheel/trackpad scroll event — vertical scroll zooms in
     around the cursor's position so that point stays fixed, horizontal
     scroll/shift-scroll pans, both clamped to `[0, 1]` and never narrower
     than `MIN_ZOOM_WIDTH`) and `apply_zoom` (narrows a set of comparison
     ranges to a zoom window, intersecting each range independently so
     zooming still means the same lap-relative moment across every compared
     lap). `AppState.zoom: Option<(f64, f64)>` holds the active window
     (`None` = fully zoomed out); it's cleared whenever the lap
     selection/comparison set changes, since a zoom fraction of the old
     set's duration wouldn't mean the same thing against a new one. Wired to
     each dock's graph via Slint `TouchArea`'s `scroll-event`; a "Reset zoom"
     button and a "12.0s - 48.0s of 120.7s"-style readout appear only while
     zoomed in.
   - **Worksheet layout modes:** stacked (the original, one full-width row
     per channel), side-by-side (one horizontal row), and a fixed 2-column
     grid. Required factoring the per-dock markup (header + graph + shared
     cursor line + scroll handling) out of the single stacked `ListView` into
     a reusable `DockPanel` sub-component in `app.slint`, instantiated once
     per layout mode's container. Grid placement (`DockData.grid-row`/
     `grid-col`) is computed in Rust (`i as i32 / GRID_COLUMNS`, `% GRID_COLUMNS`)
     rather than in markup, keeping arithmetic out of the `.slint` file per
     the project's markup/logic split. `layout-mode` is pure UI state (an
     `in-out property` on `AppWindow`, no Rust round-trip) since it doesn't
     affect what's plotted, only how the docks are arranged.

   *(Second polish pass, 2026-07-21, from two more real screenshots + user
   testing):* two more bugs and one more feature:
   - **Bug fix — Stacked/Side-by-side showed no charts (Grid did):** the new
     `DockPanel` instances forgot `width: 100%; height: 100%;` in those two
     layout containers (Grid's happened to get it). Without it, `DockPanel`'s
     root `VerticalLayout` sizes itself to its *own* natural/minimum content
     size — the header row has one (text height), but nothing gave the graph
     `Rectangle` below (`vertical-stretch: 1`) any space to stretch into, so
     it collapsed to ~0px tall. Fixed by adding the same explicit
     `width`/`height` the Grid instantiation already had.
   - **Bug fix — zoomed view didn't fill the dock width:** `build_plot`/
     `build_lap_comparison_plot` only ever plotted real samples strictly
     inside `[start, end]`, so a window that doesn't land exactly on a
     sample's timestamp (the common case once zoomed into an arbitrary
     sub-range) left a gap between the trace and each edge — invisible at
     the full lap/session zoom level (gap negligible vs. the whole span) but
     obvious once zoomed in tight (gap now a large fraction of the visible
     width). Fixed with a new `graph::windowed_samples` helper, used by both
     plotting functions: after the usual `[start, end]` filter, it
     synthesizes one extra point at each edge the real samples don't already
     reach (via the same interpolate-or-hold `value_at` lookup used for
     cursor readouts), so the line always touches both edges whenever the
     channel has *any* real overlap with the window. Deliberately returns no
     points (not a synthesized flat line) when the channel has zero overlap
     with the window at all, so a genuinely out-of-range dock still reports
     "no samples" rather than fabricating data.
   - **Channel overlay mode:** a dock can now plot more than one channel at
     once (e.g. `BRAKE`+`THROTTLE` together), the same way lap comparison
     overlays multiple laps — and the two compose (an overlay dock can also
     show each of its channels across multiple compared laps). Interaction:
     Ctrl+click a sidebar channel to queue it (highlighted orange, separate
     from the existing blue "already on the worksheet" highlight) instead of
     immediately creating a dock; an "Add overlay dock"/"Clear" pair appears
     once the queue is non-empty. `AppState.dock_channels` changed from
     `Vec<String>` to `Vec<Vec<String>>` (one dock = one or more channel
     names) — a plain click still toggles a size-1 group exactly as before,
     so the common single-channel workflow is unchanged. In `replot`, each
     channel in a group still goes through `build_lap_comparison_plot`
     independently (so every channel keeps its *own* value scale within the
     shared dock, correct when overlaying channels with different units —
     e.g. RPM alongside a 0-100 percentage), and every resulting series
     across every channel in the group gets a sequential color from the
     (now 8-entry, up from 4) `SERIES_COLORS` palette. `SeriesData` gained a
     `label` field (the channel name), populated only when a dock has more
     than one channel — `DockPanel` renders a small color-keyed legend row
     from it in that case, leaving single-channel docks and the existing
     lap-comparison legend untouched. Cursor readouts prefix each channel's
     value(s) with its name the same way, only when the group has more than
     one channel (`cursor_text_for_group`).

   *(Third polish pass, 2026-07-21, from user testing):* the previous pass's
   "zoom doesn't fill the width" fix (`windowed_samples`, boundary-extending
   the plotted samples) turned out to only be a secondary factor — the real
   cause, also explaining the *unzoomed* "empty space before/after every
   plot" report, was that Slint's `Path` element defaults to `fit: contain`
   (SVG `preserveAspectRatio="meet"` equivalent): it preserves the viewbox's
   aspect ratio and letterboxes rather than stretching x/y independently.
   Every dock uses a square 1000x1000 viewbox (`VIEW_WIDTH`/`VIEW_HEIGHT` in
   `main.rs`) rendered into a wide rectangular dock, so `contain` was scaling
   by the limiting (height) dimension only and leaving large blank margins on
   both sides — at any zoom level, not just when zoomed in tight. Fixed with
   one line, `fit: fill;`, on the `Path` in `DockPanel`. Two more UI bugs
   fixed in the same pass:
   - **Layout-mode highlight got stuck on the wrong button:** the "Grid" /
     "Stacked" / "Side-by-side" toggle used `Button { checkable: true;
     checked: root.layout-mode == N; }`. `Button`'s internal click handler
     assigns to its own `checked` property (`root.checked = !root.checked`)
     before invoking the public `clicked` callback — and in Slint, an
     internal imperative assignment to a property permanently replaces any
     external declarative binding on it. So after the *first* click, the
     `checked: root.layout-mode == N` binding was gone and that button's
     highlight simply stopped updating, no matter which layout was actually
     active. Fixed by dropping `Button` for these three and hand-rolling the
     toggle as a plain `Rectangle` (background driven purely by
     `root.layout-mode == N`) with its own `TouchArea`, the same pattern
     already used for the compare-lap chips — no internal state to fight
     with.
   - **Docks visibly jumped when dragging the cursor:** the per-dock cursor
     readout `Text` in `DockPanel`'s header had no fixed width, so every
     cursor move changed its content's character count (e.g. `"n/a"` vs.
     `"80.123 | 20.456"`), which reflowed the whole header row (and, since
     row height can be affected by reflow in some layouts, visibly shifted
     the dock). Fixed by giving it a fixed `min-width: 160px` and
     right-aligning it, so its box size no longer depends on content length.

   *(Fourth polish pass, 2026-07-21, from user feedback on the updated
   screenshot):* two UX gaps in the lap comparison controls, raised as
   questions rather than bug reports but both real:
   - **"Does the dropdown get overridden by the compare buttons?"** — yes
     (`current_ranges` in main.rs gives the compare-chip selection full
     priority over `selected_lap_index` whenever any chip is active), but
     the dropdown kept showing its stale last value with no indication it
     was being ignored, which is exactly the confusing part. Fixed by making
     the two mutually exclusive *in the UI*, not just in the underlying
     logic: whenever any compare chip is active, the `ComboBox` is replaced
     with a plain "Comparing N laps" indicator plus a "Clear" button
     (`compare-status-text`/`compare-cleared`) that exits back to dropdown
     mode. Exactly one control is ever visible for "what lap selection is in
     effect," instead of two that can silently disagree.
   - **"What happens with 20-50 laps?"** — previously nothing good: the
     compare-chip row was an unbounded `HorizontalLayout`, so it would have
     just overflowed off the right edge of the window with no way to reach
     later laps. Wrapped it in a fixed-width (`200px`) `ScrollView`
     (horizontal-scrollbar-policy: as-needed, vertical: always-off) so it
     stays bounded and reachable regardless of lap count. (The plain lap
     `ComboBox` was already fine at any size — `std-widgets`' dropdown
     scrolls internally.)

   *(Fifth polish pass, 2026-07-21 — milestone 5 closed out):* trackpad
   scroll-axis locking, the last requested item. Previously every
   `zoom-scrolled` event applied both `delta_x` (pan) and `delta_y` (zoom)
   independently whenever either was nonzero — fine for a plain mouse wheel
   (which only ever reports `delta_y`), but trackpads report a nonzero value
   on *both* axes on nearly every event, even during an intended single-axis
   swipe. Net effect: panning horizontally would also drift the zoom level
   (and vice versa) from incidental diagonal jitter. Fixed by locking each
   scroll *gesture* to one axis for its whole duration rather than deciding
   per event:
   - `graph::dominant_scroll_axis(delta_x, delta_y) -> ScrollAxis` (pure,
     tested) picks whichever axis a single event's deltas favor (ties go to
     `Zoom`, so a plain vertical wheel — which reports `delta_x == 0.0` —
     never accidentally locks to `Pan`).
   - `AppState.scroll_gesture: Option<(ScrollAxis, Instant)>` remembers which
     axis the *current* gesture locked to and when its last event arrived.
     The `zoom_scrolled` handler only calls `dominant_scroll_axis` (re-
     deciding the lock) when there's no gesture in progress or the previous
     one went quiet for `SCROLL_GESTURE_TIMEOUT` (400ms); otherwise it keeps
     the existing lock regardless of the current event's own deltas.
   - Whichever axis isn't locked has its delta zeroed out entirely before
     reaching `graph::zoom_scroll` — not just "which effect wins" but "the
     other axis's input is discarded outright" — so off-axis jitter can't
     leak through even partially.
6. **`sde-setup`** — setup sheet data model + diff view between two setups. First
   genuinely new (non-port) feature.
7. **`sde-analysis`** — derived channels layered onto graphs: damper velocity
   histograms, ABS/TC intervention markers + stats, ride-height/roll estimates,
   brake bias effectiveness. This is where the race-engineer motivation pays off.
8. **Video sync + GPS map** — port once core viewing/analysis is solid.
9. **Per-sim setup adapters** — `sde-formats::rbr` first (cross-check against
   `durandom/race-engineer`'s domain model docs), then Dirt Rally / ACR / EA WRC
   adapters as each sim's file formats are scoped (likely thinner given more
   limited/less open telemetry and setup file access than RBR). For RBR, the
   **NGP telemetry recorder** (see "RBR NGP telemetry format findings" below) is
   the actual telemetry channel-log source to target here — it's documented,
   plain-text, and column-named, unlike the `.rpl` replay files (see below), so
   it should land before/alongside any `.rpl` work.
10. *(Future, deferred)* **`sde-viz`** — data-driven pose animation:
    - `CarPose` struct (roll, pitch, yaw, position, steer angle, 4x wheel travel)
      computed from logged channels — no physics simulation, just geometry:
      ```
      roll  ≈ atan2( (travel_FR + travel_RR)/2 - (travel_FL + travel_RL)/2, track_width )
      pitch ≈ atan2( (travel_RL + travel_RR)/2 - (travel_FL + travel_FR)/2, wheelbase )
      ```
    - Start with a 2D top-down view before attempting 3D.
    - 3D approach if/when pursued: `three-d` crate, rendered off-screen to a texture,
      exposed to Slint as an `Image`, synced to the playback timeline cursor.
    - Keep `sde-viz` UI-free except for the render module, mirroring the rest of
      the workspace's modularity principle.

### RBR NGP telemetry format findings (2026-07-21)

Investigated two distinct RBR/NGP7 data sources against `.sample-data/RBR/*.rpl`
(RSF replay files) and against the locally installed NGP plugin
(`C:\Richard Burns Rally\Plugins\NGP\`). These are **two separate formats** —
don't conflate them when scoping milestone 9:

- **`.rpl` replay files** (what's currently in `.sample-data/RBR/`, generated by
  the NGPCarMenu plugin per the `.ini` sidecar's header comment): RSF replay
  recordings, not telemetry channel logs.
  - First ~730KB of `Anna_rsf_practice_410.rpl` (8.4MB total) is **readable
    Lisp-style car setup text** embedded verbatim: `MaxSteeringLock`,
    `FrontRollBarStiffness`/`RearRollBarStiffness`, diff torques
    (`CenterDiffMaxTorque`/`FrontDiffMaxTorque`/`RearDiffMaxTorque`), brake
    pressures, full gear table (`GearId0..9`, `FinalDriveId`, `DropGearId`),
    NGP damper curves (`HighSpeedBreakReboundFront_NGP`,
    `BumpStopStiffnessFront_NGP`, etc.), `HandbrakePercentage_NGP`, VCU flags
    (`AutoGears`, `GearGuard`, `ClutchHelp`, `CenterDiffThrottle_00..NN`) — a
    real, populated example of the field set `durandom/race-engineer`'s data
    model doc describes, useful for `sde-setup`'s RBR setup adapter.
  - Remaining ~7.6MB is dense binary, presumed per-tick replay/physics state.
    **Not decoded** — no public spec found; would need its own
    reverse-engineering pass (frame size looks variable) if ever pursued.
  - **Not the target for a telemetry-channel parser** — see NGP recorder below.

- **NGP telemetry recorder** (built into the NGP plugin itself, separate from
  `.rpl`): this is the actual documented, sim-native telemetry log format, and
  should be the real target for `sde-formats::rbr` telemetry parsing.
  - **Struct definition**: `Plugins\NGP\sdk\rbr.telemetry.data.TelemetryData.h`
    — `#pragma pack(1)` C++ struct tree:
    `TelemetryData { totalSteps_, Stage stage_, Control control_, Car car_ }`.
    `Car` nests `Motion` (×2: velocities_/accelerations_ — surge/sway/heave/
    roll/pitch/yaw), `Engine` (rpm_, radiator/engine coolant temps, engine
    temp), and 4× `Suspension` (`suspensionLF_/RF_/LB_/RB_`), each holding
    `Damper{damage_, pistonVelocity_}` and `Wheel{ BrakeDisk{layerTemperature_,
    temperature_, wear_}, Tire{pressure_, temperature_, carcassTemperature_,
    treadTemperature_, currentSegment_, 8× TireSegment{temperature_, wear_}} }`.
    `Control` covers steering/throttle/brake/handbrake/clutch/gear/
    footbrakePressure/handbrakePressure. `Stage` covers progress/raceTime/
    driveLineLocation/distanceToEnd. Also present: `rbr.plugin.IPhysicsNGProxy.h`,
    `rbr.plugin.IProxy.h`, `rbr.plugin.IService.h` (plugin interfaces, not
    needed for log parsing).
  - **Field selection config**: `Plugins\NGP\Telemetry.ini` (active) /
    `Telemetry.sample.ini` (reference, lists every available field under
    `[NGP_ALL]`) — a flat `[NGP]` section of dotted-path keys (e.g.
    `LF.brakeDiskTemp=1`, `LF.segmentData[0].temperature=1`,
    `vecLinearVelocityCar=1`) toggling which fields get recorded. **The output
    column set is therefore configurable per-recording, not fixed** — a real
    parser needs to read the header row rather than assume a static schema.
    `telemetryTics=5` in `RichardBurnsRally.ini`'s `[NGP]` section controls
    sample decimation (write every Nth physics tick).
  - **Output file**: plain **text file, whitespace-delimited, with a header
    row of these dotted column names**, written to `Plugins\NGP\telemetry\`
    when `telemetryRecording=1` (toggled in-game via the NGP plugin dialog,
    `Plugins → NGP`, key `T`). File name is auto-generated. Confirmed
    text/columnar (not binary) via `Plugins\NGP\scripts\rbr.gp` and its
    sibling `.gp` scripts (damper forces, tyre temps/wears, brake disk temps,
    etc.), which all use gnuplot's `column("name")` addressing — this only
    works against a headered plain-text file. `ReadMe.NGP.txt`'s "Telemetry
    Recorder" section confirms: recording only works in normal GAME mode (not
    REPLAY), and recommends gnuplot over a spreadsheet app given the volume.
  - **UDP telemetry**: same `Car`/`Control`/`Stage` data is also emittable
    live over UDP (`udpTelemetry=1`, `udpTelemetryEndpoints=127.0.0.1:6776` in
    `RichardBurnsRally.ini`) — out of scope for a file parser, but useful if
    live telemetry capture is ever wanted (parallel to the `sde-video` sync
    use case, not pursued now).
  - **No sample file captured yet** — `telemetryRecording=0` currently and
    `Plugins\NGP\telemetry\` is empty on this machine. **Manual follow-up**:
    enable `telemetryRecording=1`, drive a short lap, and pull a real sample
    into `.sample-data/` (gitignored, same as the ACC `.ld` file) before
    writing the milestone 9 parser, so real column-set/value-range quirks
    (per the MoTeC lesson above) surface before, not after, implementation.

### RSF real-capture validation (2026-07-26)

First real RBR/RSF capture, in `.sample-data/RBR/MINI JCW - Gabiria-Legazpi 2004/`
(gitignored): two runs of the same car on the same stage, seven minutes apart,
each with `motec/` (`.ld`), `setup/` (`.lsp`) and `replay/` (`.rpl` + `.ini`),
plus a shared `Pacenotes/` folder. Run1 took two "recover vehicle" penalties;
Run2 was clean and 135 s faster on a much stiffer setup. Having a *pair* of runs
that differ in both setup and incident is what made most of the below findings
falsifiable — several would have looked like noise from a single capture.

**RSF exports MoTeC LD directly**, so `sde-motec` is already the RBR telemetry
path — no separate NGP text-log parser is needed for milestone 9 (the
`Plugins\NGP\telemetry\` recorder described in the previous section is a second,
independent source, still uncaptured). Both files parse cleanly: 185 channels of
dotted NGP field names (`LF.brakeDiskTemp`, `vecLinearVelocityCar.x`, …) at
54039 / 44567 samples. Two real defects surfaced, both now fixed in
`crates/sde-formats/motec/src/lib.rs` with regression tests in its
`rsf_ngp_tests` module (synthetic in-memory `.ld` buffers, so the multi-megabyte
captures aren't needed to run them):

- **Some int32 channels are 10^6 fixed-point with `dec_pts = 0`.**
  `LF.brakeDiskTemp` decoded to 672533952 where the true value is 672.533952 K;
  `brakeWear` to 2159789 for 2.159789 %. Confirmed at both ends of the stage
  (disc cooling 672 K -> 353 K over the run is physically right). Critically,
  this is *not* fixable from the element type: `currentTyreSegment` (0..7) and
  `helperSpringActive` (0/1) share the same int32 type and are genuinely
  integral. The affected fields are exactly those typed `float` in NGP's own
  `TelemetryData.h` under `BrakeDisk { layerTemperature_, temperature_, wear_ }`,
  so the fix is keyed on the NGP field name (`NGP_MICRO_FIXED_POINT_FIELDS`),
  matched on the trailing dotted component so all four corners resolve to one
  entry. A field RSF adds later that belongs on the list but isn't yet fails
  loudly with absurd magnitudes — the safe direction to err. Display `dec_pts`
  is also overridden to 3, since the exporter's 0 would render 2.16 % wear as a
  flat `2 %` and discard the signal the fix just recovered.

- **The declared `sample_rate` is *right*; the original diagnosis here was
  wrong.** Recorded in full because the mistake is instructive and nearly
  shipped.

  Dividing each file's row count by its `raceTime` span gives 152.6 Hz and
  154.3 Hz against a declared 144 Hz, differing from *each other* by 1.1 %.
  That looked conclusive: no fixed rate fits both, so the synthetic
  `index / sample_rate` axis must be stretching each session differently.
  `apply_ngp_timebase` was written to rebuild the axis from `raceTime`
  instead, subtracting penalties, back-extrapolating the countdown and
  nudging apart the ~20 % duplicate values.

  The NGP `.tsv` disproves it. NGP's recorder writes a `utcSystemTime`
  wall-clock column that `ngp2MoTeC` drops, and against it **both captures
  sample at 144.095 Hz** — the declared rate is right to within 0.07 %
  (0.24 s of drift over a 368 s recording). The `.tsv` also carries
  `totalSteps`, whose delta is exactly `telemetryTics` (5) for every one of
  the 54038 row transitions: the row stream is perfectly uniform.

  The phantom rates came from `raceTime` not spanning the recording. It is
  the *stage* clock, so it reads a flat 0 through the countdown, jumps on a
  penalty without wall-clock time passing, and — the part that mattered —
  **freezes at the finish while recording continues** for a fixed ~20 s
  run-out (2881 and 2882 rows; the car is still braking from 116 and
  140 km/h, `distanceToEnd` running to -277 m). Dividing by a span that
  excludes ~20 s of a ~350 s run inflates the rate by exactly the ~6 %
  observed, and the residual difference between the two runs is just Run1's
  longer stage.

  Worse, deriving timecodes from `raceTime` **compressed that entire run-out
  into 0.003 ms** — real telemetry destroyed. `post_finish_run_out_is_not_
  compressed` in `rsf_ngp_tests` guards against a regression.

  What `apply_ngp_timebase` does now is much smaller. The uniform axis is
  kept; only two things are taken from `raceTime`:

  1. **The origin.** The axis is shifted so t=0 is the stage start, letting
     two runs align with no per-run offset. Countdown rows take honest
     negative timecodes (-7006.9 ms in both, matching the `.tsv`'s measured
     7.002 s). **`timecodes[0]` is therefore not guaranteed `>= 0`** — the
     doc comments on `LdChannel::timecodes` and `Channel::timecodes` say so.
     Only the origin moves; spacing is untouched.
  2. **Penalty events.** A recovery adds a fixed 35 s to the scored clock
     with no wall-clock time passing, so it leaves the sample axis alone and
     is reported as a `TimePenalty` event only. Run1 yields two, at
     219.5 s and 314.1 s, +35.0 s each; Run2 none. Corroborated by the
     replay `.ini`: Run1 has `[RunkiSpots] Count = 2` at 4342.6 m / 5810.4 m
     with `Tim = 35.0` each, and Run2 has no such section. **`[RunkiSpots]`
     is the recovery-event record.** Note scored times aren't comparable
     between a penalised run and a clean one — subtract these to compare
     driving.

  Gated on a usable `raceTime` channel, so non-RSF files are untouched;
  verified no change to the ACC capture (still 5 laps, 37 channels).

  Lesson for the next format investigation: two independent signals
  disagreeing (144 Hz declared vs 152.6/154.3 Hz measured) does not mean the
  declared one is wrong. Here the measurement was wrong, because the
  denominator silently excluded part of the recording. A wall-clock column
  settled in one query what three rounds of inference got backwards.


### NGP native `.tsv` telemetry (2026-07-26)

The previous section's open item ("no sample file captured yet") is closed —
and it turns out the `.tsv` is not redundant with the `.ld`. NGP's recorder
writes `Plugins\NGP\telemetry\<name>.tsv`, and `ngp2MoTeC` (shipped in the RBR
install at `NGP2MoTeC\ngp2MoTeC.exe`) converts it to the `.ld` we parse. Both
runs' `.tsv` are now in `.sample-data/` alongside their `.ld` (gitignored;
85 MB and 70 MB).

Format is as predicted: tab-separated, one header row of dotted field names,
one row per `telemetryTics` physics ticks (`telemetryTics=5` in
`RichardBurnsRally.ini`). 190 columns vs the `.ld`'s 185. The conversion is
lossy in three ways:

- **Four columns are dropped entirely**: `totalSteps` (physics tick counter),
  `stage`, `car`, and `utcSystemTime` (wall-clock timestamp,
  `YYYY-MM-DD HH:MM:SS.ffffff`). Two of these settled the timebase question
  above that three rounds of inference from the `.ld` alone got wrong.
- **Channel names are truncated to 32 characters** by the `.ld` channel record
  (`radiatorCoolantHeatState.temperature` -> `radiatorCoolantHeatState.tempera`).
  Five columns are affected.
- Everything else round-trips, including the 10^6 fixed-point encoding —
  `LF.brakeDiskTemp` reads `672235712` in the native `.tsv` too, so that
  scaling is **NGP's own, not an artifact of `ngp2MoTeC`**.

`utcSystemTime` is worth keeping in mind for `sde-video`: it's an absolute
wall-clock reference, which is exactly what's needed to sync telemetry against
externally-recorded video (OBS capture, phone footage) rather than relying on
manual alignment. Not pursued yet.

Not proposing a `.tsv` parser right now — the `.ld` path works and is far
cheaper to read than 85 MB of text — but the extra columns are a real argument
for one later, and for preferring the `.tsv` as the archival source.

### Install-path discovery and configuration (design note, 2026-07-26)

Per Anna: in a normal user environment the app must be pointed at the RBR
install root (e.g. `C:\Richard Burns Rally\`), with every other location
*inferred* from it and individually *overridable*. Verified against the live
install on this machine, the standard layout is:

| What | Path (relative to install root) |
| --- | --- |
| NGP telemetry (`.ld` + `.tsv`) | `Plugins\NGP\telemetry\` |
| Telemetry field selection | `Plugins\NGP\Telemetry.ini` (+ `.sample.ini`) |
| Sample-decimation / recording toggle | `RichardBurnsRally.ini` `[NGP]` |
| Replays (`.rpl` + `.ini` sidecar) | `Replays\` |
| Car setups (`.lsp`) | `SavedGames\<CarPhysicsFolder>\` |
| Pacenote plugin + its notes | `Plugins\Pacenote\` |
| RSF car/physics data | `rsfdata\cars\` |
| Stage/track data | `Maps\` |
| RSF launcher config | `RallySimFans.ini`, `rallysimfans_personal.ini` |

Note the `.sample-data/` folder layout (per-run `motec/`, `setup/`, `replay/`
subfolders) is a hand-made capture convention, **not** RSF's — real discovery
must use the table above, not that shape.

The `.ini` files double as existence checks for validating a candidate install
root (e.g. `RichardBurnsRally.ini` + `Plugins\NGP\Telemetry.ini` both present),
and several of them carry the settings the app needs to *read* anyway:
`telemetryRecording`, `telemetryTics`, and the `[NGP]` field selection that
determines which columns a recording will contain. Worth surfacing in the UI —
"telemetry recording is currently off" is a much better first-run experience
than an empty file list.

Config model, when built: a single required install root, a resolved-path
struct with per-path overrides, and validation that reports which expected
paths are missing rather than failing wholesale. Probably `sde-rbr` (path
discovery is sim-specific, and UI-free), consumed by `sde-app`.

Also worth recording from the same investigation, none of it implemented yet:

- **Setup `.lsp`** is a Lisp-style s-expression, 274 key/value pairs over 15
  sections, and diffs cleanly between the two runs (front springs 26000 ->
  45500 N/m, ARB 16000 -> 21000, brake pressure 4.0 -> 6.14 MPa, 58 values in
  all). Two parsing quirks: each section repeats its key list with *empty*
  values as a trailer (skip any key not followed by a numeric), and
  `vecTopMountPosition` carries three. Run1's file has three trailing NUL bytes.
  Values tie directly to telemetry — `TyreLF.Pressure 195000` equals the
  `LF.pressure` channel exactly — so setup/telemetry cross-validation is
  possible.
- **Replay `.ini`** is plain INI and the best session-metadata source in the set:
  stage name/ID/length, car, *the setup file used*, finish time, avg speed, tyre
  compound, weather, surface wetness/age, damage model, NGP/RSF versions, plus
  `[RunkiSpots]`. Cheapest high-value parser to write next.
- **Replay `.rpl`**: offset 0 = checksum, 4 = file length, 8 = version, 12 =
  driver name (16 B), 28 = MapID, 32 = CarSlot. **The full setup `.lsp` is
  embedded verbatim** at 396376 (byte-identical to the standalone file), so
  replays are self-describing. The body from ~421588 is a fixed **32-byte
  per-frame** car-state stream with world position as plain float32 XYZ at
  frame+8 — cross-validated against telemetry `position.x/y/z` at three widely
  separated samples, each matching a unique offset. Not yet solved: a single
  global frame base/phase across the whole file (phase appears to shift at a few
  section boundaries, so a decoder needs resync), and the other 5 floats per
  frame. Tractable now rather than opaque, but still its own project.
- **Pacenotes `.ini`**: 293 entries. Types fall in two real namespaces — 1..298
  (core RBR + extended) and 2004..2271 / 4075..4092 — which interleave within
  ~0.2 m, so one spoken call is composed of several entries. 14 entries have a
  clobbered high halfword: the recovered bytes are an exact anagram of
  `RBR-Enhanced.ini`, i.e. an adjacent string overwrote the field. `type &
  0xFFFF` recovers the true value in all 14 cases (every masked value lands in a
  known range). A further 12 entries with `flag = 0` at `distance ~= 1e-05` are a
  junk preamble. Parser rule: mask to 16 bits, drop `flag = 0`.

## Open validation tasks (do before/while coding)

- [x] Read `ldparser`'s actual `struct.unpack` format strings for the real field
      layout, cross-checked against TDA's own `data/motec.py` — see "Validation
      findings" above for the reconciled byte layout and the formula discrepancy.
- [x] Read `durandom/race-engineer`'s `docs/data-model-rbr.md` (already cloned) —
      field tables captured above; ready to inform the `sde-formats::rbr` adapter
      when milestone 9 comes up.
- [x] Generated a synthetic, format-valid `.ld` fixture (no SimHub file was locally
      available) via `ldparser`'s write path, cross-validated against TDA's
      `motec._decode()`. Committed to `crates/sde-formats/motec/tests/fixtures/`.
- [x] **Manual follow-up, real-world validation (2026-07-21):** tested against a real
      Assetto Corsa Competizione `.ld` export (`.sample-data/`, gitignored — not
      committed as a fixture; large and not ours to redistribute). Parsed cleanly
      (37 channels, physically sane values), but surfaced three real-world gaps
      fixed this session, all in `crates/sde-formats/motec/src/lib.rs` and
      `crates/sde-core/src/lib.rs`:
      - **Every channel's unit was blank.** TDA's `interpolate = unit not in ('s',
        '')` heuristic assumes real MoTeC hardware always labels analog channels
        with units; ACC's exporter doesn't, which would otherwise mark all
        channels (including RPM/speed/G-force) as non-interpolating. Fix: when
        *every* channel in a file has an empty unit (a file-level signal, not
        per-channel), fall back to `interpolate = true` unless the channel name
        (`_`-split, case-insensitive) matches a small discrete-signal token list
        (`GEAR`, `BEACON`, `ABS`, `TC`, `DRS`, `FLAG`). Real hardware files with at
        least one labeled channel are unaffected.
      - **Lap channel is named `LAP_BEACON`, not `Beacon`.** TDA's Python oracle
        (and this project's port) looked up the exact key `"Beacon"`. Fixed via a
        small `BEACON_CHANNEL_NAMES` alias list checked in order.
      - **ACC's `LAP_BEACON` channel data is all zeros anyway** — lap-crossing
        events aren't encoded in the `.ld` channel data at all for this exporter.
        The real lap data lives in a sidecar `.ldx` XML file (`<MarkerGroup
        Name="Beacons">` with `<Marker Time="...">` in microseconds, plus summary
        `<String Id="Total Laps"/Fastest Lap"/"Fastest Time">` fields). Added
        `sde_motec::ldx` (new `roxmltree`-based parser, `LdxFile`/`parse_ldx`) and
        wired it into `Session::load_motec`: if a same-stem `.ldx` file exists and
        parses with non-empty markers, its times take priority for lap splitting;
        otherwise falls back to the existing Beacon-channel state machine. A
        missing or malformed `.ldx` is not a load error — it's optional
        supplemental data.
      - Still open: no real SimHub-exported (genuine MoTeC hardware) `.ld` file
        has been tested yet, so the nonzero-shift conversion-formula risk noted
        above remains unverified against real hardware data (only against the
        synthetic fixture and this one game-exported file, which happened to have
        `shift == 0` throughout). The 2026-07-26 RSF capture does not close this
        either — its channels are all `shift == 0`, `mul == 1`, `scale == 1` too.
- [x] **Manual follow-up, RSF/RBR real-world validation (2026-07-26):** captured two
      runs of one car/stage with telemetry, setup, replay and pacenotes (see
      "RSF real-capture validation" above). Confirmed RSF exports MoTeC LD
      directly, and fixed two defects it surfaced (10^6 fixed-point int32
      channels; wrong declared `sample_rate`, replaced by a penalty-corrected
      `raceTime` axis).
- [x] **Replay `.ini` sidecar parser (2026-07-26):** new `sde-rbr` crate
      (`crates/sde-formats/rbr`) with a hand-rolled minimal INI reader (`ini.rs`,
      no new dependency) and `replay.rs`'s `ReplayInfo`. Models stage / car /
      setup-name / result / conditions / versions plus `[RunkiSpots]` recovery
      events, and offers `driving_time_secs()` = scored finish time minus
      recovery penalties — the figure that actually matches the corrected
      telemetry timebase and the only one comparable across runs. Unmodelled
      `[Replay]` keys are kept in `extra` so a newer RSF build's additions stay
      reachable; malformed fields degrade to `None` rather than failing the
      file. Tested with verbatim inline copies of both sample sidecars, plus an
      on-disk test that runs against `.sample-data/` when present and skips
      cleanly in CI.
- [ ] Decide how replay metadata reaches the app. `Session` is telemetry-shaped
      and `sde-core` deliberately doesn't depend on `sde-rbr`; pairing a `.ld`
      with its `.ini` also can't rely on the folder layout in `.sample-data/`
      (that's a hand-made capture convention, not RSF's own). Probably belongs
      in `sde-app` or a small session-assembly layer, not in `sde-core`.
- [ ] Cross-check `ReplayInfo::recovery_spots` against
      `Session::time_penalties` on load — counts should match, and each
      `position_m` should agree with the stage distance at the corresponding
      penalty. Cheap, and it validates that a `.ld` and a `.rpl` describe the
      same run.
- [ ] Capture an RSF run on a *gravel/snow* stage and a different car, to check
      whether the 10^6 fixed-point field list holds beyond this one tarmac/Mini
      combination. Anna is capturing these separately. (The 1009-row pre-start
      window is no longer open: `totalSteps` = 5050 at the stage start in both
      runs, so the countdown is a fixed physics-tick count.)
- [ ] Build install-root configuration + path discovery (see the design note
      above). Prerequisite for anything that loads data outside `.sample-data/`,
      and it supersedes the "how does replay metadata reach the app" question
      below — pairing a `.ld` with its `.rpl`/`.ini` should go through resolved
      paths, not a folder convention.
- [ ] Consider reading the NGP `.tsv` directly rather than the converted `.ld`,
      to recover `utcSystemTime` (absolute wall clock — the natural anchor for
      `sde-video` sync), `totalSteps`, and untruncated channel names.
- [ ] Decide where the `.lsp` setup parser lives (`sde-formats::rbr`) and whether
      to read the setup from the standalone file or the copy embedded in the
      `.rpl` — the latter is self-describing and can't drift from the run.
