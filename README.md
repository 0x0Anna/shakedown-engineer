# Track Data Analysis — Rust Port

A Rust + Slint port of [racer-coder/TrackDataAnalysis](https://github.com/racer-coder/TrackDataAnalysis),
a racing telemetry viewer (AiM XRK, MoTeC LD, ECUMaster ADULOG, RaceLogic VBOX, Race
Technology RUN, MegaLogViewer MLG, iRacing IBT), extended into a **race engineer tool
for rally**: creating and analyzing vehicle setups per stage (suspension, ABS/TCS/aero
electronics — not just driver performance), for use with Richard Burns Rally, Dirt
Rally, Assetto Corsa Rally, and EA WRC.

See [`PROJECT_PLAN.md`](./PROJECT_PLAN.md) for full architecture, crate layout,
milestone sequence, and format-validation notes. That file is the source of truth for
scope and design decisions — this README is just the quick-start.

## Status

Milestone 1–2 in progress: MoTeC `.ld` parsing and the core session data model.
See `PROJECT_PLAN.md`'s milestone checklist for current progress.

## Workspace layout

```
crates/
├── tda-formats/motec/   # MoTeC .ld binary parser (binrw-based), UI-free
├── tda-core/            # Session/Channel/Lap data model, wraps tda-formats
└── tda-cli/             # `dump_channels` example binary
```

Future crates (not yet started): `tda-setup`, `tda-analysis`, `tda-gis`, `tda-video`,
`tda-app` (Slint GUI), and additional `tda-formats::<format>` parsers.

## Building

Requires a stable Rust toolchain (install via [rustup](https://rustup.rs/)).

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

## CLI example

```sh
cargo run -p tda-cli --bin dump_channels -- path/to/log.ld
```

Prints driver/vehicle/venue metadata, lap count, and each channel's name, unit,
sample count, and first few values.

## Reference repos

Local clones used for format validation and domain modeling (see `PROJECT_PLAN.md`
for details on how each was used):

- `../TrackDataAnalysis` — the original Python tool being ported; primary oracle for
  the MoTeC LD format (`data/motec.py`, `data/base.py`).
- `../ldparser` — independent MoTeC LD parser, used for cross-validation and to
  generate synthetic test fixtures.
- `../race-engineer` — RBR domain model reference (`docs/data-model-rbr.md`) for the
  future `tda-formats::rbr` setup-file adapter.
