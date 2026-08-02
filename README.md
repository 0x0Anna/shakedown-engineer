# Shakedown Engineer

A rally race-engineer tool: creating and analyzing vehicle setups per stage
(suspension, ABS/TCS/aero electronics — not just driver performance), for use with
Richard Burns Rally, Dirt Rally, Assetto Corsa Rally, and EA WRC.

Its telemetry log parsing layer (`sde-formats`/`sde-core`) is a Rust + Slint port of
[racer-coder/TrackDataAnalysis](https://github.com/racer-coder/TrackDataAnalysis)
(AiM XRK, MoTeC LD, ECUMaster ADULOG, RaceLogic VBOX, Race Technology RUN, MegaLogViewer
MLG, iRacing IBT) — kept general-purpose and sim-agnostic, since it's the shared
foundation both circuit and rally analysis are built on. The setup/analysis features
built on top of it (`sde-setup`, `sde-analysis`, per-sim setup adapters) are where the
project diverges from a straight port and focuses specifically on rally.

See [`PROJECT_PLAN.md`](./PROJECT_PLAN.md) for full architecture, crate layout,
milestone sequence, and format-validation notes. That file is the source of truth for
scope and design decisions — this README is just the quick-start.

![sde-app: multi-lap comparison with stacked channel docks](./content/screenshot.png)

## Status

Milestones 1–6 done:

- **Telemetry parsing** — MoTeC `.ld` (+ the `.ldx` lap sidecar), iRacing `.ibt`,
  shtep, and NGP's native `.tsv`, behind one `sde-core::Session` model.
- **GUI** — worksheets of channel docks (drag a dock by its header to reorder
  it, or Ctrl+drag to merge it into another as an overlay group), channel
  search, lap selection and
  comparison, math channels, timeline zoom/pan, and a time-or-distance x-axis.
- **RBR/RSF integration** — install-root discovery, replay `.rpl`/`.ini`
  metadata, and a cross-check of the replay's recovery spots against the
  telemetry's own time penalties.
- **Setups** — a `.lsp` parser, a sim-agnostic setup model, a diff engine, and
  a setup panel that auto-resolves the loaded run's sheet and can show only
  what differs from another.

Next is `sde-analysis` (milestone 7): damper velocity histograms, ABS/TC
intervention stats, ride-height/roll estimates, brake bias effectiveness. See
`PROJECT_PLAN.md` — its "Where things stand / what to pick up next" section is
kept current, and the milestone list below it has the full detail.

## Workspace layout

```
crates/
├── sde-formats/motec/   # MoTeC .ld binary parser (binrw-based) + .ldx sidecar, UI-free
├── sde-formats/ibt/     # iRacing .ibt parser
├── sde-formats/shtep/   # SimHub telemetry export parser
├── sde-formats/rbr/     # RBR/RSF companion files: install paths, replay .ini, .lsp setups, .tsv
├── sde-core/            # Session/Channel/Lap data model, wraps sde-formats
├── sde-setup/           # Sim-agnostic setup model + diff engine, and the RBR adapter
├── sde-cli/             # `dump_channels` and `diff_setups` example binaries
└── sde-app/             # Slint GUI shell — the only crate that depends on Slint
```

Future crates (not yet started): `sde-analysis`, `sde-gis`, `sde-video`, and
additional `sde-formats::<format>` parsers (XRK, VBOX, ADULOG, RUN, MLG).

## Building

Requires a stable Rust toolchain (install via [rustup](https://rustup.rs/)).

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```

## CLI example

```sh
cargo run -p sde-cli --bin dump_channels -- path/to/log.ld
```

Prints driver/vehicle/venue metadata, lap count, and each channel's name, unit,
sample count, and first few values.

```sh
cargo run -p sde-cli --bin diff_setups -- path/to/setup.lsp [path/to/other.lsp]
```

With one path, prints the setup sheet; with two, prints only the values that
differ, with deltas and percentages.

## Contributing

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for build/test/lint requirements and workspace
conventions. This project follows a [Code of Conduct](./CODE_OF_CONDUCT.md).

## Reference repos

Local clones used for format validation and domain modeling (see `PROJECT_PLAN.md`
for details on how each was used):

- `../TrackDataAnalysis` — the original Python tool this project's telemetry parsing
  layer ports; primary oracle for the MoTeC LD format (`data/motec.py`, `data/base.py`).
- `../ldparser` — independent MoTeC LD parser, used for cross-validation and to
  generate synthetic test fixtures.
- `../race-engineer` — RBR domain model reference (`docs/data-model-rbr.md`) for the
  `sde-formats::rbr` / `sde-setup::rbr` setup-file adapters.
