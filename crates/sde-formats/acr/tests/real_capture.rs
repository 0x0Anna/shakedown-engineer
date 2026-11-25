//! `sde_acr::parse` against a real `acr_export --csv` file.
//!
//! The fixture is an excerpt (60 of 88,775 rows, the header untouched) of a
//! recording made on AC Rally 0.7 — Skoda Fabia RS Rally2, Sweden — exported
//! by `acr_telemetry`'s `acr_export`. Rows are spread across the run, so the
//! excerpt covers standing start, driving and the stop. The full export is
//! 153 MB and was parsed whole during validation (196 channels x 88,775
//! samples, 279.1 s, no rows dropped).
//!
//! The excerpt was exported before `acr_telemetry`'s unit-label fix, so a few
//! unit strings in it are known to be wrong (temperatures labelled `C` that
//! are Kelvin, suspension travel labelled `mm` that is metres). The parser
//! passes labels through untouched, so these tests deliberately assert only
//! labels that were always right.

use std::path::Path;

fn fixture() -> sde_acr::AcrFile {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/acr_0_7_sweden_rally2_excerpt.csv");
    sde_acr::parse(&path).expect("the real export excerpt should parse")
}

#[test]
fn reads_the_metadata_preamble() {
    let f = fixture();
    assert_eq!(f.venue, "AC Rally");
    assert_eq!(f.vehicle, "Telemetry");
    assert_eq!(f.log_date, "21/11/2026");
    assert_eq!(f.log_time, "20:51:01");
    assert_eq!(f.sample_rate_hz, Some(333.0));
}

#[test]
fn every_data_column_becomes_a_channel_with_every_row() {
    let f = fixture();
    // 197 CSV columns minus `Time`.
    assert_eq!(f.channels.len(), 196);
    assert!(f.channels.iter().all(|c| c.values.len() == 60));
    assert!(f.channels.iter().all(|c| c.timecodes.len() == 60));
}

#[test]
fn timecodes_are_milliseconds_and_increase() {
    let f = fixture();
    let t = &f.channel("speed_kmh").unwrap().timecodes;
    assert!(t.windows(2).all(|w| w[1] > w[0]));
    // Last sampled row is about 262.7 s into the run.
    assert!((t[t.len() - 1] - 262_670.685).abs() < 1.0);
}

#[test]
fn values_and_stable_units_match_the_source() {
    let f = fixture();
    let speed = f.channel("speed_kmh").unwrap();
    assert_eq!(speed.unit, "km/h");
    let top = speed.values.iter().copied().fold(f64::MIN, f64::max);
    assert!((top - 144.4473).abs() < 1e-3, "top speed {top}");

    let rpm = f.channel("rpm").unwrap();
    assert_eq!(rpm.unit, "rpm");
    assert!(rpm.values.iter().any(|&v| v > 4000.0));

    assert_eq!(f.channel("wheel_pressure_FL").unwrap().unit, "psi");
    // Brake bias was stepped up and down during the run (0.50 .. 0.54).
    let mut bias: Vec<f64> = f.channel("brake_bias").unwrap().values.clone();
    bias.sort_by(f64::total_cmp);
    bias.dedup();
    assert!(bias.len() >= 5, "brake bias values {bias:?}");
    assert!((bias[0] - 0.5).abs() < 1e-9 && (bias[bias.len() - 1] - 0.54).abs() < 1e-9);
}

#[test]
fn channels_the_game_does_not_fill_still_parse_as_zeros() {
    // Tyre wear, damage and the per-zone tyre temperatures are zero in the
    // game's shared memory even after contact on 0.7 (see the ACR note under
    // milestone 9 in PROJECT_PLAN.md); they must load, not error or vanish.
    let f = fixture();
    for name in ["tyre_wear_FL", "car_damage_front", "tyre_temp_m_FL"] {
        let c = f.channel(name).unwrap_or_else(|| panic!("missing {name}"));
        assert!(
            c.values.iter().all(|&v| v == 0.0),
            "{name} should be all zero"
        );
    }
}
