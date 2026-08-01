//! Adapter: Richard Burns Rally / RSF `.lsp` setup sheets -> [`Setup`].
//!
//! The adapter lives here rather than in `sde-rbr` so the format crates
//! stay leaf dependencies — the same direction `sde-core` sits in
//! relative to the telemetry parsers (see PROJECT_PLAN.md's modularity
//! principles). `sde-rbr` does the parsing; this maps its output onto the
//! sim-agnostic model, prettifies RBR's `CamelCase` keys for display, and
//! attaches units.

use std::path::Path;

use sde_rbr::setup::{parse_lsp, LspSetup};
use sde_rbr::RbrError;

use crate::model::{Setup, SetupEntry, SetupGroup, SetupValue};

/// What [`Setup::source`] is set to for these sheets.
pub const SOURCE: &str = "RBR/RSF .lsp";

/// Units for RBR setup keys, matched as a case-insensitive *suffix* of the
/// key so one entry covers every corner and axle variant
/// (`SpringStiffness`, `FrontRollBarStiffness`, `BumpStopStiffnessRear_NGP`
/// once its `_NGP` suffix is trimmed). Checked in order, so more specific
/// entries come first.
///
/// Deliberately incomplete. Units here are the ones the real captures
/// confirm — stiffnesses in N/m (26000), pressures in Pa (195000 tyre,
/// 4000000 brake = 4 MPa, matching PROJECT_PLAN.md's capture notes),
/// lengths in m (0.23). The angles (`MaxSteeringLock`, `WheelAxisInclination`)
/// and the various ratio-like NGP values are left unlabelled: their
/// magnitudes are *consistent* with radians and 0..1 ratios, but nothing in
/// the captures or NGP's own docs confirms it, and a wrong unit on a setup
/// sheet is worse than no unit.
const UNITS: &[(&str, &str)] = &[
    ("MaxTorque", "Nm"),
    ("Stiffness", "N/m"),
    ("Pressure", "Pa"),
    ("Damping", "Ns/m"),
    ("DampingBumpHighSpeed", "Ns/m"),
    ("Length", "m"),
    ("Height", "m"),
    ("Position", "m"),
];

/// Keys whose numeric value is an *identifier*, not a quantity — RBR's
/// `GearId0..9`, `FinalDriveId`, `DropGearId` (indices into the car's
/// ratio tables) and `TopMountSlot`. Matched as a suffix of the key with
/// any trailing digits trimmed, the same way [`UNITS`] is.
///
/// These are adapted as [`SetupValue::Text`] rather than
/// [`SetupValue::Number`] so a diff reports them as a plain change
/// (`12 → 11`) instead of computing a delta and percentage against them:
/// "gear id 12 became 11" is a *different gear*, not an 8.3% reduction in
/// anything. Whether the change is large or small is a question about the
/// ratio table these index into, which this crate doesn't have.
const IDENTIFIER_SUFFIXES: &[&str] = &["Id", "Slot"];

/// Load and adapt a `.lsp` setup file.
///
/// [`Setup::name`] is the file stem (`"Tarmac Bumpy"`), which is what RSF
/// shows and what the replay `.ini`'s `SetupName` points at.
///
/// # Errors
///
/// Returns [`RbrError::Io`] if the file can't be read. Parsing itself is
/// infallible — an unreadable section yields fewer entries, not an error.
pub fn load_lsp(path: &Path) -> Result<Setup, RbrError> {
    let parsed = parse_lsp(path)?;
    let name = path
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    Ok(from_lsp(&parsed, &name))
}

/// Adapt an already-parsed `.lsp` onto the sim-agnostic model.
#[must_use]
pub fn from_lsp(parsed: &LspSetup, name: &str) -> Setup {
    let groups = parsed
        .sections
        .iter()
        .map(|section| SetupGroup {
            key: section.name.clone(),
            name: prettify(&section.name),
            entries: section
                .entries
                .iter()
                .map(|entry| SetupEntry {
                    key: entry.key.clone(),
                    label: prettify(&entry.key),
                    value: match entry.values.as_slice() {
                        [v] if is_identifier(&entry.key) => {
                            SetupValue::Text(SetupValue::Number(*v).display())
                        }
                        [v] => SetupValue::Number(*v),
                        vs => SetupValue::Vector(vs.to_vec()),
                    },
                    unit: unit_for(&entry.key).map(ToString::to_string),
                })
                .collect(),
        })
        .collect();

    Setup {
        name: name.to_string(),
        source: SOURCE.to_string(),
        // `.lsp` files carry no car identity — the car is implied by the
        // `SavedGames\<CarPhysicsFolder>\` folder they live in. Callers
        // with that context (the replay `.ini`) fill this in.
        car: None,
        groups,
    }
}

/// Whether a key names an identifier rather than a quantity — see
/// [`IDENTIFIER_SUFFIXES`].
#[must_use]
pub fn is_identifier(key: &str) -> bool {
    let base = key_base(key);
    IDENTIFIER_SUFFIXES
        .iter()
        .any(|suffix| base.len() >= suffix.len() && base.ends_with(suffix))
}

/// The unit for a setup key, or `None` when it isn't confidently known.
#[must_use]
pub fn unit_for(key: &str) -> Option<&'static str> {
    if is_identifier(key) {
        return None;
    }
    let base = key_base(key);
    UNITS
        .iter()
        .find(|(suffix, _)| contains_ignore_case(base, suffix))
        .map(|(_, unit)| *unit)
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// Trim RBR's `_NGP` marker, which distinguishes NGP physics additions
/// from stock RBR fields but says nothing about the quantity.
fn strip_ngp(key: &str) -> &str {
    key.strip_suffix("_NGP").unwrap_or(key)
}

/// A key reduced to what the suffix tables match against: without its
/// `_NGP` marker, and without the trailing index of a numbered series
/// (`GearId0`, `CenterDiffThrottle_00`).
fn key_base(key: &str) -> &str {
    strip_ngp(key)
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches('_')
}

/// Turn an RBR key into something readable: `"FrontRollBarStiffness"` ->
/// `"Front Roll Bar Stiffness"`, `"BumpStopStiffnessRear_NGP"` ->
/// `"Bump Stop Stiffness Rear (NGP)"`, `"vecTopMountPosition"` ->
/// `"Top Mount Position"`.
///
/// Runs of capitals are kept together so corner and axle markers survive
/// (`WheelLF` -> `"Wheel LF"`, not `"Wheel L F"`), and digits stay
/// attached to the word they trail (`GearId0` -> `"Gear Id0"`,
/// `CenterDiffThrottle_00` -> `"Center Diff Throttle 00"`) so the numbered
/// series stay visually aligned in a list.
#[must_use]
pub fn prettify(key: &str) -> String {
    let is_ngp = key.ends_with("_NGP");
    // `vec` prefixes a multi-component value; the model already shows the
    // components, so the prefix is noise.
    let base = strip_ngp(key);
    let base = base.strip_prefix("vec").unwrap_or(base);

    let chars: Vec<char> = base.chars().collect();
    let mut out = String::with_capacity(base.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            out.push(' ');
            continue;
        }
        let prev = i.checked_sub(1).map(|p| chars[p]);
        let next = chars.get(i + 1).copied();
        let starts_word = c.is_ascii_uppercase()
            && match (prev, next) {
                // Nothing before it — it's already the start.
                (None, _) => false,
                // `...aB` — a capital after a lowercase always starts a word.
                (Some(p), _) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                // `...ABc` — the last capital of a run joins the word after
                // it; anything else is mid-run (`...LF`), so it doesn't.
                (Some(_), next) => next.is_some_and(|n| n.is_ascii_lowercase()),
            };
        if starts_word && !out.ends_with(' ') && !out.is_empty() {
            out.push(' ');
        }
        out.push(c);
    }

    let mut out = out.trim().to_string();
    if !out.is_empty() {
        // Section/key names are already capitalized except for the `vec`
        // and lowercase-first NGP keys.
        let mut cs = out.chars();
        if let Some(first) = cs.next() {
            out = first.to_ascii_uppercase().to_string() + cs.as_str();
        }
    }
    if is_ngp {
        out.push_str(" (NGP)");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sde_rbr::setup::parse_lsp_str;

    const SAMPLE: &str = r#"(("CarSetup"
 Car             ("Car"
 MaxSteeringLock 0.621000
                  FrontRollBarStiffness 16000.000000
                                   MaxSteeringLock
                  FrontRollBarStiffness
                  )
 SpringDamperLF  (":-D"
 SpringLength    0.230000
                  SpringStiffness 26000.000000
                  DampingBump     3500.000000
                  BumpStopStiffnessFront_NGP 175000.000000
                                   SpringLength
                  SpringStiffness
                  DampingBump
                  BumpStopStiffnessFront_NGP
                  )
 WheelLF         (":-D"
 vecTopMountPosition +0.592000 -2.557000 +0.740000
                                   vecTopMountPosition
                  )
 TyreLF          (":-D"
 Pressure        195000.000000
                                   Pressure
                  )
 ))"#;

    fn sample_setup() -> Setup {
        from_lsp(&parse_lsp_str(SAMPLE), "Tarmac Bumpy")
    }

    #[test]
    fn maps_sections_and_entries_onto_the_model() {
        let setup = sample_setup();
        assert_eq!(setup.name, "Tarmac Bumpy");
        assert_eq!(setup.source, SOURCE);
        assert_eq!(setup.car, None);
        assert_eq!(
            setup
                .groups
                .iter()
                .map(|g| g.name.as_str())
                .collect::<Vec<_>>(),
            ["Car", "Spring Damper LF", "Wheel LF", "Tyre LF"]
        );
        assert_eq!(setup.entry_count(), 8);
    }

    #[test]
    fn single_values_are_numbers_and_multi_values_are_vectors() {
        let setup = sample_setup();
        assert_eq!(
            setup
                .group("Car")
                .unwrap()
                .entry("MaxSteeringLock")
                .unwrap()
                .value,
            SetupValue::Number(0.621)
        );
        assert_eq!(
            setup
                .group("WheelLF")
                .unwrap()
                .entry("vecTopMountPosition")
                .unwrap()
                .value,
            SetupValue::Vector(vec![0.592, -2.557, 0.74])
        );
    }

    #[test]
    fn units_are_attached_where_known_and_omitted_where_not() {
        let setup = sample_setup();
        let group = setup.group("SpringDamperLF").unwrap();
        assert_eq!(
            group.entry("SpringStiffness").unwrap().display(),
            "26000 N/m"
        );
        assert_eq!(group.entry("SpringLength").unwrap().display(), "0.23 m");
        assert_eq!(group.entry("DampingBump").unwrap().display(), "3500 Ns/m");
        assert_eq!(
            setup
                .group("TyreLF")
                .unwrap()
                .entry("Pressure")
                .unwrap()
                .display(),
            "195000 Pa"
        );
        // Angles are deliberately unlabelled — see UNITS.
        assert_eq!(
            setup
                .group("Car")
                .unwrap()
                .entry("MaxSteeringLock")
                .unwrap()
                .unit,
            None
        );
    }

    #[test]
    fn identifier_keys_carry_no_arithmetic() {
        assert!(is_identifier("GearId0"));
        assert!(is_identifier("FinalDriveId"));
        assert!(is_identifier("DropGearId"));
        assert!(is_identifier("TopMountSlot"));
        assert!(!is_identifier("SpringStiffness"));
        assert!(!is_identifier("Pressure"));

        let setup = from_lsp(
            &parse_lsp_str("((\"CarSetup\"\n Drive (\":-D\"\n DropGearId 12\n DropGearId\n )\n ))"),
            "a",
        );
        let other = from_lsp(
            &parse_lsp_str("((\"CarSetup\"\n Drive (\":-D\"\n DropGearId 11\n DropGearId\n )\n ))"),
            "b",
        );
        // Text, so the change is reported without a meaningless "-8.3%".
        assert_eq!(
            setup
                .group("Drive")
                .unwrap()
                .entry("DropGearId")
                .unwrap()
                .value,
            SetupValue::Text("12".into())
        );
        let d = crate::diff(&setup, &other);
        assert_eq!(d.change_count(), 1);
        assert_eq!(d.groups[0].entries[0].delta(), None);
        assert_eq!(d.groups[0].entries[0].percent_change(), None);
        assert_eq!(d.groups[0].entries[0].summary(), "12 -> 11");
    }

    #[test]
    fn ngp_suffixed_keys_still_resolve_their_unit() {
        assert_eq!(unit_for("BumpStopStiffnessFront_NGP"), Some("N/m"));
        assert_eq!(unit_for("HighSpeedDampingReboundFront_NGP"), Some("Ns/m"));
        assert_eq!(unit_for("MaxBrakePressureFront"), Some("Pa"));
        assert_eq!(unit_for("FrontDiffMaxTorque"), Some("Nm"));
        assert_eq!(unit_for("HandbrakePercentage_NGP"), None);
        assert_eq!(unit_for("GearId0"), None);
    }

    #[test]
    fn keys_prettify_readably() {
        assert_eq!(
            prettify("FrontRollBarStiffness"),
            "Front Roll Bar Stiffness"
        );
        assert_eq!(
            prettify("BumpStopStiffnessRear_NGP"),
            "Bump Stop Stiffness Rear (NGP)"
        );
        assert_eq!(prettify("vecTopMountPosition"), "Top Mount Position");
        assert_eq!(prettify("VehicleControlUnit"), "Vehicle Control Unit");
        assert_eq!(prettify("CenterDiffThrottle_00"), "Center Diff Throttle 00");
        assert_eq!(prettify("GearId0"), "Gear Id0");
        // Corner markers stay whole.
        assert_eq!(prettify("SpringDamperLF"), "Spring Damper LF");
        assert_eq!(prettify("TyreRB"), "Tyre RB");
        assert_eq!(prettify("MaxSteeringLock"), "Max Steering Lock");
    }

    /// The payoff case: the two real captures differ in setup, and the
    /// diff should surface exactly the values that changed. Skips when the
    /// gitignored sample data isn't present (i.e. in CI).
    #[test]
    fn diffs_the_two_real_sample_setups_when_available() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.sample-data/RBR/MINI JCW - Gabiria-Legazpi 2004");
        let (left, right) = (
            root.join("Run1/setup/Tarmac Bumpy.lsp"),
            root.join("Run2/setup/my tarmac test.lsp"),
        );
        if !left.exists() || !right.exists() {
            return;
        }

        let left = load_lsp(&left).expect("Run1 setup should load");
        let right = load_lsp(&right).expect("Run2 setup should load");
        assert_eq!(left.name, "Tarmac Bumpy");
        assert_eq!(left.entry_count(), 274);

        let d = crate::diff(&left, &right);
        // 73 values differ between the two runs, front springs
        // 26000 -> 45500 N/m among them. PROJECT_PLAN.md's capture notes
        // said 58; that was an undercount from the original manual
        // investigation — a plain line diff of the two files (ignoring the
        // trailing NULs) reports 73 differing value lines, agreeing with
        // this parser exactly.
        assert_eq!(d.change_count(), 73);
        let front_spring = d
            .groups
            .iter()
            .find(|g| g.key == "SpringDamperLF")
            .and_then(|g| g.entries.iter().find(|e| e.key == "SpringStiffness"))
            .expect("front spring stiffness should differ");
        assert_eq!(front_spring.delta(), Some(19500.0));
        assert_eq!(front_spring.summary(), "26000 -> 45500 N/m (+19500)");
        // Identity: a setup diffed against itself has no changes.
        assert!(crate::diff(&left, &left).is_empty());
    }
}
