//! Cross-check between a replay's `[RunkiSpots]` recoveries (`sde-rbr`) and
//! the stage-time-penalty discontinuities detected independently in the
//! paired telemetry log (`sde-core`'s `Session::time_penalties`).
//!
//! Per `PROJECT_PLAN.md`'s open validation tasks: these two lists come from
//! entirely different sources (a text `.ini` sidecar vs. a discontinuity
//! detector run over the `.ld`'s scored-time channel) that *should* describe
//! the same recovery events for a matching pair of files. Agreement is
//! therefore evidence the `.ld`/`.tsv` and the `.rpl`/`.ini` describe the
//! same run; disagreement flags a mismatched pairing or a parsing bug in one
//! of the two independent detectors.
//!
//! Deliberately lives in `sde-app`, not `sde-core` or `sde-rbr`: it needs
//! both a `Session` and a `ReplayInfo` together, and `sde-core` deliberately
//! doesn't depend on `sde-rbr` (see PROJECT_PLAN.md's "how does replay
//! metadata reach the app" note) — this module, plus whatever UI eventually
//! surfaces it, is that pairing's home.

use sde_core::Session;
use sde_rbr::ReplayInfo;

use crate::graph::value_at;

/// How close a replay's recorded recovery position and the telemetry's
/// distance-channel value at the matching penalty's timecode need to be to
/// call them the same event. Generous on purpose: the replay records the
/// position where the "recover vehicle" prompt was accepted, while the
/// telemetry discontinuity is the stage-clock jump, so a driver coasting a
/// short distance between the two is expected, not an error.
const POSITION_TOLERANCE_M: f64 = 50.0;

/// One matched (or partially unmatched, if the two lists have different
/// lengths) pair between a replay's recovery spot and a telemetry-derived
/// time penalty, paired by chronological order.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RecoveryMatch {
    pub replay_position_m: Option<f64>,
    pub replay_penalty_secs: Option<f64>,
    pub telemetry_timecode_ms: Option<f64>,
    pub telemetry_penalty_secs: Option<f64>,
    /// The session's distance channel evaluated at
    /// `telemetry_timecode_ms`, if the session has a distance channel
    /// ([`sde_core::KeyChannelMap::distance`]). Compared against
    /// `replay_position_m` by [`RecoveryMatch::positions_agree`].
    pub telemetry_position_m: Option<f64>,
}

impl RecoveryMatch {
    /// Whether this pair's positions agree closely enough
    /// (within [`POSITION_TOLERANCE_M`]) to plausibly be the same recovery
    /// event. `None` when either side has no position to compare against —
    /// that's uninformative, not a mismatch.
    #[must_use]
    pub fn positions_agree(&self) -> Option<bool> {
        let (a, b) = (self.replay_position_m?, self.telemetry_position_m?);
        Some((a - b).abs() <= POSITION_TOLERANCE_M)
    }
}

/// Result of cross-checking a [`ReplayInfo`]'s recovery spots against a
/// [`Session`]'s independently-detected [`sde_core::TimePenalty`]s.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecoveryCrossCheck {
    pub replay_count: usize,
    pub telemetry_count: usize,
    /// `true` iff both sources report the same number of recovery events.
    /// The cheapest and most load-bearing check here — see
    /// `PROJECT_PLAN.md`.
    pub counts_match: bool,
    pub matches: Vec<RecoveryMatch>,
}

impl RecoveryCrossCheck {
    /// `true` iff the counts match and every paired position agrees (or is
    /// uninformative). A conservative "nothing looks wrong" signal, not a
    /// guarantee — see [`RecoveryMatch::positions_agree`].
    #[must_use]
    pub fn looks_consistent(&self) -> bool {
        self.counts_match
            && self
                .matches
                .iter()
                .all(|m| m.positions_agree().unwrap_or(true))
    }
}

/// Cross-check `replay`'s `[RunkiSpots]` recoveries against `session`'s
/// independently-derived stage-time penalties.
///
/// Pairs entries by chronological order (both lists are already ordered —
/// `recovery_spots` by file position, `time_penalties` by timecode) rather
/// than by nearest position, since matching by position is exactly the
/// thing this function is meant to validate, not assume.
#[must_use]
pub fn cross_check_recoveries(session: &Session, replay: &ReplayInfo) -> RecoveryCrossCheck {
    let distance_channel = session
        .key_channel_map
        .distance
        .as_ref()
        .and_then(|name| session.channels.get(name));

    let pair_count = session
        .time_penalties
        .len()
        .max(replay.recovery_spots.len());

    let matches = (0..pair_count)
        .map(|i| {
            let spot = replay.recovery_spots.get(i);
            let penalty = session.time_penalties.get(i);
            RecoveryMatch {
                replay_position_m: spot.and_then(|s| s.position_m),
                replay_penalty_secs: spot.and_then(|s| s.penalty_secs),
                telemetry_timecode_ms: penalty.map(|p| p.timecode_ms),
                telemetry_penalty_secs: penalty.map(|p| p.penalty_ms / 1000.0),
                telemetry_position_m: penalty.and_then(|p| {
                    distance_channel.and_then(|c| value_at(c, p.timecode_ms))
                }),
            }
        })
        .collect();

    RecoveryCrossCheck {
        replay_count: replay.recovery_spots.len(),
        telemetry_count: session.time_penalties.len(),
        counts_match: replay.recovery_spots.len() == session.time_penalties.len(),
        matches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sde_core::{Channel, KeyChannelMap, Lap, TimePenalty};
    use sde_rbr::replay::RecoverySpot;
    use std::collections::HashMap;

    fn session_with(time_penalties: Vec<TimePenalty>, distance: Channel) -> Session {
        let mut channels = HashMap::new();
        channels.insert(distance.name.clone(), distance.clone());
        Session {
            key_channel_map: KeyChannelMap {
                distance: Some(distance.name),
                ..Default::default()
            },
            channels,
            laps: vec![Lap {
                num: 1,
                start_time: 0.0,
                end_time: 1000.0,
            }],
            metadata: HashMap::new(),
            file_name: "test.ld".to_string(),
            time_penalties,
        }
    }

    fn distance_channel() -> Channel {
        Channel {
            name: "LapDist".to_string(),
            units: "m".to_string(),
            dec_pts: 1,
            interpolate: true,
            timecodes: vec![0.0, 1000.0, 2000.0, 3000.0],
            values: vec![0.0, 100.0, 200.0, 300.0],
        }
    }

    fn replay_with(recovery_spots: Vec<RecoverySpot>) -> ReplayInfo {
        ReplayInfo {
            recovery_spots,
            recovery_section_present: true,
            ..Default::default()
        }
    }

    #[test]
    fn matching_counts_and_close_positions_look_consistent() {
        let session = session_with(
            vec![TimePenalty {
                timecode_ms: 1000.0,
                penalty_ms: 35001.0,
            }],
            distance_channel(),
        );
        let replay = replay_with(vec![RecoverySpot {
            kind: Some("C4H".to_string()),
            position_m: Some(105.0), // within POSITION_TOLERANCE_M of 100.0
            penalty_secs: Some(35.0),
        }]);

        let result = cross_check_recoveries(&session, &replay);

        assert_eq!(result.replay_count, 1);
        assert_eq!(result.telemetry_count, 1);
        assert!(result.counts_match);
        assert_eq!(result.matches[0].telemetry_position_m, Some(100.0));
        assert_eq!(result.matches[0].positions_agree(), Some(true));
        assert!(result.looks_consistent());
    }

    #[test]
    fn mismatched_counts_are_flagged() {
        let session = session_with(
            vec![TimePenalty {
                timecode_ms: 1000.0,
                penalty_ms: 35001.0,
            }],
            distance_channel(),
        );
        let replay = replay_with(vec![]); // clean run per the replay, but telemetry saw a penalty

        let result = cross_check_recoveries(&session, &replay);

        assert!(!result.counts_match);
        assert!(!result.looks_consistent());
    }

    #[test]
    fn far_apart_positions_are_flagged_even_with_matching_counts() {
        let session = session_with(
            vec![TimePenalty {
                timecode_ms: 1000.0,
                penalty_ms: 35001.0,
            }],
            distance_channel(),
        );
        let replay = replay_with(vec![RecoverySpot {
            kind: None,
            position_m: Some(5000.0), // nowhere near the telemetry's 100.0
            penalty_secs: Some(35.0),
        }]);

        let result = cross_check_recoveries(&session, &replay);

        assert!(result.counts_match);
        assert_eq!(result.matches[0].positions_agree(), Some(false));
        assert!(!result.looks_consistent());
    }

    #[test]
    fn missing_distance_channel_is_uninformative_not_a_mismatch() {
        let session = Session {
            key_channel_map: KeyChannelMap::default(),
            channels: HashMap::new(),
            laps: vec![],
            metadata: HashMap::new(),
            file_name: "test.ld".to_string(),
            time_penalties: vec![TimePenalty {
                timecode_ms: 1000.0,
                penalty_ms: 35001.0,
            }],
        };
        let replay = replay_with(vec![RecoverySpot {
            kind: None,
            position_m: Some(100.0),
            penalty_secs: Some(35.0),
        }]);

        let result = cross_check_recoveries(&session, &replay);

        assert_eq!(result.matches[0].telemetry_position_m, None);
        assert_eq!(result.matches[0].positions_agree(), None);
        assert!(result.looks_consistent());
    }
}
