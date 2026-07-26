//! Parser for the replay metadata `.ini` sidecar that the NGPCarMenu
//! plugin writes next to every RSF `.rpl` replay.
//!
//! Despite being a few hundred bytes of plain INI, this is the richest
//! session-context source in an RSF capture: it names the stage, the car,
//! *and the setup file that was used*, plus the finish time and the full
//! surface/weather conditions. Telemetry alone can't say whether two runs
//! are comparable — this can.
//!
//! Field names and groupings below follow the file's own `[Replay]` keys
//! (see `PROJECT_PLAN.md`'s "RSF real-capture validation" section for a
//! full sample). Everything is optional: these files are written by
//! several NGPCarMenu/RSF versions and older ones omit fields.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::RbrError;
use crate::ini::{decode_text, Ini};

const REPLAY: &str = "Replay";
const RUNKI_SPOTS: &str = "RunkiSpots";

/// Which stage was driven.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageInfo {
    /// Display name, e.g. `"Gabiria-Legazpi 2004"`.
    pub name: Option<String>,
    /// RSF's numeric stage id, e.g. `450`. The stable key for matching a
    /// run against a stage; `name` varies with the installed track pack.
    pub map_id: Option<u32>,
    /// Stage length in metres.
    pub length_m: Option<u32>,
    /// Install-relative track folder, e.g. `"Maps\\450-Gabiria-Legazpi"`.
    pub track_folder: Option<String>,
    /// RSF's combined surface/weather preset id, e.g.
    /// `"450M_hazy_lightcloud"`.
    pub track_setting: Option<String>,
}

/// Which car, and which setup it ran.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CarInfo {
    /// Display name, e.g. `"Mini JCW WRC"`.
    pub model: Option<String>,
    /// In-game car slot, which is also what the `.ld`'s `Vehicle`
    /// metadata field contains for RSF exports.
    pub slot: Option<u32>,
    /// Path to the `.lsp` setup used, e.g.
    /// `"savedgames\\Mini_JCW_WRC_ngp6\\Tarmac Bumpy.lsp"`. The link
    /// between a run and its setup sheet.
    pub setup_name: Option<String>,
    pub model_folder: Option<String>,
    /// Physics folder, which also identifies the NGP physics revision the
    /// car ran (e.g. `"rsfdata\\cars\\Mini_JCW_WRC_ngp6"`).
    pub physics_folder: Option<String>,
    pub rsf_car_id: Option<u32>,
}

/// How the run turned out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultInfo {
    /// Final *scored* stage time, i.e. including any recovery penalties.
    /// Matches the last value of the telemetry's `raceTime` channel — see
    /// `sde_motec::apply_ngp_timebase`, which strips those penalties back
    /// out to recover physical driving time.
    pub finish_time_secs: Option<f64>,
    pub avg_speed_kph: Option<f64>,
    /// e.g. `"Hotlap"`.
    pub rally_type: Option<String>,
    pub rally_name: Option<String>,
}

/// Surface, weather and damage settings. These decide whether two runs are
/// meaningfully comparable, so they matter as much as the times do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conditions {
    /// e.g. `"Realistic"`.
    pub damage_type: Option<String>,
    /// e.g. `"Tarmac Dry"`.
    pub tyre_type: Option<String>,
    pub weather_type: Option<String>,
    pub time_of_day: Option<String>,
    pub sky_cloud_type: Option<String>,
    pub sky_type: Option<String>,
    /// e.g. `"Damp"`.
    pub surface_wetness: Option<String>,
    /// e.g. `"New"`.
    pub surface_age: Option<String>,
}

/// Which builds produced the replay — worth recording because NGP physics
/// revisions change car behaviour, so telemetry isn't comparable across
/// them even for the same car and setup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Versions {
    /// Replay flavour, e.g. `"RSF"`.
    pub replay_type: Option<String>,
    /// NGP physics version, e.g. `"7.5.779.508"`.
    pub ngp: Option<String>,
    /// RSF launcher version, e.g. `"0.60.5074.0"`.
    pub rsf: Option<String>,
}

/// A "recover vehicle" event: the driver went off, the car was placed back
/// on the road book, and a fixed time penalty was added.
///
/// These correspond 1:1 with the `raceTime` discontinuities that
/// `sde_motec::apply_ngp_timebase` detects and reports as
/// `sde_motec::TimePenalty`, and cross-checking the two is the cheapest way
/// to confirm a telemetry file and a replay describe the same run: the
/// counts should match, and each `position_m` should line up with the stage
/// distance at the corresponding penalty.
#[derive(Debug, Clone, PartialEq)]
pub struct RecoverySpot {
    /// RSF's spot-type code, e.g. `"C4H"`. Not decoded — kept verbatim.
    pub kind: Option<String>,
    /// Distance along the stage, in metres, where the recovery happened.
    pub position_m: Option<f64>,
    /// Time penalty applied, in seconds (35.0 as of NGP 7.5).
    pub penalty_secs: Option<f64>,
}

/// A parsed replay metadata `.ini` sidecar.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReplayInfo {
    pub stage: StageInfo,
    pub car: CarInfo,
    pub result: ResultInfo,
    pub conditions: Conditions,
    pub versions: Versions,
    /// Vehicle recoveries, in file order. Empty both when the run was
    /// clean and when the writing plugin predates the feature — see
    /// [`ReplayInfo::recovery_section_present`] to tell those apart.
    pub recovery_spots: Vec<RecoverySpot>,
    /// Whether the file carried a `[RunkiSpots]` section at all.
    ///
    /// Distinguishes "clean run, no recoveries" from "this file can't tell
    /// us". Only meaningful alongside knowledge of the writing plugin
    /// version, so treat it as a hint, not proof. (Observed: Run1, which
    /// took two recoveries, has the section; Run2, which was clean, omits
    /// it entirely rather than writing `Count = 0`.)
    pub recovery_section_present: bool,
    /// `[Replay]` keys this parser doesn't model, so a newer RSF build's
    /// additions are still reachable without a code change. Keys are
    /// lowercased.
    pub extra: BTreeMap<String, String>,
}

/// Every `[Replay]` key mapped onto a typed field above. Used to decide
/// what lands in [`ReplayInfo::extra`].
const KNOWN_REPLAY_KEYS: &[&str] = &[
    "type",
    "name",
    "trackfolder",
    "mapid",
    "maplength",
    "rallytype",
    "rallyname",
    "carmodel",
    "carslot",
    "setupname",
    "finishtimesecs",
    "avgspeedkph",
    "carmodelfolder",
    "carphysicsfolder",
    "damagetype",
    "tyretype",
    "weathertype",
    "timeofday",
    "skycloudtype",
    "skytype",
    "surfacewetness",
    "surfaceage",
    "tracksetting",
    "ngp",
    "rsf",
    "rsfcarid",
];

impl ReplayInfo {
    /// Total time penalty from recoveries, in seconds.
    #[must_use]
    pub fn total_penalty_secs(&self) -> f64 {
        self.recovery_spots
            .iter()
            .filter_map(|s| s.penalty_secs)
            .sum()
    }

    /// Physical driving time: the scored finish time with recovery
    /// penalties removed. `None` if the file has no finish time.
    ///
    /// This is the figure that should match the span of the corrected
    /// telemetry timebase (`sde_motec::apply_ngp_timebase`), and the one to
    /// compare between runs — scored times aren't comparable when one run
    /// took a penalty and the other didn't.
    #[must_use]
    pub fn driving_time_secs(&self) -> Option<f64> {
        Some(self.result.finish_time_secs? - self.total_penalty_secs())
    }
}

/// Parse a replay metadata `.ini` from disk.
///
/// # Errors
///
/// Returns [`RbrError::Io`] if the file can't be read. Parsing itself
/// doesn't fail: unknown or malformed fields become `None` rather than
/// costing the caller the rest of the file.
pub fn parse_replay_ini(path: &Path) -> Result<ReplayInfo, RbrError> {
    let bytes = std::fs::read(path).map_err(|source| RbrError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(parse_replay_ini_str(&decode_text(&bytes)))
}

/// Parse replay metadata `.ini` text already in memory.
#[must_use]
pub fn parse_replay_ini_str(text: &str) -> ReplayInfo {
    let ini = Ini::parse(text);

    let s = |key: &str| ini.get_nonempty(REPLAY, key).map(ToString::to_string);

    let extra = ini
        .section(REPLAY)
        .map(|sec| {
            sec.iter()
                .filter(|(k, _)| !KNOWN_REPLAY_KEYS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();

    ReplayInfo {
        stage: StageInfo {
            name: s("Name"),
            map_id: ini.get_parsed(REPLAY, "MapID"),
            length_m: ini.get_parsed(REPLAY, "MapLength"),
            track_folder: s("TrackFolder"),
            track_setting: s("TrackSetting"),
        },
        car: CarInfo {
            model: s("CarModel"),
            slot: ini.get_parsed(REPLAY, "CarSlot"),
            setup_name: s("SetupName"),
            model_folder: s("CarModelFolder"),
            physics_folder: s("CarPhysicsFolder"),
            rsf_car_id: ini.get_parsed(REPLAY, "RSFCarID"),
        },
        result: ResultInfo {
            finish_time_secs: ini.get_parsed(REPLAY, "FinishTimeSecs"),
            avg_speed_kph: ini.get_parsed(REPLAY, "AvgSpeedKPH"),
            rally_type: s("RallyType"),
            rally_name: s("RallyName"),
        },
        conditions: Conditions {
            damage_type: s("DamageType"),
            tyre_type: s("TyreType"),
            weather_type: s("WeatherType"),
            time_of_day: s("TimeOfDay"),
            sky_cloud_type: s("SkyCloudType"),
            sky_type: s("SkyType"),
            surface_wetness: s("SurfaceWetness"),
            surface_age: s("SurfaceAge"),
        },
        versions: Versions {
            replay_type: s("Type"),
            ngp: s("NGP"),
            rsf: s("RSF"),
        },
        recovery_spots: parse_recovery_spots(&ini),
        recovery_section_present: ini.has_section(RUNKI_SPOTS),
        extra,
    }
}

/// Read the `[RunkiSpots]` section: a `Count`, then `R1Typ`/`R1Pos`/`R1Tim`
/// triples numbered from 1.
///
/// `Count` is trusted only as far as it agrees with the keys actually
/// present — a spot is emitted only if at least one of its three keys
/// exists, so an inflated `Count` yields fewer spots rather than a run of
/// empty ones.
fn parse_recovery_spots(ini: &Ini) -> Vec<RecoverySpot> {
    let count: usize = ini.get_parsed(RUNKI_SPOTS, "Count").unwrap_or(0);

    (1..=count)
        .map(|i| {
            (
                ini.get_nonempty(RUNKI_SPOTS, &format!("R{i}Typ")),
                ini.get_parsed::<f64>(RUNKI_SPOTS, &format!("R{i}Pos")),
                ini.get_parsed::<f64>(RUNKI_SPOTS, &format!("R{i}Tim")),
            )
        })
        .filter(|(kind, pos, tim)| kind.is_some() || pos.is_some() || tim.is_some())
        .map(|(kind, position_m, penalty_secs)| RecoverySpot {
            kind: kind.map(ToString::to_string),
            position_m,
            penalty_secs,
        })
        .collect()
}
