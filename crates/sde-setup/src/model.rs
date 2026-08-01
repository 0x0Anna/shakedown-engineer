//! The setup sheet data model.

/// A single adjustable value.
///
/// Numbers cover almost everything; [`SetupValue::Vector`] exists because
/// some values are genuinely multi-component (RBR's `vecTopMountPosition`
/// is an XYZ position), and [`SetupValue::Text`] for the categorical
/// settings sims express as names rather than numbers (tyre compound,
/// diff preset). Splitting these matters for [`super::diff`]: two vectors
/// differ per component, and text values can only differ as a whole.
#[derive(Debug, Clone, PartialEq)]
pub enum SetupValue {
    Number(f64),
    Vector(Vec<f64>),
    Text(String),
}

impl SetupValue {
    /// The scalar reading, for the numeric-only operations (delta,
    /// percentage change). `None` for vectors and text.
    #[must_use]
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(v) => Some(*v),
            _ => None,
        }
    }

    /// Human-readable form, without units.
    ///
    /// Numbers are printed at the shortest precision that round-trips
    /// rather than at a fixed number of decimals — a setup sheet mixes
    /// gear ids (`4`), stiffnesses (`26000`) and lengths (`0.23`), and a
    /// fixed format is wrong for at least one of them.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Number(v) => format_number(*v),
            Self::Vector(vs) => vs
                .iter()
                .map(|v| format_number(*v))
                .collect::<Vec<_>>()
                .join(", "),
            Self::Text(s) => s.clone(),
        }
    }
}

/// Format one number for display, trimming the trailing zeros that RBR's
/// fixed `%f` output is full of (`16000.000000` -> `16000`).
fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{v:.0}");
    }
    let mut s = format!("{v:.6}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

/// One adjustable item on the sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct SetupEntry {
    /// Stable identity within its group — the sim's own key, used to
    /// match entries between two setups. Compared case-insensitively.
    pub key: String,
    /// Display name. Adapters may prettify; falls back to `key`.
    pub label: String,
    pub value: SetupValue,
    /// Unit symbol (`"N/m"`, `"Pa"`, `"m"`), where the adapter knows it
    /// with confidence. `None` means unknown, *not* dimensionless — a
    /// wrong unit on a setup sheet is worse than no unit.
    pub unit: Option<String>,
}

impl SetupEntry {
    /// Value with its unit, e.g. `"26000 N/m"`.
    #[must_use]
    pub fn display(&self) -> String {
        match &self.unit {
            Some(unit) => format!("{} {unit}", self.value.display()),
            None => self.value.display(),
        }
    }
}

/// A named group of entries — one panel of the sheet (`Drive`, the front
/// left spring/damper, ...). Ordering is the sim's own.
#[derive(Debug, Clone, PartialEq)]
pub struct SetupGroup {
    /// Stable identity, matched case-insensitively between setups.
    pub key: String,
    /// Display name.
    pub name: String,
    pub entries: Vec<SetupEntry>,
}

impl SetupGroup {
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&SetupEntry> {
        self.entries
            .iter()
            .find(|e| e.key.eq_ignore_ascii_case(key))
    }
}

/// A complete setup sheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Setup {
    /// Display name — normally the setup file's stem, e.g.
    /// `"Tarmac Bumpy"`.
    pub name: String,
    /// Where it came from, e.g. `"RBR/RSF .lsp"`. Shown next to the name
    /// so a diff between two sims' sheets can't be mistaken for a
    /// like-for-like comparison.
    pub source: String,
    /// Car this setup belongs to, when the source names one. RBR's `.lsp`
    /// doesn't — the car is implied by the folder it sits in — so this is
    /// filled in by whoever has that context (the replay `.ini`, or the
    /// user).
    pub car: Option<String>,
    pub groups: Vec<SetupGroup>,
}

impl Setup {
    #[must_use]
    pub fn group(&self, key: &str) -> Option<&SetupGroup> {
        self.groups.iter().find(|g| g.key.eq_ignore_ascii_case(key))
    }

    /// Total entries across every group.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.groups.iter().map(|g| g.entries.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_display_without_trailing_zero_noise() {
        assert_eq!(SetupValue::Number(16000.0).display(), "16000");
        assert_eq!(SetupValue::Number(0.621).display(), "0.621");
        assert_eq!(SetupValue::Number(-2.557).display(), "-2.557");
        assert_eq!(SetupValue::Number(0.0).display(), "0");
    }

    #[test]
    fn vectors_and_text_display_as_written() {
        assert_eq!(
            SetupValue::Vector(vec![0.592, -2.557, 0.74]).display(),
            "0.592, -2.557, 0.74"
        );
        assert_eq!(
            SetupValue::Text("Tarmac Dry".into()).display(),
            "Tarmac Dry"
        );
    }

    #[test]
    fn entry_display_appends_the_unit_only_when_known() {
        let with = SetupEntry {
            key: "SpringStiffness".into(),
            label: "Spring stiffness".into(),
            value: SetupValue::Number(26000.0),
            unit: Some("N/m".into()),
        };
        let without = SetupEntry {
            unit: None,
            ..with.clone()
        };
        assert_eq!(with.display(), "26000 N/m");
        assert_eq!(without.display(), "26000");
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let setup = Setup {
            groups: vec![SetupGroup {
                key: "Car".into(),
                name: "Car".into(),
                entries: vec![SetupEntry {
                    key: "FrontRollBarStiffness".into(),
                    label: "Front ARB".into(),
                    value: SetupValue::Number(16000.0),
                    unit: None,
                }],
            }],
            ..Setup::default()
        };
        assert!(setup
            .group("CAR")
            .unwrap()
            .entry("frontrollbarstiffness")
            .is_some());
        assert_eq!(setup.entry_count(), 1);
    }
}
