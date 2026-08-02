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

## Where things stand / what to pick up next *(updated 2026-08-02)*

A running "you are here" pointer, so a session starting cold doesn't have to
reconstruct the state from the milestone list below. **Update this section
whenever a milestone closes or a new blocker appears** — the detail belongs in
the milestone entries and design notes; this is only the index into them.

**Done:** milestones 1–6. Telemetry parsing (MoTeC `.ld` + `.ldx`, iRacing
`.ibt`, shtep, NGP `.tsv`), `sde-core::Session`, the Slint app with
worksheets/docks/lap comparison/math channels/zoom-pan, RBR install discovery +
replay `.ini` pairing + recovery cross-check, and `sde-setup` (`.lsp` parser,
sim-agnostic model, diff engine, CLI + app panel).

**Open branch:** `milestone-6-sde-setup` -> PR #12, which carries milestone 6
plus the 2026-08-02 interaction pass (setup panel relayout, proportional
zoom/pan with deferred axis locking, and dock-header dragging — plain to
reorder, Ctrl to merge into an overlay group).

**Next up, in the order they make sense:**

1. **Eyeball PR #12 before merging.** The setup-panel relayout and the
   drag-to-merge feel are reasoned from the markup and Slint's input source,
   and are covered by unit tests on the *arithmetic*, but neither has been
   watched on screen. Everything else in the PR has been verified against the
   real `.sample-data/` captures.
2. **Milestone 7, `sde-analysis`** — the next planned milestone and the one
   the whole race-engineer motivation points at: damper velocity histograms,
   ABS/TC intervention markers and stats, ride-height/roll estimates, brake
   bias effectiveness. Nothing blocks it; `sde-core::Session` and the math
   channel machinery are both in place.
3. **Second half of design-note principle 8** — pairing the setup diff with a
   *performance* delta trace. Deliberately deferred out of milestone 6 because
   it needs the app to hold **two loaded sessions** at once, which `AppState`
   currently isn't shaped for (one `session: Option<Session>`). That reshape
   is the real work; the setup diff itself is done.

**Known gaps worth recording (none blocking):**

- **An overlay group can be built but not taken apart.** Dragging merges
  (Ctrl) and reorders (plain), but there's no gesture for pulling one channel
  back out of a merged dock — the only exit is removing the whole dock with
  its "x" and re-adding the channels individually. The natural shape is a
  small remove control per swatch in the dock's existing per-channel legend
  (`app.slint`'s `DockPanel`, shown only when a dock overlays more than one
  channel), which would need `dock_channels[i]` to drop one entry and collapse
  the dock back to a plain one when a single channel is left.
- The stacked layout's `ListView` would normally steal a drag past 8px and
  cancel the dragged header's grab; it doesn't only because Slint's
  `ScrollView` defaults to `interactive: false` (drag-to-pan off). If that
  scroll behaviour is ever changed, header dragging in stacked mode breaks —
  see the interaction-pass notes under milestone 5.
- No settings screen. The install root persists
  (`%APPDATA%\sde-app\install_root.txt`) but nothing else does — no saved
  worksheet layouts, no window state.
- Telemetry files and replay `.ini`s are still paired by hand in the app
  (`refresh_setup_panel` auto-resolves the *setup* from a loaded replay, but
  the replay itself is a manual pick, and auto-matching by modification time
  is only a fallback). No known filename/folder convention exists to pair
  them reliably — see the open-task list at the bottom.
- No genuine MoTeC-hardware `.ld` has ever been tested, so the nonzero-`shift`
  conversion formula remains unverified against real hardware. Every file
  tested so far (synthetic fixture, ACC export, RSF capture) has
  `shift == 0`.

**House rules that aren't in the code:** every change goes on a branch with a
PR, never straight to `main`. `sde-formats`/`sde-core` stay UI-free and
dependency-light; `sde-app` is the only crate allowed to depend on Slint, and
arithmetic belongs in Rust, not in `.slint` markup. On-disk tests run against
the gitignored `.sample-data/` when it's present and return early when it
isn't, so CI stays green without it.

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
│   ├── shtep/              # SimHub TSV export (companion plugin repo `../shtep`) —
│   │                       #   DONE (`sde-shtep`). No TrackDataAnalysis oracle (bespoke
│   │                       #   schema, not a port) — `../shtep/SCHEMA.md` is authoritative.
│   ├── rbr/               # RBR/RSF companion files — STARTED (`sde-rbr`).
│   │                      #   ini.rs     — shared minimal INI reader
│   │                      #   replay.rs  — replay metadata .ini sidecar (DONE)
│   │                      #   install.rs — install-root path discovery (DONE)
│   │                      #   pairing.rs — telemetry <-> replay matching (DONE)
│   │                      #   setup.rs   — .lsp setup sheet (DONE)
│   │                      #   todo: pacenote .ini, .rpl frames
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
├── sde-setup/             # DONE (race-engineer feature, milestone 6): sim-agnostic setup
│                          # sheet model + diff engine. Ordered named groups of named
│                          # entries rather than a fixed springs/dampers/ARBs struct, so
│                          # each sim's own sheet shape survives. Per-sim adapters live
│                          # here (sde_setup::rbr) reading sde-formats::<sim> parsers,
│                          # keeping the format crates leaf dependencies.
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
- **`shtep` (SimHub Telemetry Export Plugin)** — companion repo, `../shtep`, C#/SimHub
  PluginSDK. Writes `{base}.tsv` + `{base}.meta.json` sidecar pairs per its own
  `SCHEMA.md` (v1.1). **Not a port** — this is a bespoke format this project's own
  companion plugin defines, so `SCHEMA.md` is the authoritative spec directly, no
  `TrackDataAnalysis` oracle involved. `sde-formats/shtep` implements the Rust-side
  reader; the plugin repo owns recording only (no `.ld`/`.ibt`/MoTeC knowledge lives
  there by design). See milestone 4 below for what's implemented.

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

### IBT (iRacing) format findings (2026-07-27)

Cross-checked `TrackDataAnalysis/data/iracing.py` (`_decode`/`_decode_var`/`_find_laps`/
`_filter_gps`) against real `.ibt` captures now in `.sample-data/iRacing/` (a Hell RX
rallycross session with a Joker lap, and a Mt. Washington Hillclimb stage). Verified the
byte offsets below with a standalone Python re-implementation of the oracle's
`struct.unpack_from` calls against one of the real files (`Mt Washington Hillclimb/
Subaru_WRX_STI/*.ibt`) before writing any Rust:

- **Fixed 144-byte header at offset 0** (little-endian), read as
  `struct.unpack_from('<10i12xi', m, 0)` in the oracle:
  - `+0`: `ver` (i32)
  - `+4`: `status` (i32)
  - `+8`: `tick_rate` (i32, Hz — e.g. `60`)
  - `+12`: `session_info_update` (i32, unused by the oracle)
  - `+16`: `session_info_len` (i32) — byte length of the trailing YAML block
  - `+20`: `session_info_offset` (i32) — absolute file offset of the YAML block
  - `+24`: `num_vars` (i32) — channel/variable count
  - `+28`: `var_header_offset` (i32) — absolute file offset of the first
    `RawVarHeader` record (real IBT header is `irsdk_header`, but nothing
    downstream needs to assume this equals 144; always read it from the field)
  - `+32`: `num_buf` (i32) — the oracle only ever reads buffer 0 and just prints a
    warning if this isn't `1`; real captures always have exactly one buffer, so the
    Rust port also only reads `varBuf[0]` (not an error, matching the oracle)
  - `+36`: `buf_len` (i32) — stride in bytes between consecutive sample records
  - `+40..48`: `pad1[2]` (unused)
  - `+48..112`: `varBuf[4]`, each `{ tick_count: i32, buf_offset: i32, pad: [i32; 2] }`
    (16 bytes) — only `varBuf[0].buf_offset` at `+52` is used
  - `+112`: `irsdk_diskSubHeader` (32 bytes), read as
    `struct.unpack_from('<I4xddii', m, 112)`:
    - `+112`: `session_start_date` (u32, Unix `time_t`) — 4 bytes of padding follow
      (`4x`), so this is *not* a full 8-byte `time_t`, just the low 32 bits
    - `+120`: `session_start_time` (f64, seconds)
    - `+128`: `session_end_time` (f64, seconds)
    - `+136`: `session_lap_count` (i32)
    - `+140`: `session_record_count` (i32) — number of sample records in the buffer
  - Confirmed against the real file: `var_header_offset=144`,
    `session_info_offset = 144 + num_vars*144` (var headers are contiguous, 144 bytes
    each), and `varBuf[0].buf_offset = session_info_offset + session_info_len`
    (sample buffer immediately follows the YAML block) — the three regions are
    laid out back-to-back with no gaps in practice.
- **144-byte `RawVarHeader` records**, `num_vars` of them starting at
  `var_header_offset`, per `_decode_var`'s
  `struct.unpack_from('<3ib', m, offs)` + fixed string fields:
  - `+0`: `type` (i32) — index into `['c', '?', 'i', 'I', 'f', 'd']`: `0`=char(1B),
    `1`=bool(1B), `2`=int32 signed(4B), `3`=bitfield/u32(4B), `4`=float32(4B),
    `5`=float64(8B)
  - `+4`: `offset` (i32) — this variable's byte offset *within* each `buf_len`-wide
    sample record (not an absolute file offset)
  - `+8`: `count` (i32) — array length for vector variables (e.g. per-wheel). **The
    oracle ignores this and only ever decodes a single scalar per record** (ports
    faithfully — ignoring `count` means multi-element vars only get element 0; flagged
    as a known simplification inherited from the oracle, not a bug to "fix" in the
    Rust port without also fixing/porting a change upstream first)
  - `+12`: `count_as_time` (u8, 3 bytes padding) — unused by the oracle
  - `+16`: `name` (32 bytes, ASCII, NUL-padded)
  - `+48`: `desc` (64 bytes) — unused by the oracle
  - `+112`: `unit` (32 bytes, ASCII, NUL-padded)
- **Sample data**: a single buffer of `session_record_count` fixed-width records,
  each `buf_len` bytes, starting at `varBuf[0].buf_offset`. A variable's values are
  read with a strided view: byte `record_index * buf_len + var.offset`, width per its
  type. Timecodes are synthetic and uniform, `record_index * (1000 / tick_rate)` ms —
  matching MoTeC's `index / sample_rate` axis in shape, so `sde-core`'s existing
  `Channel.timecodes` contract (strictly increasing ms) needs no special-casing.
  `dec_pts = 2` and `interpolate = true` for float/double types (rtype 4/5); all other
  types are treated as discrete (`interpolate = false`, `dec_pts = 0`), mirroring
  MoTeC's `interpolate` split between continuous and event/state channels.
  A variable whose `unit` is exactly `"%"` is stored as a 0..1 ratio in the file; the
  oracle multiplies by 100 to get an actual percentage — ported as-is.
- **Session metadata is a trailing YAML document** (`session_info_offset` /
  `session_info_len`), *not* part of the fixed binary layout — confirmed ~14.7 KB of
  YAML in the real Mt. Washington capture, with `WeekendInfo.TrackDisplayName` (venue)
  and `DriverInfo.Drivers[].UserName` matched against `DriverInfo.DriverUserID` (the
  active driver, since replays/other cars' data can also appear in `Drivers`). The
  oracle only reads these two facts plus the binary header's `session_start_date` (for
  `Log Date`/`Log Time`, formatted via `time.localtime`); the Rust port deserializes
  only the needed sub-shape via `serde_yaml`, tolerating unknown fields, rather than
  modeling the entire session-info schema (dozens of unused sections:
  `SessionInfo`, `CameraInfo`, `RadioInfo`, `SplitTimeInfo`, `CarSetup`, etc.) —
  matching the oracle's own narrow read, and avoiding a large speculative surface no
  current feature needs. `Log Date`/`Log Time` are formatted from
  `session_start_date` treating it as UTC (not local time like the Python oracle,
  which depends on the *parsing* machine's timezone, not the recording sim's — an
  already-nondeterministic choice not worth replicating) — done with a small
  self-contained days/seconds-since-epoch decomposition rather than pulling in a date
  crate for two formatted strings.
- **Lap splitting is deliberately *not* part of `sde-ibt`** (unlike GPS
  zero-filtering, kept in the crate — see below), mirroring the MoTeC crate's split:
  `sde-motec` doesn't derive laps either, `sde-core::laps_from_beacon` does. IBT
  exposes an explicit `Lap` channel directly (confirmed present in both sample
  captures) plus `LapDist`/`Speed`, so `sde-core` gets an analogous
  `laps_from_lap_channel`, porting `_find_laps`'s "lap number changed, back-compute
  the exact crossing time from `LapDist`/`Speed` rather than trusting the lap-counter
  sample's own timecode" state machine verbatim (milestone 4/6 boundary: parser crate
  stays format-only, `sde-core` remains the one place session-level derived state —
  laps for every format — lives).
- **GPS zero-filtering (`_filter_gps`) stays in `sde-ibt`, not `sde-core`** — unlike
  lap-splitting, this is a per-format data-quality fixup (iRacing logs `Lat`/`Lon`/`Alt`
  as `0` before GPS lock acquires, or for cars/replay data that never had it), not a
  cross-format derived-state concern, so it belongs with the rest of the format-specific
  decode step, applied only when all three of `Lat`/`Lon`/`Alt` are present.
- **Rallycross Joker lap** (Hell RX sample): confirmed iRacing's `Lap` channel just
  increments through a Joker lap like any other lap — no separate "took the joker"
  channel found in the sampled var list, so distinguishing a Joker lap from a normal
  one (if ever needed) would have to come from track-geometry/distance analysis in a
  later milestone, not from anything `sde-ibt` can expose directly.
- Both real sample files (`.sample-data/iRacing/Hell RX/…`, `.sample-data/iRacing/Mt
  Washington Hillclimb/…`) parse successfully against this layout (manually verified,
  not committed as test fixtures — the smallest is ~4.8 MB, too large for
  `tests/fixtures/`). A small hand-built synthetic `.ibt` buffer with known values is
  the committed fixture instead (no existing library can round-trip-write real IBT
  files the way `ldparser` does for `.ld`), same pattern as MoTeC's `synthetic.ld`.
  Getting real files down to a trimmed fixture size (few samples, few channels) remains
  a **manual follow-up**, not a blocker.
  - The Mt. Washington capture: 26207 records, altitude climbs 465 m → 1878 m
    (a hillclimb, not a lap circuit — matches expectations), `SessionTime` starts at
    ~29 s (countdown before the timed run) and the stage completes at ~466 s.
  - The larger Hell RX capture: 29660 records, `Lap` running 0..11 (several laps plus
    the Joker lap the user drove — iRacing's `Lap` channel just increments through a
    Joker lap like any other, per the finding above). The smaller same-track capture
    parses to 0 records (`session_record_count = 0`) — a session that was started and
    immediately stopped; not a parser bug, just an empty recording.
  - **Non-ASCII venue names decode lossy, not oracle-faithfully**: `Lånkebanen`'s
    session-info YAML round-trips through `from_utf8_lossy` as `L�nkebanen` — the
    stream isn't valid UTF-8 despite the YAML body's own (unused-by-the-parser)
    `Encoding: ISO_8859_1` field. `serde_yaml`/PyYAML both assume UTF-8-or-BOM-detected
    input; neither actually honors that in-band field. The Rust port degrades
    gracefully (no panic, garbled string) rather than erroring — arguably better than
    the Python oracle, which would raise `UnicodeDecodeError` on this exact file.
    Full Latin-1-aware YAML decoding is a possible future improvement, not a blocker.

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
   *(IBT done, 2026-07-27 — see `crates/sde-formats/ibt`)*: parses the fixed
   header/variable-header-array/strided-sample-buffer layout plus the trailing
   session-info YAML block (`serde_yaml`, narrow struct — only `Driver`/`Venue`, not
   the full schema), ported from `TrackDataAnalysis/data/iracing.py`; see
   "IBT (iRacing) format findings" above for the full byte layout and the design
   choices (GPS zero-filtering stays in the crate, lap-splitting doesn't). Validated
   against real captures added to `.sample-data/iRacing/` (a Hell RX rallycross
   session with a Joker lap, a Mt. Washington Hillclimb stage) plus a committed
   synthetic fixture (`tests/fixtures/synthetic.ibt`, cross-checked against an
   independent Python re-decode). Wired into `sde-core::Session::load_ibt` — new
   `laps_from_lap_channel` ports `_find_laps`'s `Lap`/`LapDist`/`Speed`-based
   crossing-time back-computation (analogous to `laps_from_beacon` for MoTeC), and
   `dump_channels` now dispatches on file extension (`.ld` vs `.ibt`).
   *(IBT lap-splitting bug fix, 2026-07-27):* `sde-app`'s "Open file..." dialog only
   ever offered `.ld` — `.ibt` had no GUI path to load at all until this pass added an
   `.ibt` filter and extension-based dispatch to `main.rs`'s `load_file`, mirroring
   `dump_channels`'s existing dispatch. That exposed a real, previously-untested bug
   in `laps_from_lap_channel`: ported near-verbatim from TDA's `_find_laps`, it treats
   any transition to `Lap == 0` as "probably no more useful data" and stops looking
   for further laps right there — the real Hell RX rallycross capture (see above)
   disproves that assumption outright, its `Lap` channel going `0 -> 1 -> 0 -> 1 -> 2
   -> … -> 11` (a brief `Lap == 1` during the pre-green-flag formation movement,
   then the counter resets to `0` at the actual race start and counts up through
   eleven real racing laps afterward). The oracle's heuristic truncated lap
   detection to the session's first ~16.6s (2 laps) out of a real ~494s recording (the
   correct read is 16 boundaries/laps, averaging ~40s each, consistent with this
   rallycross layout). Fixed by treating every `Lap` value change uniformly —
   including a drop back to `0` — as an ordinary boundary, the same way
   `laps_from_beacon` and `sde-shtep`'s `laps_from_lap_number_channel` (below) already
   do, rather than special-casing zero — a deliberate, documented divergence from the
   Python oracle, not a straight port anymore. While in this function, also hardened
   the `LapDist`/`Speed` crossing-time back-computation against a `speed == 0`
   transition sample (the real capture has exactly this at its very last sample),
   which could otherwise divide out to a non-finite `f64` and corrupt every boundary
   after it — falls back to the sample's own raw timecode when the back-computed
   value isn't finite. Two new unit tests in `sde-core/src/lib.rs`
   (`lap_channel_tests`) cover both: the real capture's zero-reset shape, and the
   zero-speed division case. **Known cosmetic artifact, not fixed:** the real Hell RX
   file's very last sample has a one-sample `Lap` flicker (`0 -> 11`, real telemetry
   noise, not a parsing bug) that now surfaces as one spurious 0.0s-duration lap
   entry in the picker; no general "drop degenerate-length laps" filter was added to
   paper over it, since that's a judgment call better made with more real files to
   validate against, not guessed at from one capture's glitch.
   *(`shtep` TSV parser done, 2026-07-27 — see `crates/sde-formats/shtep`)*: not part
   of the original XRK/IBT/VBOX/ADULOG/RUN/MLG list above — added because the `../shtep`
   companion plugin (SimHub Telemetry Export Plugin) needs a Rust-side reader for what
   it writes. Plain text, no binary layout to reverse-engineer: a `{base}.tsv` (tab-
   separated, `Time_s` first column always, one configurable-subset column per
   canonical channel, matched by header name not position) plus a `{base}.meta.json`
   sidecar (session/car/driver metadata, `sessionType` "stage"/"stint",
   `schemaVersion`-gated backward compatibility — a newer-than-understood version is
   refused, everything else tolerated). No real plugin-exported fixture exists yet
   (the plugin is still being built, no `.tsv`/`.meta.json` pairs committed there) —
   the committed test fixture (`tests/fixtures/synthetic.tsv`/`.meta.json`) is
   hand-authored directly from `SCHEMA.md`, same role the other crates' synthetic
   fixtures play, swap for a real plugin export once one exists. Channel
   unit/interpolate/dec_pts metadata is derived from the header name itself (`_kmh`,
   `_pct`, `_deg`, `_mm`/`_m` suffixes; `Paused`/`Discontinuity`/`Gear`/`LapNumber`
   treated as discrete) since `SCHEMA.md` bakes units into the column name rather than
   declaring them separately, unlike MoTeC/IBT. Wired into `sde-core::Session::load_shtep`
   — `laps_from_lap_number_channel` is new, boundary-per-`LapNumber`-change (simpler
   than IBT's back-computed crossing time; `SCHEMA.md`'s fixed 100 Hz default sample
   rate makes the imprecision a small enough window not to be worth the extra
   complexity here) and only consulted for `"stint"` sessions — a `"stage"` recording
   is one continuous run by definition (stage-start to stage-end, one file), so it
   always gets a single whole-session lap. `KeyChannelMap::distance` now also checks
   `"LapDistance_m"`. Sidecar `discontinuities`/`rewinds` arrays are parsed and exposed
   on `ShtepFile` but not yet consumed by `sde-core` (both describe different things
   than RBR/NGP's `TimePenalty` — a discontinuity/rewind isn't a scoring deduction) —
   a documented follow-up, not forced into that model. `sde-app`'s "Open file..."
   dialog does **not** yet offer `.tsv` — out of scope for this pass, matching how the
   `.ibt` GUI wiring landed separately from the `sde-ibt` parser crate itself.
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

   *(Discrete-channel indicators — scoped 2026-07-27, not started.)* Today every
   channel, including discrete/state ones, is just another line-graph dock — there's
   no dedicated display for "what state is the car in right now at the cursor".
   Prompted by `sde-ibt` (milestone 4) exposing iRacing enum channels
   (`PlayerTrackSurface` = `irsdk_TrkLoc`: off-track/approaching-pits/on-track/etc.,
   `PlayerTrackSurfaceMaterial` = `irsdk_TrkSurf`: asphalt/dirt/gravel/grass/etc.,
   `TrackWetness` = `irsdk_TrackWetness`) plus plain analog ones worth calling out
   specially (`HandbrakeRaw`, `ClutchRaw`) — same category as the ABS/TC intervention
   channels MoTeC/RSF captures already carry, which today are equally just raw traces.
   Two-part scope:
   1. **Enum label mapping** — a small `channel name -> (raw value -> label)` lookup,
      *display-only*: `sde-ibt`/`sde-motec` keep returning raw numeric values (the
      format-parser boundary faithfully mirrors what TDA's oracles do, and staying
      numeric keeps match-expression/math-channel logic simple). Proposed home:
      `sde-core`, not `sde-app` — so `sde-cli`'s `dump_channels` can also print labels,
      and so it isn't duplicated if/when a second UI surface exists. **Blocked on
      sourcing the actual enum tables**: neither `TrackDataAnalysis/data/iracing.py`
      nor `../iracing-telemetry-tool` (this project's two iRacing reference repos)
      define them — they only read numeric values through, same as `sde-ibt` does now.
      Needs iRacing's own public SDK header (`irsdk_defines.h`, from the iRacing SDK
      download) as a new oracle before writing the mapping — do not guess at the
      variant order/values from memory; a wrong mapping (e.g. swapping `Gravel`/
      `Grass`) is worse than no label. New open validation task, tracked below.
   2. **UI indicator strip** — a small horizontal strip in `sde-app` (`ui/app.slint`),
      separate from the line-graph docks, showing current discrete-channel state at
      the shared cursor position: a colored surface swatch/icon, handbrake on/off,
      similar treatment for ABS/TC intervention markers. Reuses the cursor-position
      plumbing the per-dock readouts (`cursor-values`) already have; doesn't need a
      new interaction model, just a new small render target fed by the same lookup.
   Not started — this is a scoping note, not an implementation plan; sizing/sequencing
   happens when milestone 5 (or a follow-on) picks it up.

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

   *(Interaction pass, 2026-08-02 — from live testing feedback on a real
   RBR session):* three fixes/additions, all reported as "the UI does seem
   quirky".
   - **Setup panel didn't render.** The panel was wrapped in an `if
     root.setup-panel-visible:` inside the worksheet's `HorizontalBox`, so
     toggling it added/removed a layout *child* rather than resizing one.
     Now always instantiated and collapsed instead (`width:
     root.setup-panel-visible ? 340px : 0px; visible: ...`), matching how
     the sidebar — which never had the problem — sits in the same layout.
   - **Zoom/pan glitchiness.** Two causes. (a) The zoom applied a flat
     `0.9`/`1/0.9` factor per *event*, which is right for a mouse wheel
     (one event = one 120-unit notch) but wildly wrong for a trackpad
     emitting ~20 fractional-notch events per swipe: `0.9^20 ≈ 0.12`, a
     ~8x zoom from one gesture. Zoom (and pan) are now proportional to
     `delta / WHEEL_NOTCH`, so a notch does exactly what it always did and
     a partial notch does proportionally less. (b) The axis lock above
     decided from the gesture's *first* event, which on a trackpad is
     often a couple of jittery pixels — an intended pan whose first event
     favored vertical would zoom for the rest of the gesture.
     `graph::ScrollGesture` now accumulates until `AXIS_LOCK_THRESHOLD`
     (12) of movement before locking, then applies the accumulated delta
     so nothing is lost to the wait. One wheel notch clears the threshold
     immediately, so a mouse still zooms on its first event.
   - **Drag a dock by its header to reorder it; hold Ctrl to merge it into
     the dock you drop it on** — the answer to a worksheet cluttered with
     panels that could be consolidated. Slint 1.17 does have `DragArea`/
     `DropArea`, but their payload type is only reachable through
     `slint::private_unstable_api`, so this uses a plain `TouchArea` on
     the dock name (`MouseCursor.grab`) and puts the arithmetic in Rust
     with the rest of the layout math. `graph::drag_drop_target` turns the
     drag offset plus the dragged dock's own cell size into a target
     index — every layout mode places docks on a fixed pitch, so no hit
     testing in markup is needed — and that one index then feeds either
     `graph::reorder_docks` (move-to-index semantics) or
     `graph::merge_docks` (folds the source's channels into the target,
     skipping duplicates and keeping the target's order, then drops the
     source). All three are pure and unit tested, including the grid's
     don't-wrap-off-the-edge case and the rounds-to-zero case that makes a
     plain click on the handle harmless.
   - *Why a modifier rather than drop-between-docks zones* (considered,
     rejected): one gesture and one mental model beats two, drop zones get
     thin exactly when the worksheet is crowded enough to need them, and
     in the 2-column grid "between" is ambiguous (between two cells in a
     row, or between rows?). Because `drag_drop_target` returns a plain
     target index and says nothing about what the drop *does*, a modifier
     distinguishes the two actions at identical drag geometry for free.
     **Ctrl** specifically, because Ctrl+click already means "overlay" in
     the channel sidebar — same vocabulary, same result.
   - The modifier is sampled from `pointer-event` (which carries live
     `modifiers`) on every movement and reported alongside the drag
     offset, *not* read at release: whichever way the hovered dock is
     highlighted is then always what a release actually does. While
     dragging, the source fades; a merge target tints blue (its contents
     are about to change), a reorder target only outlines in amber (only
     the position is). Driven by two `-1`-means-none int properties plus
     one bool that Rust pushes back
     (`dock-drag-source`/`dock-drag-target`/`dock-drag-merges`).
6. **`sde-setup`** — setup sheet data model + diff view between two setups. First
   genuinely new (non-port) feature. *(Done, 2026-08-01 — see
   `crates/sde-setup`, `crates/sde-formats/rbr/src/setup.rs`,
   `crates/sde-app/src/setup_view.rs`.)* Four pieces:
   - **`.lsp` parser** (`sde_rbr::setup`, the location decided in the open-task
     entry below): an s-expression tokenizer rather than a line reader, so the
     copy embedded in a `.rpl` parses the same way if that's ever pursued.
     Handles the two real quirks — every section repeats its key list as a
     value-less trailer (a key with no values *is* the trailer marker, and is
     dropped), and `vecTopMountPosition` carries three values, so an entry
     holds a list. Number atoms are recognized narrowly (leading digit after an
     optional sign) rather than via `f64::from_str`, which would otherwise
     swallow a key spelled `inf`/`NaN` as a value of the entry before it.
   - **Sim-agnostic model** (`sde_setup::{Setup, SetupGroup, SetupEntry,
     SetupValue}`): ordered named groups of named entries, *not* a fixed
     struct of springs/dampers/ARBs. Every sim exposes a different subset of
     adjustments under different names; a fixed schema would either discard
     what doesn't fit or force adapters to invent values they don't have.
     Ordered groups also preserve each sim's own sheet layout — the one its
     users already know — while still giving the diff a stable key to match on.
   - **Diff** (`sde_setup::diff`): keeps only what differs, matched by key
     within a group (never by position — two sheets from different plugin
     versions can carry different entry counts), reporting one-sided entries
     rather than dropping them. `percent_change` divides by the left value's
     *magnitude* so its sign agrees with the delta for negative quantities
     (`WheelAxisInclination` -0.057 -> -0.070 is a -22% change, not +22%).
   - **RBR adapter** (`sde_setup::rbr`), living in `sde-setup` rather than
     `sde-rbr` so the format crates stay leaf dependencies — the same direction
     `sde-core` sits in relative to the telemetry parsers. Prettifies RBR's
     `CamelCase` keys (`"BumpStopStiffnessRear_NGP"` -> `"Bump Stop Stiffness
     Rear (NGP)"`, corner markers like `LF`/`RB` kept whole) and attaches units
     only where the captures confirm them (N/m, Pa, Ns/m, Nm, m). The angles
     and the ratio-like NGP values are deliberately left unlabelled: their
     magnitudes are *consistent* with radians and 0..1 ratios, but nothing
     confirms it, and a wrong unit on a setup sheet is worse than no unit.
   - **Surfaces**: `sde-cli` gained a `diff_setups` binary (one path prints the
     sheet, two print the diff with percentages), and `sde-app` a "Setup"
     panel — the loaded run's sheet, auto-resolved from the replay `.ini`'s
     `SetupName` against the install root (`sde_app::setup_view::
     resolve_setup_path`, which splits RSF's Windows-style relative path on
     both separators and also tries a relocated `saved_games_dir`), plus
     "Open setup..." / "Compare..." pickers that switch the panel to showing
     only what differs. This is the first half of the design note's principle 8
     (setup-diff-aware comparison); pairing it with a *performance* delta trace
     is still to come, and needs the comparison to span two loaded sessions
     rather than one.
   - **Correction to an earlier note:** the "Install-path discovery" section
     below says the two sample setups differ in "58 values in all". That was an
     undercount — the real figure is **73**, confirmed both by this parser and
     independently by a plain line diff of the two files. The other figures in
     that note (274 key/value pairs, 15 sections — 16 by this parser's count,
     which excludes the document's own `"CarSetup"` wrapper) hold.
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
  all — **the count is wrong, it's 73**; see the correction under milestone 6). Two parsing quirks: each section repeats its key list with *empty*
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

### UI/UX direction: telemetry dashboard, reimagined (design note, 2026-07-27)

Reviewed four screenshots of Pi Research's Pi Toolbox (a professional circuit-racing
telemetry tool) against the same iRacing Hell RX capture `sde-ibt` now parses, for
inspiration — **not** as a feature checklist to clone. Screenshots aren't committed
(personal captures of third-party software, not project sample data); the design
decisions below are. Per Anna: Pi Toolbox's interface itself reads as archaic —
dense 1990s-MDI window management, not a bar to match. The goal stays what
`PROJECT_PLAN.md`'s Overview already states: "a modern, clean UX… not just parity
with the dated original" — that principle now extends past the original
`TrackDataAnalysis` Qt tool to this newer reference point too.

**What to explicitly *not* copy** (the parts of Pi Toolbox's UX that are dated, not
just old-looking):
- **Floating MDI docks**, each independently draggable/resizable/closable with its
  own title bar, lock icon, maximize button — non-responsive by construction (every
  pane has a fixed pixel position that a window resize doesn't reflow), and the
  chrome-per-pane overhead dominates the screen at anything less than a large
  monitor.
- **Tab-per-preset workspaces** (`Driver` / `Driver Braking Data` / `Tyre Data` /
  `Throttle/Brake Analysis`) that are really four separate fixed dock layouts a user
  built by hand — switching "views" means switching to an entirely different,
  independently-laid-out screen, not a filtered/reconfigured version of one coherent
  dashboard.
- **Flat, exhaustive property dumps** (the `DriverInfo:Drivers:0:CarClassWeightPenalty`-
  style sidebar, the "Important Channel Values" table showing `No Value` rows for
  systems the car doesn't even have) — everything at equal visual weight, nothing
  curated or grouped, forcing the user to already know what they're looking for.
- **Skeuomorphic gauges** (analog speedometer-style dial for steering angle) that
  cost far more screen space than a number/sparkline for the same information
  density, and don't scale down.

**Design principles for `sde-app` going forward:**
1. **One responsive canvas, not floating windows.** The existing worksheet/dock
   model (milestone 5) is the right foundation — extend it, don't bolt an MDI
   window manager onto it. Docks reflow (stack, resize, collapse) as the window
   resizes, the way a modern web dashboard does, not as independently positioned
   panes that just get cut off.
2. **Saved views are filters/layouts over one dashboard, not separate screens.**
   Pi Toolbox's tab-per-preset becomes a picker that reconfigures which docks are
   visible and how they're arranged within the same responsive canvas — switching
   "Tyre Data" to "Braking Data" rearranges, it doesn't teleport to an unrelated
   fixed layout.
3. **Distance as the primary x-axis**, time as a togglable secondary mode — every
   Pi Toolbox dock reviewed plots against distance (m), confirming the gap already
   noted in milestone 3's deferred list. This matters more for rally/hillclimb
   content than it did for circuit racing: distance is stage-relative and
   comparable across runs even when pace (and therefore elapsed time at a given
   point) differs.
4. **"Run", not "Lap", as the primary unit — a rally-native model, not a
   circuit-racing one ported as-is.** Pi Toolbox's lap-selector bar assumes laps;
   this project's real content spans laps (rallycross, with a Joker lap — see the
   IBT findings above), a single continuous stage with no intermediate crossings
   (hillclimb, RBR/RSF), and eventually point-to-point WRC/Dirt Rally stages. The
   selector should present whatever `Session::laps` actually contains — one entry
   for a single-stage run, several for a lapped session — rather than assuming a
   lap-timed circuit and degrading awkwardly for stage formats.
5. **Indicators surface analysis, not just raw enums.** The "Bit Indicator"
   concept (validated as worth building — see the earlier "Discrete-channel
   indicators" milestone-5 note, which this section supersedes/absorbs) shouldn't
   stop at relabeling raw channel values (`PlayerTrackSurfaceMaterial` = "Gravel").
   Once `sde-analysis` exists (milestone 7), the same indicator strip is the
   natural home for *derived* engineering signals — ABS/TC intervention
   count/duration, brake-balance effectiveness — which is this project's actual
   differentiator per the Overview ("analyzing the vehicle, not just the driver").
   Raw-enum indicators can ship first as the milestone-5-scale version; the strip's
   design shouldn't paint itself into "raw channel passthrough only."
6. **Curated metadata, progressive disclosure.** Group session/setup/weather/car
   fields semantically and hide anything with no meaningful value by default
   (no `No Value` rows cluttering a glance-able summary); the full raw property
   list stays available (search/expand), it just isn't the default view.
7. **Track map as a configurable analysis overlay, not a fixed pedal-state
   palette.** `sde-gis` (currently unstarted) is the natural home for this; Pi
   Toolbox's throttle/brake-colored track ("Summit" dock) is one useful overlay
   mode among several worth supporting — ABS/TC intervention zones, or (once
   `sde-formats::rbr` parses pacenotes/setup) pacenote sector or setup-change
   markers laid over the same geometry.
8. **Setup-diff-aware comparison.** Pi Toolbox's lap-delta view is
   performance-only (`LapDeltaCompare` vs. distance) with no way to see *why* —
   because Pi Toolbox has no setup model. This project's `sde-setup` does. A
   comparison view that shows a performance delta trace next to what actually
   changed between the two runs' setups (front ARB, brake bias, etc.) is a
   genuinely differentiated feature Pi Toolbox structurally cannot offer, not
   just a restyle of what it already has.
9. **Visual language**: theme-aware (light/dark, matching the system/user
   preference — not Pi Toolbox's fixed dark-with-magenta-titlebars skin), a
   consistent, restrained color palette reused across every widget (graphs,
   indicators, track-map overlays) rather than each dock choosing its own
   red/green/purple/yellow scheme independently, and legible-at-a-glance
   information density (numbers/sparklines over gauges) — the same "read as one
   system" bar this project already holds visualizations to elsewhere.

Not started — this is a design note superseding the narrower "Discrete-channel
indicators" scoping under milestone 5 above (still correct in its specifics — enum
mapping layer in `sde-core`, blocked on sourcing `irsdk_defines.h` — just now framed
as one instance of principle 5 rather than a standalone feature). Sizing/sequencing
into concrete milestone-5/7/`sde-gis` work happens separately.

*(Principles 3 and 4 implemented, 2026-07-27):* distance x-axis mode and "Run"
terminology, the two most immediately implementable principles (the rest are
blocked on the enum source, or on `sde-gis`/`sde-setup`, both unstarted).
- **Distance axis (principle 3):** `sde_core::KeyChannelMap` gained a `distance:
  Option<String>` field, populated the same way as `speed`/`lat`/`long`/`alt` via a
  new `DISTANCE_CHANNEL_NAMES` candidate list (`["LapDist", "Distance"]` — IBT's own
  name first, since it's a confirmed real channel per the IBT findings above; a
  generic `"Distance"` fallback for MoTeC, unvalidated against real hardware, same
  caveat as `BEACON_CHANNEL_NAMES`). `sde-app`'s `graph.rs` gained `AxisMode` (`Time`
  default / `Distance`) threaded through `build_lap_comparison_plot` as two new
  parameters (`axis`, `distance_channel: Option<&Channel>`); lap/zoom *selection*
  stays entirely time-based (unchanged) — only the plotted x-coordinate switches
  from `t - start` to `distance_channel.value_at(t) - distance_channel.value_at(start)`
  per range, still independently rebased to `0` the same way time-mode already was.
  `AxisMode::Distance` with no distance channel available transparently behaves like
  `AxisMode::Time` — callers (and the UI toggle) don't need to branch on
  availability. Cursor lookups needed real new logic, not just a threaded parameter:
  a cursor drag's `fraction` now means "fraction along the distance axis," and two
  compared runs generally aren't at the same *elapsed time* at the same *distance* —
  that's the entire point of a distance comparison — so each range needs its own
  absolute time, found by inverting the distance channel. New `graph::time_at_distance`
  windows the distance channel to the specific `[start, end]` range first (via the
  existing `windowed_samples`) before searching — necessary because a channel like
  `LapDist` resets to `0` every lap and is therefore only monotonic *within* one lap;
  searching the raw unwindowed channel would feed `value_at_raw`'s bracket search a
  non-sorted array and silently return a bracket from the wrong lap (caught by a unit
  test, `time_at_distance_stays_within_its_own_lap_despite_the_reset`). `main.rs`'s
  `cursor_text_for_group` was refactored to take a list of already-resolved absolute
  times (one per range) instead of computing one shared `t_rel` internally, and a new
  `cursor_abs_times` picks between the old shared-offset logic (time mode) and the
  per-range inversion (distance mode). UI: a small "Axis: Time/Distance" toggle chip
  in `app.slint`'s layout-mode row, greyed out (but still clickable — falls back
  harmlessly) when the loaded session has no distance channel; the bottom cursor
  readout is now fully formatted in Rust (`"t = 12.3 ms"` / `"d = 45.6 m"`) since
  which one applies depends on the active mode. **Known limitation, not fixed:** a
  single "All" range spanning *multiple* laps would still sawtooth in distance mode
  (each lap's `LapDist` resets to 0) — out of scope for the rally/hillclimb
  single-stage content (exactly one lap, see below) this was built for; a genuinely
  multi-lap circuit session should stick to per-lap/comparison views in distance mode.
- **"Run" terminology (principle 4):** `graph::lap_labels` now special-cases a
  session with exactly one lap (a continuous stage/hillclimb run, not a lap-timed
  circuit) to return a single `"Full Run (…s)"` label instead of the previous
  `["All", "Lap 1 (…s)"]` pair showing the identical range twice under a
  circuit-assuming name. `index 0` still means "the whole session" either way, so no
  downstream indexing changed. `app.slint`'s header label next to the lap/run
  `ComboBox` switches from `"Lap:"` to `"Run:"` when `lap-labels.length == 1`.

*(Default worksheet panels, 2026-07-27):* a freshly loaded session previously started
either from one bare channel (`pick_default_channel`'s alphabetical-first-interpolating
pick) or a fully empty worksheet — neither matches the "glance-able default view"
every established telemetry tool (MoTeC i2, Pi Toolbox) opens with. New
`graph::default_dock_channels` builds one dock per role in a fixed
`DEFAULT_DOCK_ROLES` list (speed, RPM, throttle, brake, gear, steering) that the
session actually has a channel for — skipping roles with no match rather than
leaving a blank dock. Each role is itself a candidate-name list checked in order,
since the same channel is named differently per format (`"Ground Speed"` in MoTeC,
`"Speed"` in IBT, `"Speed_kmh"` in `shtep`; `"RPM"` vs `"Engine0_RPM"`, etc.) — this
can't be a single flat name list the way `KeyChannelMap::distance`'s lookup is,
since here *multiple* different roles each need their own resolved name. Falls back
to `pick_default_channel`'s old single-channel behavior (as one dock) only if *none*
of the roles match anything, so an unfamiliar channel naming scheme still doesn't
start from a blank canvas. `main.rs`'s `load_file` swaps this in for the old
single-channel default; no other worksheet/dock plumbing changed, since
`default_dock_channels` returns exactly the `Vec<Vec<String>>` shape
`AppState.dock_channels` already expects.

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
- [x] **Decide how replay metadata reaches the app (2026-07-31):** `sde-app`
      is the pairing point, confirmed by implementing the next item there —
      it's the one crate allowed to depend on both `sde-core` (for `Session`)
      and `sde-rbr` (for `ReplayInfo`) without violating `sde-core`'s
      deliberate non-dependency on `sde-rbr`. Added `sde-rbr` as an `sde-app`
      dependency. *(UI wiring landed 2026-07-31, see below — was originally
      scoped as decision-only.)*
- [x] **Cross-check `ReplayInfo::recovery_spots` against
      `Session::time_penalties` (2026-07-31):** new `sde_app::replay_check`
      module (`crates/sde-app/src/replay_check.rs`, Slint-free and unit
      tested independent of any UI) — `cross_check_recoveries(&Session,
      &ReplayInfo) -> RecoveryCrossCheck` pairs the two lists by
      chronological order (both are already ordered — recovery spots by file
      position, time penalties by timecode) rather than by nearest position,
      since matching by position is exactly what this validates, not an
      assumption to bake in. Reports whether the counts match, and per pair,
      whether the replay's recorded `position_m` agrees (within a 50 m
      `POSITION_TOLERANCE_M`, generous because the replay's position is where
      the recovery prompt was accepted, not the exact stage-clock jump point)
      with the session's distance channel evaluated at the telemetry
      penalty's timecode (via `sde_app::graph::value_at`, reused rather than
      re-implemented). A session with no distance channel reports `None` for
      that comparison — uninformative, not a mismatch, so
      `RecoveryCrossCheck::looks_consistent()` still passes rather than
      penalizing formats/sessions that never had a distance channel to begin
      with.
- [x] **UI wiring for install root / replay info (2026-07-31):** `sde-app`
      gained two header buttons — "RBR install root..." (folder picker ->
      `InstallConfig::new(root).resolve()` -> `validate()` +
      `read_ngp_settings()`, shown as a status line: root validity, count of
      missing expected paths, and the NGP recording-on/off toggle) and "Open
      replay info..." (file picker filtered to `.ini`, defaulting to
      `install_paths.replays_dir` when a root is set, same way "Open file..."
      now defaults to `install_paths.ngp_telemetry_dir`) — plus a status row
      in `app.slint` showing both. Loading a replay `.ini` (or loading a new
      telemetry file while one is already loaded) calls
      `refresh_replay_status`, which runs `replay_check::cross_check_recoveries`
      against the current session whenever both are loaded and reports
      `"consistent"` or `"MISMATCH"` alongside the replay's stage/car/driving
      time. **Still manual, not automatic**: the app does not auto-pair a
      loaded `.ld`/`.tsv` with a `.rpl`/`.ini` — the user opens each
      independently (still no known filename/folder convention to pair them
      by, per the note above; `install_paths` only narrows *which folder*
      each picker starts in). One real bug caught and fixed while wiring
      this in: `load_file`'s failure path unconditionally reset the whole
      `AppState` to `default()`, which would have silently discarded an
      already-configured install root and loaded replay info on a bad file
      pick — now preserves both across a failed load. Verified by hand
      (real `.ld`/`.rpl`/`.ini` triple from `.sample-data/`): both buttons
      render, the install status line shows the root path and "Telemetry
      recording: ON", and pairing a `.ld` with a mismatched `.ini` correctly
      surfaces as "MISMATCH" in the replay status line.
- [x] **Three bugs found in live testing, fixed (2026-07-31):** Anna tested
      the above UI wiring directly and found three real problems, all fixed
      in the same pass:
      1. **Default worksheet showed only one panel for RSF/NGP sessions.**
         `graph::DEFAULT_DOCK_ROLES` (milestone 5's default-dock-channels
         list) only knew MoTeC/IBT/shtep naming conventions
         (`"Ground Speed"`/`"Speed"`/`"Speed_kmh"`, etc.); RSF/NGP's own
         lowercase dotted field names (`speed`, `engineRotation`,
         `throttle`, `brake`, `gear`, `steering` — confirmed via
         `dump_channels` against the real `.sample-data/RBR/…/Run1/motec/
         *.ld` capture) matched none of them, so every RSF session fell
         through to the single-channel fallback. Fixed by adding the
         confirmed real names to each role's candidate list; new
         `default_dock_channels_picks_one_match_per_role_rsf_ngp_style_names`
         regression test in `graph.rs` uses the exact confirmed names.
      2. **Install root wasn't remembered across app restarts.** No
         persistence existed at all — every launch required re-picking the
         root via the button. New `sde_app::config` module
         (`crates/sde-app/src/config.rs`) writes the chosen root to
         `%APPDATA%\sde-app\install_root.txt` (best-effort — a failed
         write just means it isn't remembered next time, not a hard
         error) and `main()` auto-loads it at startup through the same
         `apply_install_root` helper the button uses (extracted from the
         button's closure so both paths report identically). Four new
         unit tests (round-trip, missing file, empty file, overwrite).
      3. **"Open replay info..." appeared to do nothing** on Anna's first
         live test. Root cause not fully isolated (the cross-check logic
         itself checks out under its own unit tests and worked correctly
         once retested), but a real related bug was found and fixed while
         investigating: `load_file`'s failure path (see the entry above)
         reset the whole `AppState`, which — before this fix — would have
         made a just-loaded replay's info vanish the moment *any*
         subsequent file load failed, which could plausibly look like "the
         button did nothing" if a failed load happened to follow it.
      All three verified live afterward (Anna confirmed the default
      multi-panel view; a fresh headless-launch screenshot showed the
      install root and NGP recording status auto-populated with no
      button click, proving the persistence round-trip).
- [x] **Oscilloscope-style trace color rotation (2026-07-31):** every
      non-overlay, non-comparison dock previously rendered its single
      trace in the same color (`SERIES_COLORS[0]`, always blue) — the
      per-series color index reset to `0` at the start of every dock. Per
      Anna's request, `replot` in `main.rs` now threads a single
      `worksheet_color_index` across the whole dock loop instead of a
      fresh `color_index` per dock, so successive docks/channels cycle
      through the 8-entry palette like an oscilloscope (first channel
      blue, second orange, third green, ...). Deliberately **not** applied
      during lap comparison: there, color already has an established,
      different meaning (which lap, via the shared top-level legend built
      in `refresh_compare_ui`, indexed by position in
      `compare_lap_indices`) that's consistent across every dock — mixing
      the two would desync a dock's trace color from what that legend
      says it represents. `replot` decides `comparing = ranges.len() > 1`
      once (same `ranges` list is shared by every dock in one call) and
      only advances `worksheet_color_index` when `!comparing`, so
      comparison mode's per-range coloring (still starting fresh at `0`
      per dock) is unchanged. An empty/no-data dock doesn't consume a
      color slot. Verified live: a freshly loaded shtep session's default
      three-panel view (`Speed_kmh`/`RPM`/`Throttle_pct`) now renders
      blue/orange/green respectively instead of all blue.
      *(Bug found by independent code review, fixed 2026-08-01):* `comparing`
      was computed as `ranges.len() > 1`, but a user can toggle on exactly
      **one** compare chip (nothing stops them, and `refresh_compare_ui`'s
      status text already special-cases "Comparing 1 lap" grammar), which
      also yields exactly one range — so a single-lap comparison was
      silently treated as the plain case, letting the color rotation
      advance past dock 0 and desync every dock after the first from the
      legend's single swatch (which always starts at color 0 for that one
      lap). Fixed by keying `comparing` on `!state.compare_lap_indices.is_empty()`
      instead of the range count — the actual selection state, not a
      count that happens to coincide with it in the common multi-lap case.
- [x] **Auto-locate the matching replay `.ini` (2026-08-01):** per Anna's
      request, after testing "Open replay info..." and realizing she'd
      expected it to auto-populate rather than need a second manual pick.
      New `sde_rbr::pairing` module (`crates/sde-formats/rbr/src/
      pairing.rs`) — `find_matching_replay_ini(telemetry_path, replays_dir)`
      matches by modification-time proximity, since RSF/NGP has no shared
      filename/folder convention between a `.ld`/`.tsv` and its `.rpl`/
      `.ini` (confirmed no correlation between
      `telemetry-NGP-Car_5-Mini JCW WRC-Stage_450-...` and
      `Anna_rsf_hotlap_Gabiria-Legazpi 2004_...` filenames). Timing *is*
      shared, though: checked real file mtimes from both sample captures
      (`PROJECT_PLAN.md`'s "RSF real-capture validation" section) — each
      run's `.ld` trails its own `.ini` by 6s and 41s respectively, while
      the two different runs in that capture are ~7 minutes apart. A
      180s tolerance window (comfortably above the largest observed gap,
      comfortably below the inter-run gap) with nearest-match-wins is
      therefore a workable heuristic, not a guarantee — flagged as such in
      the doc comment, and still only as good as the real distribution of
      gaps this was tuned against. Five unit tests using real
      `std::fs::File::set_modified` temp files (closest-within-tolerance
      picked over a further-but-still-in-range one matching the two real
      runs' gaps, out-of-tolerance is `None`, non-`.ini` files ignored,
      missing telemetry file/replays dir don't panic).
      `sde-app`'s `load_file` now calls this automatically whenever
      `install_paths` is set, replacing (not just adding to) whatever
      `replay_info` a previous file load left behind — a stale replay
      belongs to a different run and would otherwise silently mismatch (see
      `AppState::replay_info`'s doc comment: reset on every successful
      load, still preserved across a *failed* one). A new
      `AppState::replay_auto_match_gap` records how tight the winning
      match was, shown in the status line (`"Replay (auto-matched,
      Δ40s): ..."`) so the user can judge confidence; `None` (and no
      "(auto-matched, ...)" prefix) when `replay_info` came from the
      manual "Open replay info..." button instead. Verified live against
      the real install (`C:\Richard Burns Rally\Replays\` /
      `Plugins\NGP\telemetry\`, not just `.sample-data/`): loading a real
      `.ld` auto-matched its replay at `Δ40s` and displayed correctly.
- [x] **Surface more of the replay `.ini`'s fields (2026-08-01):** the
      replay status line (`refresh_replay_status` in `main.rs`) previously
      showed only stage/car/driving-time. Per Anna's question about what
      else is in `ReplayInfo`, added setup file name (`car.setup_name`,
      last path segment only via a new `path_basename` helper — RSF's own
      values use `\` regardless of host OS, so `Path::file_name` isn't
      reliable here), surface/weather conditions
      (`tyre_type`/`weather_type`/`surface_wetness`), and NGP physics
      version (`versions.ngp`) — the fields that decide whether two runs
      are *actually* comparable (matches `PROJECT_PLAN.md`'s "UI/UX
      direction" design note, principle 6: curated metadata, not a flat
      property dump; this is also the raw material principle 8's
      setup-diff-aware comparison would consume, once `sde-setup` exists).
      Left out for now as less immediately actionable: map length, rally
      type/name, sky type, surface age, RSF launcher version — easy
      follow-ons, not modeled as a gap since `ReplayInfo` already carries
      them, just not surfaced in this status line yet.
- [ ] Capture an RSF run on a *gravel/snow* stage and a different car, to check
      whether the 10^6 fixed-point field list holds beyond this one tarmac/Mini
      combination. Anna is capturing these separately. (The 1009-row pre-start
      window is no longer open: `totalSteps` = 5050 at the stage start in both
      runs, so the countdown is a fixed physics-tick count.)
- [x] **Install-root configuration + path discovery (2026-07-31)** — new
      `sde_rbr::install` module (`crates/sde-formats/rbr/src/install.rs`,
      UI-free per the design note above). `InstallConfig { root, overrides }`
      resolves to `InstallPaths` (one field per row of the path table above:
      NGP telemetry dir, `Telemetry.ini`, `RichardBurnsRally.ini`, `Replays\`,
      `SavedGames\`, `Plugins\Pacenote\`, `rsfdata\cars\`, `Maps\`,
      `RallySimFans.ini`, `rallysimfans_personal.ini`) — `PathOverrides` lets
      any single path be redirected without affecting the others (e.g. a
      telemetry folder relocated to another drive). `validate(&InstallPaths)
      -> ValidationReport` checks every path against the filesystem and
      reports *all* that are missing rather than stopping at the first one,
      matching the design note's "report which expected paths are missing
      rather than failing wholesale." `ValidationReport::root_looks_valid()`
      narrows this to the two cheapest, most load-bearing markers
      (`RichardBurnsRally.ini` + `Plugins\NGP\Telemetry.ini`) — an install
      missing `Maps\` or an unconfigured `Plugins\Pacenote\` isn't "wrong
      root," but missing both of those two almost certainly is. Also added
      `read_ngp_settings(&InstallPaths) -> NgpSettings` (via the existing
      `ini::Ini` reader, no new dependency) exposing `RichardBurnsRally.ini`'s
      `[NGP]` `telemetryRecording`/`telemetryTics` keys — the design note's
      "'telemetry recording is currently off' is a much better first-run
      experience than an empty file list" is now backed by an actual reader.
      Ten new unit tests (path resolution, per-path override isolation,
      validation reporting every miss, root validity, NGP settings
      happy/missing-key/missing-file paths) using real temp directories
      rather than mocking the filesystem, consistent with this crate's
      existing on-disk test style. *(UI wiring landed 2026-07-31 — see the
      "UI wiring for install root / replay info" entry above: a "RBR install
      root..." picker surfaces `validate()`/`read_ngp_settings()` as a status
      line, and both "Open file..." and "Open replay info..." now default to
      the relevant resolved folder.)* **Still not done:** no settings
      screen (the install root isn't persisted across app restarts — it's
      re-picked each session), and no auto-pairing of a loaded `.ld`/`.tsv`
      with its matching `.rpl`/`.ini` (still two independent manual pickers,
      per the note above — no known filename/folder convention to pair them
      by).
- [ ] Consider reading the NGP `.tsv` directly rather than the converted `.ld`,
      to recover `utcSystemTime` (absolute wall clock — the natural anchor for
      `sde-video` sync), `totalSteps`, and untruncated channel names.
- [x] **Decide where the `.lsp` setup parser lives, and which copy to read
      (2026-07-31):** lives in `sde-formats::rbr` (a new `setup.rs` module,
      alongside `replay.rs` and `ini.rs`) — same crate as the other RBR/RSF
      companion-file readers, per the workspace's per-sim adapter pattern.
      **Read the standalone `.lsp` file, not the copy embedded in `.rpl`,
      for the first implementation.** The embedded copy is more
      self-describing (can't drift from the run it's paired with) but
      extracting it means locating it inside the mostly-unreverse-engineered
      `.rpl` binary body — the "RBR NGP telemetry format findings" section
      above documents the embedded offset for exactly *one* observed file
      (396376) with no confirmed rule for finding it in another, since the
      surrounding `.rpl` layout itself isn't understood well enough yet to
      derive that offset generally. The standalone `.lsp` needs no such
      dependency: it's plain Lisp-style text at a fixed, already-parseable
      shape (see "Install-path discovery" section above — 274 key/value pairs
      over 15 sections, trailing empty-value duplicates to skip, three
      trailing NULs on one sample file). Once `.rpl` decoding is itself a
      real feature (not scoped yet), revisit reading the embedded copy as a
      drift-check against the standalone file — the same
      cross-validation spirit as this section's recovery-spot cross-check —
      but that's a follow-on, not a blocker for a first `.lsp` parser.
      *(Implemented 2026-08-01 exactly as decided here — `sde_rbr::setup`
      reads the standalone file; see milestone 6 above for the parser's
      design and the two file quirks it handles. The `.rpl`-embedded copy
      remains untouched.)*
- [ ] **Discrete-channel indicators (2026-07-27, scoped, not started):** source
      iRacing's official `irsdk_defines.h` (from the iRacing SDK download — not
      currently a local reference repo) for the real `irsdk_TrkLoc`/`irsdk_TrkSurf`/
      `irsdk_TrackWetness` enum tables before writing the `sde-core` label-mapping
      layer described in milestone 5 above. Neither of this project's two iRacing
      reference repos (`TrackDataAnalysis/data/iracing.py`, `../iracing-telemetry-
      tool`) define these enums — both only pass the numeric values through.
