//! Edge cases for `Session`'s Beacon-derived lap splitting, beyond the
//! multi-lap fixture covered in `beacon_laps.rs`. These build a fake
//! `tda_motec::LdFile` directly in memory (all its fields are public)
//! rather than requiring a new `.ld` fixture, since we're exercising
//! `Session::from_ld_file`'s lap logic, not the MoTeC parser itself.

// These tests assert on exact floating-point boundary values that are
// either the literal `0.0` fallback or echoed straight back from the
// input timecodes — exact `==` (via `assert_eq!`) is correct here, not
// an approximation bug. `doc_markdown`: see the crate root's note.
#![allow(clippy::float_cmp, clippy::doc_markdown)]

use tda_core::Session;
use tda_motec::{LdChannel, LdFile, LdMetadata};

fn channel(name: &str, timecodes: Vec<f64>, values: Vec<f64>, interpolate: bool) -> LdChannel {
    LdChannel {
        name: name.to_string(),
        short_name: String::new(),
        unit: String::new(),
        sample_rate: 1,
        dec_pts: 0,
        interpolate,
        timecodes,
        values,
    }
}

fn ld_file(channels: Vec<LdChannel>) -> LdFile {
    LdFile {
        metadata: LdMetadata::default(),
        channels,
        file_name: "in-memory.ld".to_string(),
    }
}

#[test]
fn no_beacon_channel_falls_back_to_a_single_lap() {
    let speed = channel("Speed", vec![0.0, 10.0, 20.0], vec![1.0, 2.0, 3.0], true);
    let session = Session::from_ld_file(ld_file(vec![speed]));

    assert_eq!(session.laps.len(), 1);
    assert_eq!(session.laps[0].num, 0);
    assert_eq!(session.laps[0].start_time, 0.0);
    assert_eq!(session.laps[0].end_time, 20.0);
}

#[test]
fn beacon_channel_that_never_triggers_falls_back_to_a_single_lap() {
    // Values never go negative, so the state machine's "open a sequence"
    // branch (`v < 0.0`) is never entered and no boundary is ever pushed.
    let beacon = channel(
        "Beacon",
        vec![0.0, 10.0, 20.0, 30.0],
        vec![5.0, 5.0, 5.0, 5.0],
        false,
    );
    let speed = channel(
        "Speed",
        vec![0.0, 10.0, 20.0, 30.0],
        vec![1.0, 2.0, 3.0, 4.0],
        true,
    );
    let session = Session::from_ld_file(ld_file(vec![beacon, speed]));

    assert_eq!(session.laps.len(), 1);
    assert_eq!(session.laps[0].start_time, 0.0);
    assert_eq!(session.laps[0].end_time, 30.0);
}

#[test]
fn beacon_sequence_opened_but_never_closed_with_a_valid_code_yields_no_extra_boundary() {
    // Opens a sequence (negative value) and sees a >= 16384 value, but
    // the closing code is never 100 or 2 (the narrow upstream trigger),
    // so no lap boundary should be recorded despite the sequence opening.
    let beacon = channel(
        "Beacon",
        vec![0.0, 10.0, 20.0],
        vec![-1.0, 16400.0, 42.0],
        false,
    );
    let session = Session::from_ld_file(ld_file(vec![beacon]));

    assert_eq!(session.laps.len(), 1);
    assert_eq!(session.laps[0].start_time, 0.0);
    assert_eq!(session.laps[0].end_time, 20.0);
}

#[test]
fn empty_channel_set_yields_a_single_zero_length_lap() {
    let session = Session::from_ld_file(ld_file(vec![]));

    assert_eq!(session.laps.len(), 1);
    assert_eq!(session.laps[0].num, 0);
    assert_eq!(session.laps[0].start_time, 0.0);
    assert_eq!(session.laps[0].end_time, 0.0);
    assert!(session.channels.is_empty());
}
