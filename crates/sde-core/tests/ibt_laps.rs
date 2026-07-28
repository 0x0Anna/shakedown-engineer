//! Verifies `Session::load_ibt` derives lap boundaries from iRacing's
//! `Lap`/`LapDist`/`Speed` channels via the state machine ported from
//! TDA's `data/iracing.py` `_find_laps` (see PROJECT_PLAN.md's "IBT
//! (iRacing) format findings").
//!
//! Fixture: `crates/sde-formats/ibt/tests/fixtures/synthetic.ibt` — `Lap`
//! goes 0,0,1,1,2 over 5 samples at 10 Hz, with `LapDist`/`Speed` set so
//! each lap-number change lands exactly 50 ms before the sample where
//! it's observed (back-computed via `LapDist / Speed`), matching the
//! oracle's whole point: the lap counter's own timecode isn't the true
//! crossing instant. Expected boundaries were computed by an independent
//! Python re-implementation of `_find_laps` against the same decoded
//! fixture data (see `synthetic_expected.json`'s `laps` field).

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
fn splits_laps_from_lap_channel() {
    const EPSILON: f64 = 1e-3;

    let fixtures_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../sde-formats/ibt/tests/fixtures");
    let ibt_path = fixtures_dir.join("synthetic.ibt");
    let expected_path = fixtures_dir.join("synthetic_expected.json");

    let expected: ExpectedFixture = {
        let text = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("failed to read {expected_path:?}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("failed to parse {expected_path:?}: {e}"))
    };

    let session = sde_core::Session::load_ibt(&ibt_path)
        .unwrap_or_else(|e| panic!("failed to load {ibt_path:?}: {e}"));

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
        assert!(
            (w[0].end_time - w[1].start_time).abs() < EPSILON,
            "laps should be contiguous"
        );
    }

    let key = &session.key_channel_map;
    assert_eq!(key.speed.as_deref(), Some("Speed"));
    assert_eq!(key.lat.as_deref(), Some("Lat"));
    assert_eq!(key.long.as_deref(), Some("Lon"));
    assert_eq!(key.alt.as_deref(), Some("Alt"));

    assert_eq!(
        session.metadata.get("Driver").map(String::as_str),
        Some("Test Driver")
    );
    assert_eq!(
        session.metadata.get("Venue").map(String::as_str),
        Some("Synthetic Test Track")
    );
}
