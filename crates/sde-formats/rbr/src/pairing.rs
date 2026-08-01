//! Best-effort pairing of a loaded telemetry file with the replay `.ini`
//! sidecar that describes the same run.
//!
//! RSF/NGP has no shared filename or folder convention between the two:
//! NGP telemetry (`.ld`/`.tsv`) lives under `Plugins\NGP\telemetry\`, named
//! `telemetry-NGP-Car_<slot>-<car>-Stage_<id>-<export timestamp>`, while a
//! replay's `.ini` lives under `Replays\`, named
//! `<driver>_rsf_<type>_<stage>_<time>_<carslot>` — no shared stem to match
//! on (see `PROJECT_PLAN.md`'s "Install-path discovery" design note).
//!
//! What *is* shared is timing: the `.ld` is produced by exporting/
//! converting shortly after a run finishes and its replay files are
//! written. Confirmed against two real captures (`PROJECT_PLAN.md`'s "RSF
//! real-capture validation" section): each run's `.ld` modification time
//! trails its `.ini`'s by well under a minute (6s and 41s in the two
//! samples), while the two *different* runs in that same capture are ~7
//! minutes apart. Nearest-modification-time-within-a-generous-window is
//! therefore a workable heuristic, even though it's not a guarantee — see
//! [`find_matching_replay_ini`].

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How far apart two files' modification times can be and still be
/// considered a match. Comfortably above the largest observed real gap
/// (41s) while comfortably below the gap between different runs in the
/// same capture (~7 minutes) — see the module doc for the source capture.
const MAX_MATCH_GAP: Duration = Duration::from_secs(180);

/// The absolute difference between two [`SystemTime`]s, regardless of
/// which one is later. `SystemTime::duration_since` only succeeds in one
/// direction (self >= earlier), so this tries both.
fn abs_diff(a: SystemTime, b: SystemTime) -> Duration {
    a.duration_since(b)
        .unwrap_or_else(|_| b.duration_since(a).unwrap_or(Duration::ZERO))
}

/// Find the `.ini` in `replays_dir` whose modification time is closest to
/// `telemetry_path`'s, provided the gap is within [`MAX_MATCH_GAP`].
/// Returns the matched path and the gap between the two files'
/// modification times (so a caller can show how confident the match is),
/// or `None` if `telemetry_path` or `replays_dir` can't be read, or no
/// `.ini` in `replays_dir` is within tolerance.
///
/// This is a heuristic, not a guarantee: two runs recorded back-to-back
/// within `MAX_MATCH_GAP` of each other could still be mismatched. Callers
/// that can cross-check the result against something else (e.g.
/// `sde_app::replay_check::cross_check_recoveries` against the loaded
/// session's own detected time penalties) should still do so.
#[must_use]
pub fn find_matching_replay_ini(
    telemetry_path: &Path,
    replays_dir: &Path,
) -> Option<(PathBuf, Duration)> {
    let telemetry_mtime = std::fs::metadata(telemetry_path).ok()?.modified().ok()?;

    std::fs::read_dir(replays_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ini"))
        })
        .filter_map(|p| {
            let mtime = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((p, abs_diff(mtime, telemetry_mtime)))
        })
        .filter(|(_, gap)| *gap <= MAX_MATCH_GAP)
        .min_by_key(|(_, gap)| *gap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sde-rbr-pairing-test-{label}-{}",
            std::process::id()
        ))
    }

    /// Set a file's modification time to `base + offset`.
    fn touch_at(path: &Path, base: SystemTime, offset: Duration) {
        let file = std::fs::File::create(path).unwrap();
        file.set_modified(base + offset).unwrap();
    }

    #[test]
    fn picks_the_closest_ini_within_tolerance() {
        let dir = temp_dir("closest");
        let replays = dir.join("Replays");
        std::fs::create_dir_all(&replays).unwrap();

        let base = SystemTime::now();
        let ld_path = dir.join("telemetry.ld");
        touch_at(&ld_path, base, Duration::from_secs(1000));

        // Run1's replay: 6s before the .ld, matching the real capture.
        let close = replays.join("run1.ini");
        touch_at(&close, base, Duration::from_secs(994));
        // Run2's replay: ~7 minutes before the .ld, matching the real
        // capture's inter-run gap — should not be picked.
        let far = replays.join("run2.ini");
        touch_at(&far, base, Duration::from_secs(580));

        let (matched, gap) = find_matching_replay_ini(&ld_path, &replays).expect("a match");
        assert_eq!(matched, close);
        assert_eq!(gap, Duration::from_secs(6));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_ini_within_tolerance_is_none() {
        let dir = temp_dir("out-of-range");
        let replays = dir.join("Replays");
        std::fs::create_dir_all(&replays).unwrap();

        let base = SystemTime::now();
        let ld_path = dir.join("telemetry.ld");
        touch_at(&ld_path, base, Duration::from_secs(1000));

        let far = replays.join("unrelated.ini");
        touch_at(&far, base, Duration::from_secs(0));

        assert_eq!(find_matching_replay_ini(&ld_path, &replays), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn non_ini_files_are_ignored() {
        let dir = temp_dir("non-ini");
        let replays = dir.join("Replays");
        std::fs::create_dir_all(&replays).unwrap();

        let base = SystemTime::now();
        let ld_path = dir.join("telemetry.ld");
        touch_at(&ld_path, base, Duration::from_secs(1000));

        // A closer-in-time .rpl should be ignored in favor of the (still
        // in-tolerance but further) .ini sidecar.
        let rpl = replays.join("run1.rpl");
        touch_at(&rpl, base, Duration::from_secs(999));
        let ini = replays.join("run1.ini");
        touch_at(&ini, base, Duration::from_secs(990));

        let (matched, _) = find_matching_replay_ini(&ld_path, &replays).expect("a match");
        assert_eq!(matched, ini);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_replays_dir_is_none_not_a_panic() {
        let dir = temp_dir("missing-dir");
        std::fs::create_dir_all(&dir).unwrap();
        let ld_path = dir.join("telemetry.ld");
        std::fs::write(&ld_path, b"").unwrap();

        assert_eq!(
            find_matching_replay_ini(&ld_path, &dir.join("NoSuchReplaysDir")),
            None
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_telemetry_file_is_none_not_a_panic() {
        let dir = temp_dir("missing-telemetry");
        let replays = dir.join("Replays");
        std::fs::create_dir_all(&replays).unwrap();
        std::fs::write(replays.join("run1.ini"), b"").unwrap();

        assert_eq!(
            find_matching_replay_ini(&dir.join("does-not-exist.ld"), &replays),
            None
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
