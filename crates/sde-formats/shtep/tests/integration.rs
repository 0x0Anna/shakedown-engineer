//! Parses the committed `synthetic.tsv`/`synthetic.meta.json` fixture
//! from disk (exercising `parse`'s file I/O and sidecar-path derivation,
//! not just `parse_str`'s in-memory tests in `src/lib.rs`) and asserts
//! every value against what's hand-computed from the fixture's own
//! content.

use std::path::Path;

#[test]
fn parses_synthetic_fixture() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/synthetic.tsv");
    let file = sde_shtep::parse(&path).unwrap_or_else(|e| panic!("failed to parse {path:?}: {e}"));

    assert_eq!(file.sim, "rbr");
    assert_eq!(file.session_type, "stage");
    assert_eq!(file.context, "Maantie 1");
    assert_eq!(file.car, "Ford Escort MkII");
    assert_eq!(file.driver, "Annalise");
    assert_eq!(file.file_name, "synthetic.tsv");
    assert!(file.discontinuities.is_empty());
    assert!(file.rewinds.is_empty());

    assert_eq!(file.channels.len(), 7);

    let speed = file.channel("Speed_kmh").expect("Speed_kmh channel");
    assert_eq!(speed.unit, "km/h");
    assert!(speed.interpolate);
    assert_eq!(speed.timecodes, vec![0.0, 10.0, 20.0, 30.0]);
    assert_eq!(speed.values, vec![0.0, 12.5, 25.0, 31.2]);

    let gear = file.channel("Gear").expect("Gear channel");
    assert!(!gear.interpolate);
    assert_eq!(gear.values, vec![1.0, 1.0, 2.0, 2.0]);

    let lap_distance = file
        .channel("LapDistance_m")
        .expect("LapDistance_m channel");
    assert_eq!(lap_distance.unit, "m");
    assert_eq!(lap_distance.values, vec![0.0, 0.125, 0.375, 0.700]);
}
