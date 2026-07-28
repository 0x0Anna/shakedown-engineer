use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ExpectedChannel {
    unit: String,
    values: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct ExpectedFixture {
    channels: HashMap<String, ExpectedChannel>,
    driver: String,
    venue: String,
    record_count: usize,
    tick_rate: f64,
}

/// float32 rounds every value through the fixture's own build step (see
/// `build_ibt_fixture.py`), so exact f64 equality isn't expected.
const EPSILON: f64 = 1e-3;

/// `Lat`/`Lon`/`Alt` in the fixture: rows where the fixture deliberately
/// planted `Lat == 0 && Lon == 0` (pre-GPS-lock samples) are stripped by
/// `filter_gps`, so those three channels are checked separately below
/// rather than against the raw (unfiltered) JSON values.
const GPS_CHANNELS: &[&str] = &["Lat", "Lon", "Alt"];

#[test]
fn parses_synthetic_fixture() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let ibt_path = fixture_dir.join("synthetic.ibt");
    let expected_path = fixture_dir.join("synthetic_expected.json");

    let expected: ExpectedFixture = {
        let text = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("failed to read {expected_path:?}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("failed to parse {expected_path:?}: {e}"))
    };

    let ibt_file = sde_ibt::parse(&ibt_path)
        .unwrap_or_else(|e| panic!("failed to parse {ibt_path:?}: {e}"));

    assert_eq!(
        ibt_file.channels.len(),
        expected.channels.len(),
        "channel count mismatch"
    );

    for channel in &ibt_file.channels {
        let exp = expected
            .channels
            .get(&channel.name)
            .unwrap_or_else(|| panic!("unexpected channel {:?} in parsed file", channel.name));

        assert_eq!(channel.unit, exp.unit, "unit for {}", channel.name);

        let expected_values: Vec<f64> = if GPS_CHANNELS.contains(&channel.name.as_str()) {
            // Rows 0 and 4 have Lat == Lon == 0 in the fixture (see
            // build_ibt_fixture.py) and get dropped by filter_gps.
            exp.values[1..4].to_vec()
        } else {
            exp.values.clone()
        };

        assert_eq!(
            channel.values.len(),
            expected_values.len(),
            "sample count for {}",
            channel.name
        );
        assert_eq!(
            channel.timecodes.len(),
            expected_values.len(),
            "timecode count for {}",
            channel.name
        );

        for (i, (actual, expected_val)) in channel.values.iter().zip(expected_values.iter()).enumerate()
        {
            assert!(
                (actual - expected_val).abs() < EPSILON,
                "{}[{i}]: expected {expected_val}, got {actual}",
                channel.name
            );
        }
    }

    // Every non-GPS channel shares the same uniform timecode axis:
    // record_index * (1000 / tick_rate) ms.
    let speed = ibt_file.channel("Speed").unwrap();
    assert_eq!(speed.values.len(), expected.record_count);
    for (i, tc) in speed.timecodes.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let expected_tc = i as f64 * (1000.0 / expected.tick_rate);
        assert!(
            (tc - expected_tc).abs() < EPSILON,
            "Speed: timecode[{i}] expected {expected_tc}, got {tc}"
        );
    }

    // interpolate/dec_pts split: float/double vars interpolate, everything
    // else (int, bool) doesn't.
    assert!(ibt_file.channel("Speed").unwrap().interpolate);
    assert_eq!(ibt_file.channel("Speed").unwrap().dec_pts, 2);
    assert!(!ibt_file.channel("Lap").unwrap().interpolate);
    assert_eq!(ibt_file.channel("Lap").unwrap().dec_pts, 0);
    assert!(!ibt_file.channel("OnPitRoad").unwrap().interpolate);

    assert!(ibt_file.channel("LapDist").is_some());

    assert_eq!(ibt_file.metadata.driver, expected.driver);
    assert_eq!(ibt_file.metadata.venue, expected.venue);
    assert_eq!(ibt_file.metadata.log_date, "07/27/2026");
    assert_eq!(ibt_file.metadata.log_time, "21:16:26");
}
