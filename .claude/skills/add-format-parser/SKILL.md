---
name: add-format-parser
description: Use when adding a new telemetry log format parser under crates/sde-formats/ (e.g. AiM XRK, iRacing IBT, RaceLogic VBOX, ECUMaster ADULOG, Race Technology RUN, MegaLogViewer MLG) — establishes the pattern to follow, based on how sde-motec was built and validated.
---

# Adding a new sde-formats parser

This project, **Shakedown Engineer** (see `PROJECT_PLAN.md`), ports its telemetry
log parsing from [racer-coder/TrackDataAnalysis](https://github.com/racer-coder/TrackDataAnalysis)
— that stays general-purpose, format-by-format, independent of the rally-focused
setup/analysis features being built on top of it. Each supported log format gets
its own crate under `crates/sde-formats/<format>/`,
following the pattern established by `crates/sde-formats/motec/`. Use that crate as
the reference implementation.

## Process

1. **Find the oracle.** Prefer the original Python `TrackDataAnalysis` repo's own
   parser for the format (`../TrackDataAnalysis/data/<format>.py`) as the primary,
   authoritative reference — it's the tool actually being ported, actively
   maintained, MIT-licensed. Only fall back to third-party reverse-engineered parsers
   (e.g. `ldparser` for MoTeC) as a secondary cross-check, and treat any formula or
   layout disagreement between sources as a flagged risk, not a coin flip — dig into
   *why* they disagree (see `PROJECT_PLAN.md`'s "Validation findings" section for a
   worked example: a shift/offset conversion-formula discrepancy between `ldparser`
   and TDA's own parser).
2. **Document the validated byte layout** in `PROJECT_PLAN.md` before writing Rust,
   the same way the MoTeC channel record layout is documented there. Read the Python
   oracle's actual unpacking code (`struct.unpack_from` calls, explicit byte offsets)
   rather than trusting any pre-existing written spec — reconstructed docs for these
   formats are frequently wrong in subtle ways (see the MoTeC precedent).
3. **Generate a test fixture without requiring real hardware/sim exports.** If the
   Python oracle (or a cross-check library) supports writing the format back out
   (like `ldparser`'s `ldData.frompd(df).write(path)`), use that to generate a
   synthetic-but-format-valid fixture with known values, and cross-validate it
   parses identically through both Python implementations before trusting it. Real
   vendor/sim-exported files remain a valuable non-blocking follow-up — note them as
   a manual task in `PROJECT_PLAN.md` rather than blocking on acquiring them.
4. **Crate structure**, mirroring `sde-motec`:
   - `Cargo.toml` — keep dependencies minimal, no UI/GUI deps (this crate must stay
     usable standalone per `PROJECT_PLAN.md`'s modularity principles).
   - `src/raw.rs` — `binrw`-derived structs for genuinely fixed/sequential binary
     records.
   - `src/bytes.rs` (if needed) — offset-based helpers for sparse/pointer-chased
     data that doesn't fit a single contiguous struct (e.g. optional linked
     sub-records) — don't force `binrw` onto data that isn't actually laid out
     sequentially.
   - `src/error.rs` — format-specific error type.
   - `src/lib.rs` — public API. Keep field names close to TDA's `data/base.py`
     `Channel`/`LogFile` dataclasses so `sde-core` can wrap it consistently across
     formats — check `sde-motec`'s `LdFile`/`LdChannel` types for the shape to match.
   - `tests/fixtures/` — committed binary fixture(s) + a hand-written or
     oracle-derived `*_expected.json` with values to assert against.
   - `tests/integration.rs` — parse the fixture, assert every channel's
     name/unit/sample_rate/values against the expected JSON (use an epsilon
     comparison for float precision, not exact equality).
5. **Wire into `sde-core`** via a `Session::load_<format>` constructor once the
   parser crate itself is tested and green, matching `Session::load_motec`.
6. **Verify before reporting done:** `cargo build --workspace`, `cargo test
   --workspace`, `cargo clippy --workspace --all-targets` must all pass. Rust is
   installed but not always on PATH in a fresh shell on this machine — prepend
   `export PATH="$HOME/.cargo/bin:$PATH" &&` (bash) or
   `$env:Path += ";$HOME\.cargo\bin";` (PowerShell) if `cargo`/`rustc` isn't found.
7. **Update `PROJECT_PLAN.md`'s milestone checklist** to reflect what's done, the
   same way milestone 1/2 progress has been tracked — don't let the plan drift out
   of sync with actual repo state.

## Scope discipline

Don't add GUI code (`sde-app` is Slint-only, per `PROJECT_PLAN.md`) and don't reach
into `sde-setup`/`sde-analysis`/`sde-gis`/`sde-video` from a format-parser crate —
those are separate later milestones with their own scope.
