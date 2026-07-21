//! Verifies `Session::load_motec` derives lap boundaries from a `Beacon`
//! channel via the state machine ported from TDA's `data/motec.py`
//! `MOTEC()` (see PROJECT_PLAN.md, milestone 1 notes).
//!
//! Fixture: `crates/sde-formats/motec/tests/fixtures/synthetic_with_laps.ld`,
//! generated the same way as the milestone-1 `synthetic.ld` fixture (via
//! `ldparser`'s `ldData.frompd(df).write(path)`), with a hand-constructed
//! `Beacon` channel exercising: two "real" lap triggers (`v == 100` and
//! `v == 2`), the `v == 100/2` bit-masking + `>= 8192` negative-wrap
//! correction, and a `v == 56` no-op (opens/closes a sequence without
//! recording a lap boundary — per the upstream "but not 56?" comment).
//! Expected boundaries were computed by re-running the equivalent Python
//! state machine against the actual re-parsed fixture data (see
//! `synthetic_with_laps_expected.json`, generated alongside the fixture).

// `doc_markdown`: see the crate root's note. `float_cmp` below compares
// two lap-boundary values that must be bit-identical by construction
// (the end of one lap and the start of the next are set from the same
// value) — exact `==` is the right check, not an epsilon bug.
#![allow(clippy::doc_markdown, clippy::float_cmp)]

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ExpectedLap {
    num: u32,
    start_time: f64,
    end_time: f64,
}

#[derive(Debug, Deserialize)]
struct ExpectedFixture {
    laps: Vec<ExpectedLap>,
}

#[test]
fn splits_laps_from_beacon_channel() {
    const EPSILON: f64 = 1e-6;

    let fixtures_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../sde-formats/motec/tests/fixtures");
    let ld_path = fixtures_dir.join("synthetic_with_laps.ld");
    let expected_path = fixtures_dir.join("synthetic_with_laps_expected.json");

    let expected: ExpectedFixture = {
        let text = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("failed to read {expected_path:?}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("failed to parse {expected_path:?}: {e}"))
    };

    let session = sde_core::Session::load_motec(&ld_path)
        .unwrap_or_else(|e| panic!("failed to load {ld_path:?}: {e}"));

    assert!(
        session.channels.contains_key("Beacon"),
        "fixture should have a Beacon channel"
    );

    assert_eq!(
        session.laps.len(),
        expected.laps.len(),
        "lap count mismatch: got {:?}, expected {:?}",
        session.laps,
        expected.laps
    );

    for (actual, exp) in session.laps.iter().zip(expected.laps.iter()) {
        assert_eq!(actual.num, exp.num, "lap num mismatch");
        assert!(
            (actual.start_time - exp.start_time).abs() < EPSILON,
            "lap {}: start_time expected {}, got {}",
            exp.num,
            exp.start_time,
            actual.start_time
        );
        assert!(
            (actual.end_time - exp.end_time).abs() < EPSILON,
            "lap {}: end_time expected {}, got {}",
            exp.num,
            exp.end_time,
            actual.end_time
        );
    }

    // Consecutive laps should be contiguous (end of lap N == start of lap N+1).
    for w in session.laps.windows(2) {
        assert_eq!(w[0].end_time, w[1].start_time, "laps should be contiguous");
    }
}
