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
│   └── rbr/, dirt_rally/, acr/, ea_wrc/   # NEW: per-sim SETUP FILE adapters
│                                            # (read each sim's own install-dir car/track/
│                                            #  setup files, map into sde-setup's model —
│                                            #  same adapter pattern as telemetry parsers,
│                                            #  but for setup data instead of channel data)
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
        `shift == 0` throughout).
