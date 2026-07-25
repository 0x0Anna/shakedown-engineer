# Contributing

Shakedown Engineer is an early-stage, mostly-solo project (see [`PROJECT_PLAN.md`](./PROJECT_PLAN.md)
for the "why" and current milestone status). It's not yet at a point where the public API or
architecture is stable, so expect things to move — but issues, bug reports, and PRs are welcome.

## Before you start

- Read [`PROJECT_PLAN.md`](./PROJECT_PLAN.md) first. It's the source of truth for scope, design
  decisions, and what's already been validated (e.g. binary format layouts, conversion formulas) —
  not just a wishlist. If you're proposing a design that conflicts with something documented there,
  open an issue/discussion before sinking time into a PR.
- For anything sizable (a new feature, a new crate, a new file format parser), open an issue first
  so scope can be agreed before you write code.
- If you're adding a new telemetry log format parser under `crates/sde-formats/`, see the
  `.claude/skills/add-format-parser/SKILL.md` checklist — it captures the process (oracle
  selection, fixture generation, crate layout) used to build the MoTeC parser, and is meant to be
  followed whether or not you're using Claude Code.

## Workspace conventions

- **Modularity is load-bearing, not a style preference.** `sde-formats` and `sde-core` must stay
  UI-free and dependency-light; `sde-app` is the *only* crate allowed to depend on Slint. A PR that
  pulls a GUI dependency into a lower crate, or reaches from a format-parser crate into
  `sde-setup`/`sde-analysis`/`sde-gis`/`sde-video`, will be asked to change.
- Keep parser field names/shapes close to the original
  [`TrackDataAnalysis`](https://github.com/racer-coder/TrackDataAnalysis) Python dataclasses
  (`Channel`/`Lap`/`LogFile`) where a format is a port of that tool — see `PROJECT_PLAN.md` for the
  exact shape.

## Before opening a PR

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

All four must be clean — this matches CI exactly (`.github/workflows/ci.yml`), so a green run
locally means a green run on the PR.

- New parsers/format work should come with a test fixture (synthetic if real vendor/sim data can't
  be redistributed) and an integration test, not just manual verification — see
  `crates/sde-formats/motec/tests/` for the pattern.
- Update `PROJECT_PLAN.md`'s milestone checklist if your change completes or changes the status of
  a tracked item. Letting that file drift out of sync with reality is worse than not having it.

## Code of conduct

This project follows the [Code of Conduct](./CODE_OF_CONDUCT.md). By participating, you're expected
to uphold it.
