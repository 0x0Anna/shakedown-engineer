use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ExpectedChannel {
    freq: u16,
    unit: String,
    count: usize,
    values: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct ExpectedFixture {
    channels: HashMap<String, ExpectedChannel>,
}

/// float32-precision epsilon: the synthetic fixture's values were
/// computed in Python float64 then cast down to float32 before being
/// written to the `.ld` file, and our parser reads them back as f32
/// then widens to f64 — so exact f64 equality isn't expected, only
/// agreement to float32 precision.
const EPSILON: f64 = 1e-3;

#[test]
fn parses_synthetic_fixture() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let ld_path = fixture_dir.join("synthetic.ld");
    let expected_path = fixture_dir.join("synthetic_expected.json");

    let expected: ExpectedFixture = {
        let text = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("failed to read {expected_path:?}: {e}"));
        serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("failed to parse {expected_path:?}: {e}"))
    };

    let ld_file =
        tda_motec::parse(&ld_path).unwrap_or_else(|e| panic!("failed to parse {ld_path:?}: {e}"));

    assert_eq!(
        ld_file.channels.len(),
        expected.channels.len(),
        "channel count mismatch"
    );

    for channel in &ld_file.channels {
        let exp = expected
            .channels
            .get(&channel.name)
            .unwrap_or_else(|| panic!("unexpected channel {:?} in parsed file", channel.name));

        assert_eq!(
            channel.sample_rate, exp.freq,
            "sample_rate for {}",
            channel.name
        );
        assert_eq!(channel.unit, exp.unit, "unit for {}", channel.name);
        assert_eq!(
            channel.values.len(),
            exp.count,
            "sample count for {}",
            channel.name
        );
        assert_eq!(
            channel.timecodes.len(),
            exp.count,
            "timecode count for {}",
            channel.name
        );

        for (i, (actual, expected_val)) in channel.values.iter().zip(exp.values.iter()).enumerate()
        {
            assert!(
                (actual - expected_val).abs() < EPSILON,
                "{}[{i}]: expected {expected_val}, got {actual}",
                channel.name
            );
        }

        // Timecodes should be i * (1000 / sample_rate) ms.
        // `i as f64`: sample counts here are tiny (test fixture, tens of
        // samples), nowhere near f64's exact-integer range limit.
        #[allow(clippy::cast_precision_loss)]
        for (i, tc) in channel.timecodes.iter().enumerate() {
            let expected_tc = i as f64 * (1000.0 / f64::from(exp.freq));
            assert!(
                (tc - expected_tc).abs() < EPSILON,
                "{}: timecode[{i}] expected {expected_tc}, got {tc}",
                channel.name
            );
        }
    }

    // Sanity-check a couple of the header-derived metadata fields against
    // what `ldparser`'s `frompd()` synthetic-fixture generator wrote.
    assert_eq!(ld_file.metadata.driver, "testdriver");
    assert_eq!(ld_file.metadata.vehicle, "testvehicleid");
    assert_eq!(ld_file.metadata.venue, "testvenue");
}
