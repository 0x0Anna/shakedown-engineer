//! RBR/RSF install-root path discovery and configuration.
//!
//! Per `PROJECT_PLAN.md`'s "Install-path discovery and configuration" design
//! note: in a normal user environment the app is pointed at one RBR install
//! root (e.g. `C:\Richard Burns Rally\`), and every other location the app
//! needs is *inferred* from it, with each individually *overridable* (a
//! user's `Plugins\NGP\telemetry\` might live on a different drive, a
//! symlinked `SavedGames`, etc.). This module is that config model: a single
//! required root, a resolved-path struct derived from it, per-path
//! overrides, and validation that reports which expected paths are missing
//! rather than failing wholesale — an install with a missing `Maps\` folder
//! should still let the app read telemetry.
//!
//! UI-free, like the rest of `sde-formats` — consumed by `sde-app`.

use std::path::{Path, PathBuf};

/// Per-path overrides layered on top of an [`InstallConfig::root`]. Every
/// field defaults to `None` (derive from `root`); setting one only changes
/// that single resolved path, not the others.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathOverrides {
    pub ngp_telemetry_dir: Option<PathBuf>,
    pub ngp_telemetry_ini: Option<PathBuf>,
    pub rbr_ini: Option<PathBuf>,
    pub replays_dir: Option<PathBuf>,
    pub saved_games_dir: Option<PathBuf>,
    pub pacenote_dir: Option<PathBuf>,
    pub rsf_cars_dir: Option<PathBuf>,
    pub maps_dir: Option<PathBuf>,
    pub rsf_ini: Option<PathBuf>,
    pub rsf_personal_ini: Option<PathBuf>,
}

/// A single required install root, plus optional per-path overrides.
/// [`InstallConfig::resolve`] turns this into the actual [`InstallPaths`]
/// the rest of the app reads from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallConfig {
    pub root: PathBuf,
    pub overrides: PathOverrides,
}

impl InstallConfig {
    /// A config with no overrides: every path is derived from `root`
    /// following the standard RBR/RSF layout.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            overrides: PathOverrides::default(),
        }
    }

    /// Resolve every path this app cares about, applying any overrides on
    /// top of `root`-derived defaults. Doesn't touch the filesystem — see
    /// [`validate`] for existence checks.
    #[must_use]
    pub fn resolve(&self) -> InstallPaths {
        let r = &self.root;
        let o = &self.overrides;
        InstallPaths {
            root: r.clone(),
            ngp_telemetry_dir: o
                .ngp_telemetry_dir
                .clone()
                .unwrap_or_else(|| r.join("Plugins").join("NGP").join("telemetry")),
            ngp_telemetry_ini: o
                .ngp_telemetry_ini
                .clone()
                .unwrap_or_else(|| r.join("Plugins").join("NGP").join("Telemetry.ini")),
            rbr_ini: o
                .rbr_ini
                .clone()
                .unwrap_or_else(|| r.join("RichardBurnsRally.ini")),
            replays_dir: o.replays_dir.clone().unwrap_or_else(|| r.join("Replays")),
            saved_games_dir: o
                .saved_games_dir
                .clone()
                .unwrap_or_else(|| r.join("SavedGames")),
            pacenote_dir: o
                .pacenote_dir
                .clone()
                .unwrap_or_else(|| r.join("Plugins").join("Pacenote")),
            rsf_cars_dir: o
                .rsf_cars_dir
                .clone()
                .unwrap_or_else(|| r.join("rsfdata").join("cars")),
            maps_dir: o.maps_dir.clone().unwrap_or_else(|| r.join("Maps")),
            rsf_ini: o
                .rsf_ini
                .clone()
                .unwrap_or_else(|| r.join("RallySimFans.ini")),
            rsf_personal_ini: o
                .rsf_personal_ini
                .clone()
                .unwrap_or_else(|| r.join("rallysimfans_personal.ini")),
        }
    }
}

/// Every location the app needs, resolved from an [`InstallConfig`]. See
/// `PROJECT_PLAN.md`'s path table for the source of each mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPaths {
    pub root: PathBuf,
    /// `.ld`/`.tsv` NGP telemetry recordings.
    pub ngp_telemetry_dir: PathBuf,
    /// Active field-selection config (a sibling `Telemetry.sample.ini`
    /// reference copy is not modeled here — it's a static reference file,
    /// not something the app reads per-install).
    pub ngp_telemetry_ini: PathBuf,
    /// `RichardBurnsRally.ini` — carries the `[NGP]` recording toggle and
    /// tick-decimation settings; see [`read_ngp_settings`].
    pub rbr_ini: PathBuf,
    /// `.rpl` + `.ini` replay sidecar pairs.
    pub replays_dir: PathBuf,
    /// Car setups (`.lsp`) live under `<saved_games_dir>\<CarPhysicsFolder>\`
    /// — which physics folder depends on the car, so this is the parent
    /// directory, not a specific car's setup folder.
    pub saved_games_dir: PathBuf,
    pub pacenote_dir: PathBuf,
    /// RSF car/physics data.
    pub rsf_cars_dir: PathBuf,
    /// Stage/track data.
    pub maps_dir: PathBuf,
    pub rsf_ini: PathBuf,
    pub rsf_personal_ini: PathBuf,
}

/// One expected path that didn't exist, from a [`validate`] pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPath {
    /// Human-readable name of what this path was supposed to be, e.g.
    /// `"NGP telemetry directory"`.
    pub label: &'static str,
    pub path: PathBuf,
}

/// Result of checking an [`InstallPaths`] against the real filesystem.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    /// Every expected path that doesn't exist. An install can be usable
    /// with some of these missing (e.g. no `Maps\` folder yet) — this
    /// reports facts, it doesn't decide pass/fail on its own.
    pub missing: Vec<MissingPath>,
}

impl ValidationReport {
    /// Whether `root` itself looks like a real RBR install: both
    /// `RichardBurnsRally.ini` and `Plugins\NGP\Telemetry.ini` are present.
    /// These two are the cheapest, most load-bearing existence checks — every
    /// other path can be absent (an unconfigured plugin, an empty `Maps\`)
    /// without meaning `root` was pointed at the wrong place entirely.
    #[must_use]
    pub fn root_looks_valid(&self) -> bool {
        !self.missing.iter().any(|m| {
            m.label == "RichardBurnsRally.ini" || m.label == "NGP telemetry field selection"
        })
    }
}

/// Check every path in `paths` against the filesystem and report which are
/// missing. Never fails wholesale — a missing `Maps\` folder doesn't stop
/// the rest of the paths from being checked and reported on. See
/// [`ValidationReport::root_looks_valid`] for the narrower "is this even an
/// RBR install" question.
#[must_use]
pub fn validate(paths: &InstallPaths) -> ValidationReport {
    let checks: &[(&str, &Path)] = &[
        ("RichardBurnsRally.ini", &paths.rbr_ini),
        ("NGP telemetry field selection", &paths.ngp_telemetry_ini),
        ("NGP telemetry directory", &paths.ngp_telemetry_dir),
        ("Replays directory", &paths.replays_dir),
        ("SavedGames directory", &paths.saved_games_dir),
        ("Pacenote plugin directory", &paths.pacenote_dir),
        ("RSF car/physics data directory", &paths.rsf_cars_dir),
        ("Maps directory", &paths.maps_dir),
        ("RSF launcher config", &paths.rsf_ini),
    ];

    let missing = checks
        .iter()
        .filter(|(_, path)| !path.exists())
        .map(|(label, path)| MissingPath {
            label,
            path: path.to_path_buf(),
        })
        .collect();

    ValidationReport { missing }
}

/// The `[NGP]` settings in `RichardBurnsRally.ini` that decide whether a
/// telemetry recording exists at all, and how densely it's sampled — worth
/// surfacing before the app goes looking for files, per the design note's
/// "'telemetry recording is currently off' is a much better first-run
/// experience than an empty file list."
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NgpSettings {
    /// Whether the NGP plugin is currently set to record telemetry at all.
    /// `None` if the key is missing (older/differently-configured installs).
    pub telemetry_recording: Option<bool>,
    /// Sample decimation: telemetry is written every Nth physics tick.
    pub telemetry_tics: Option<u32>,
}

/// Read [`NgpSettings`] from `RichardBurnsRally.ini`'s `[NGP]` section at
/// `paths.rbr_ini`.
///
/// # Errors
///
/// Returns [`std::io::Error`] if the file can't be read. Missing or
/// malformed individual keys degrade to `None` fields rather than failing
/// the whole read — matching every other RBR/RSF `.ini` reader in this
/// crate.
pub fn read_ngp_settings(paths: &InstallPaths) -> std::io::Result<NgpSettings> {
    let bytes = std::fs::read(&paths.rbr_ini)?;
    let text = crate::ini::decode_text(&bytes);
    let ini = crate::ini::Ini::parse(&text);
    Ok(NgpSettings {
        telemetry_recording: ini
            .get_parsed::<u8>("NGP", "telemetryRecording")
            .map(|v| v != 0),
        telemetry_tics: ini.get_parsed("NGP", "telemetryTics"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_default_paths_from_root() {
        let paths = InstallConfig::new("C:\\Richard Burns Rally").resolve();

        assert_eq!(
            paths.ngp_telemetry_dir,
            PathBuf::from("C:\\Richard Burns Rally\\Plugins\\NGP\\telemetry")
        );
        assert_eq!(
            paths.ngp_telemetry_ini,
            PathBuf::from("C:\\Richard Burns Rally\\Plugins\\NGP\\Telemetry.ini")
        );
        assert_eq!(
            paths.rbr_ini,
            PathBuf::from("C:\\Richard Burns Rally\\RichardBurnsRally.ini")
        );
        assert_eq!(
            paths.replays_dir,
            PathBuf::from("C:\\Richard Burns Rally\\Replays")
        );
        assert_eq!(
            paths.saved_games_dir,
            PathBuf::from("C:\\Richard Burns Rally\\SavedGames")
        );
        assert_eq!(
            paths.pacenote_dir,
            PathBuf::from("C:\\Richard Burns Rally\\Plugins\\Pacenote")
        );
        assert_eq!(
            paths.rsf_cars_dir,
            PathBuf::from("C:\\Richard Burns Rally\\rsfdata\\cars")
        );
        assert_eq!(
            paths.maps_dir,
            PathBuf::from("C:\\Richard Burns Rally\\Maps")
        );
        assert_eq!(
            paths.rsf_ini,
            PathBuf::from("C:\\Richard Burns Rally\\RallySimFans.ini")
        );
        assert_eq!(
            paths.rsf_personal_ini,
            PathBuf::from("C:\\Richard Burns Rally\\rallysimfans_personal.ini")
        );
    }

    #[test]
    fn a_single_override_only_changes_that_one_path() {
        let mut config = InstallConfig::new("C:\\Richard Burns Rally");
        config.overrides.ngp_telemetry_dir = Some(PathBuf::from("D:\\ngp-telemetry"));
        let paths = config.resolve();

        assert_eq!(paths.ngp_telemetry_dir, PathBuf::from("D:\\ngp-telemetry"));
        // Everything else still derives from root.
        assert_eq!(
            paths.replays_dir,
            PathBuf::from("C:\\Richard Burns Rally\\Replays")
        );
    }

    #[test]
    fn validate_reports_every_missing_path_without_stopping_early() {
        let dir = std::env::temp_dir().join(format!("sde-rbr-install-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Only create the two "root looks valid" markers.
        std::fs::write(dir.join("RichardBurnsRally.ini"), "").unwrap();
        std::fs::create_dir_all(dir.join("Plugins").join("NGP")).unwrap();
        std::fs::write(dir.join("Plugins").join("NGP").join("Telemetry.ini"), "").unwrap();

        let paths = InstallConfig::new(&dir).resolve();
        let report = validate(&paths);

        assert!(report.root_looks_valid());
        // Everything else (telemetry dir, Replays, SavedGames, ...) is
        // still absent and should be reported, not silently skipped.
        assert!(report
            .missing
            .iter()
            .any(|m| m.label == "NGP telemetry directory"));
        assert!(report
            .missing
            .iter()
            .any(|m| m.label == "Replays directory"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_root_markers_mean_root_does_not_look_valid() {
        let dir =
            std::env::temp_dir().join(format!("sde-rbr-install-test-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let paths = InstallConfig::new(&dir).resolve();
        let report = validate(&paths);

        assert!(!report.root_looks_valid());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_ngp_settings_from_rbr_ini() {
        let dir =
            std::env::temp_dir().join(format!("sde-rbr-install-test-ngp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("RichardBurnsRally.ini"),
            "[NGP]\ntelemetryRecording=1\ntelemetryTics=5\n",
        )
        .unwrap();

        let paths = InstallConfig::new(&dir).resolve();
        let settings = read_ngp_settings(&paths).unwrap();

        assert_eq!(settings.telemetry_recording, Some(true));
        assert_eq!(settings.telemetry_tics, Some(5));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_ngp_keys_are_none_not_an_error() {
        let dir = std::env::temp_dir().join(format!(
            "sde-rbr-install-test-ngp-missing-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("RichardBurnsRally.ini"), "[NGP]\n").unwrap();

        let paths = InstallConfig::new(&dir).resolve();
        let settings = read_ngp_settings(&paths).unwrap();

        assert_eq!(settings.telemetry_recording, None);
        assert_eq!(settings.telemetry_tics, None);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_ngp_settings_errors_when_rbr_ini_is_missing() {
        let paths = InstallConfig::new("C:\\definitely-does-not-exist-sde-rbr").resolve();
        assert!(read_ngp_settings(&paths).is_err());
    }
}
