//! Adapter: Assetto Corsa Rally's saved car setups (`CarSetupsDataSaveSlot.sav`)
//! -> [`Setup`].
//!
//! `sde-acr` parses the save into dotted keys with text values
//! (`Brakes.FrontLeft.PadCompound` = `(Pads:Type00_Soft)`). This maps that
//! onto the sim-agnostic model: the first path segment becomes the group
//! (`Brakes`, `Differentials`, `Suspensions`, ...), the rest the entry, and
//! each value becomes a number or a text, so [`crate::diff`] can report
//! "front bias 0.50 -> 0.54" with a delta rather than as an opaque string.
//!
//! One file holds many setups (per car, per stage), so loading returns a
//! list; pick one by name and car.

use std::path::Path;

use sde_acr::savegame::{parse_car_setups, SavedSetup};
use sde_acr::AcrError;

use crate::model::{Setup, SetupEntry, SetupGroup, SetupValue};

/// What [`Setup::source`] is set to for these sheets.
pub const SOURCE: &str = "AC Rally saved setup";

/// Units, matched as a case-insensitive *suffix* of the dotted key.
///
/// Deliberately short. Only units confirmed against other data are listed:
/// tyre pressure is psi (the recorded `wheel_pressure_*` channels sit at
/// 28.9..29.6 psi against a 31 psi setting), and master cylinder bores are
/// mm (19.05 and 20.64 are exactly 3/4" and 13/16"). Spring rates, anti-roll
/// bars, camber and toe are left unlabelled: their magnitudes fit several
/// unit systems and a wrong unit on a setup sheet is worse than none.
const UNITS: &[(&str, &str)] = &[
    ("TyrePressure", "psi"),
    ("MasterCylinderFront", "mm"),
    ("MasterCylinderRear", "mm"),
];

/// Load every saved setup in a `CarSetupsDataSaveSlot.sav`, in file order.
///
/// # Errors
///
/// Any [`AcrError`] from parsing the save.
pub fn load_sav(path: &Path) -> Result<Vec<Setup>, AcrError> {
    let save = parse_car_setups(path)?;
    Ok(save.setups().map(from_saved).collect())
}

/// Adapt one parsed setup onto the sim-agnostic model.
#[must_use]
pub fn from_saved(saved: &SavedSetup) -> Setup {
    let mut groups: Vec<SetupGroup> = Vec::new();
    for (key, raw) in &saved.entries {
        let (group_key, entry_key) = key.split_once('.').unwrap_or(("Other", key.as_str()));
        let entry = SetupEntry {
            key: entry_key.to_string(),
            label: prettify(entry_key),
            value: value_of(raw),
            unit: unit_for(key).map(ToString::to_string),
        };
        if let Some(group) = groups
            .iter_mut()
            .find(|g| g.key.eq_ignore_ascii_case(group_key))
        {
            group.entries.push(entry);
        } else {
            groups.push(SetupGroup {
                key: group_key.to_string(),
                name: prettify(group_key),
                entries: vec![entry],
            });
        }
    }

    Setup {
        name: saved.name.clone(),
        source: SOURCE.to_string(),
        car: Some(saved.car_id.clone()),
        groups,
    }
}

/// Number if the value is one, or a `(Category:Number)` wrapper around one
/// (`(MasterCylinders:19.05)`); otherwise text, with the `(Category:...)`
/// wrapper removed so `(Pads:Type00_Soft)` shows as `Type00_Soft`.
fn value_of(raw: &str) -> SetupValue {
    if let Ok(n) = raw.trim().parse::<f64>() {
        return SetupValue::Number(n);
    }
    let inner = raw
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| s.split_once(':'))
        .map(|(_, v)| v);
    match inner {
        Some(v) => v
            .parse::<f64>()
            .map_or_else(|_| SetupValue::Text(v.to_string()), SetupValue::Number),
        None => SetupValue::Text(raw.to_string()),
    }
}

fn unit_for(key: &str) -> Option<&'static str> {
    let lower = key.to_ascii_lowercase();
    UNITS
        .iter()
        .find(|(suffix, _)| lower.ends_with(&suffix.to_ascii_lowercase()))
        .map(|(_, unit)| *unit)
}

/// `FrontLeft.PadCompound` -> `Front Left Pad Compound`; runs of capitals
/// stay together (`ARBStiffness` -> `ARB Stiffness`, `LSDRamps` -> `LSD Ramps`).
fn prettify(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let mut out = String::with_capacity(key.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c == '.' {
            out.push(' ');
            continue;
        }
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_ascii_digit() || (prev.is_uppercase() && next_lower) {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(entries: &[(&str, &str)]) -> SavedSetup {
        SavedSetup {
            name: "Gravel".to_string(),
            author: String::new(),
            saved_ticks: 0,
            game_version: "0.6".to_string(),
            car_id: "SkodaFabiaRSRally2".to_string(),
            stage: "S".to_string(),
            surface: None,
            entries: entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }

    #[test]
    fn groups_by_first_segment_in_file_order() {
        let setup = from_saved(&saved(&[
            ("Brakes.BrakesMain.FrontBias", "0.500000"),
            ("Differentials.Front.LSDPreload", "151.000000"),
            ("Brakes.FrontLeft.PadCompound", "(Pads:Type00_Soft)"),
        ]));
        let keys: Vec<_> = setup.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["Brakes", "Differentials"]);
        assert_eq!(setup.group("Brakes").unwrap().entries.len(), 2);
        assert_eq!(setup.car.as_deref(), Some("SkodaFabiaRSRally2"));
        assert_eq!(setup.source, SOURCE);
    }

    #[test]
    fn values_become_numbers_or_unwrapped_text() {
        assert_eq!(value_of("0.500000"), SetupValue::Number(0.5));
        assert_eq!(
            value_of("(MasterCylinders:19.05)"),
            SetupValue::Number(19.05)
        );
        assert_eq!(
            value_of("(Pads:Type00_Soft)"),
            SetupValue::Text("Type00_Soft".to_string())
        );
        assert_eq!(
            value_of("(LSDRampAngles:64_70)"),
            SetupValue::Text("64_70".to_string())
        );
        assert_eq!(value_of("plain"), SetupValue::Text("plain".to_string()));
    }

    #[test]
    fn only_confirmed_units_are_attached() {
        assert_eq!(unit_for("Wheels.FrontLeft.TyrePressure"), Some("psi"));
        assert_eq!(unit_for("Brakes.BrakesMain.MasterCylinderRear"), Some("mm"));
        assert_eq!(unit_for("Suspensions.FrontLeft.SpringStiffness"), None);
        assert_eq!(unit_for("Wheels.FrontLeft.Camber"), None);
    }

    #[test]
    fn labels_split_camel_case_and_keep_acronyms() {
        assert_eq!(prettify("FrontLeft.PadCompound"), "Front Left Pad Compound");
        assert_eq!(prettify("Axles"), "Axles");
        assert_eq!(prettify("Front.LSDRamps"), "Front LSD Ramps");
        assert_eq!(prettify("Axles"), "Axles");
        assert_eq!(prettify("ARBStiffness"), "ARB Stiffness");
    }

    #[test]
    fn diff_reports_a_numeric_delta_for_front_bias() {
        let a = from_saved(&saved(&[("Brakes.BrakesMain.FrontBias", "0.500000")]));
        let b = from_saved(&saved(&[("Brakes.BrakesMain.FrontBias", "0.540000")]));
        let d = crate::diff(&a, &b);
        assert_eq!(d.change_count(), 1);
    }
}
