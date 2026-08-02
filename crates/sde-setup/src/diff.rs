//! Diffing two setup sheets.
//!
//! The output keeps only what *differs*, grouped the same way the sheets
//! are: a 274-entry RBR setup differs in 73 values between the two real
//! sample runs, and showing all 274 to surface those 73 is exactly the
//! flat property dump PROJECT_PLAN.md's UI/UX design note (principle 6)
//! rejects.
//!
//! Entries are matched by key within a group, and groups by key — never
//! by position, since two sheets from different sim/plugin versions can
//! carry different entry counts. An entry present in only one side is
//! reported as such rather than silently dropped: a setup that gained a
//! field is a real difference between the two, just not a numeric one.

use crate::model::{Setup, SetupEntry, SetupValue};

/// How one entry differs.
#[derive(Debug, Clone, PartialEq)]
pub enum SetupChange {
    /// Present in both, with different values.
    Changed { left: SetupValue, right: SetupValue },
    /// Present only in the left setup.
    OnlyLeft(SetupValue),
    /// Present only in the right setup.
    OnlyRight(SetupValue),
}

/// One differing entry.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryDiff {
    pub key: String,
    pub label: String,
    pub unit: Option<String>,
    pub change: SetupChange,
}

impl EntryDiff {
    /// Signed change, for entries numeric on both sides. `None` for
    /// vectors, text, and one-sided entries — there's no meaningful
    /// arithmetic difference against a value that isn't there.
    #[must_use]
    pub fn delta(&self) -> Option<f64> {
        match &self.change {
            SetupChange::Changed { left, right } => Some(right.as_number()? - left.as_number()?),
            _ => None,
        }
    }

    /// Change as a percentage of the left value's *magnitude*. `None`
    /// under the same conditions as [`EntryDiff::delta`], and additionally
    /// when the left value is zero (an increase from nothing has no
    /// percentage).
    ///
    /// Dividing by the magnitude rather than the signed value keeps the
    /// sign agreeing with [`EntryDiff::delta`] for negative quantities —
    /// RBR's `WheelAxisInclination` going -0.057 -> -0.070 is a *negative*
    /// change of 22%, not a positive one, however the reference value is
    /// signed.
    #[must_use]
    pub fn percent_change(&self) -> Option<f64> {
        let SetupChange::Changed { left, .. } = &self.change else {
            return None;
        };
        let left = left.as_number()?;
        if left == 0.0 {
            return None;
        }
        Some(self.delta()? / left.abs() * 100.0)
    }

    /// One-line description, e.g. `"26000 -> 45500 N/m (+19500)"`.
    #[must_use]
    pub fn summary(&self) -> String {
        let unit = self
            .unit
            .as_ref()
            .map_or(String::new(), |u| format!(" {u}"));
        match &self.change {
            SetupChange::Changed { left, right } => {
                let delta = self
                    .delta()
                    .map_or(String::new(), |d| format!(" ({})", signed(d)));
                format!("{} -> {}{unit}{delta}", left.display(), right.display())
            }
            SetupChange::OnlyLeft(v) => format!("{}{unit} -> (absent)", v.display()),
            SetupChange::OnlyRight(v) => format!("(absent) -> {}{unit}", v.display()),
        }
    }
}

fn signed(v: f64) -> String {
    let magnitude = SetupValue::Number(v.abs()).display();
    if v < 0.0 {
        format!("-{magnitude}")
    } else {
        format!("+{magnitude}")
    }
}

/// Differing entries within one group.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupDiff {
    pub key: String,
    pub name: String,
    pub entries: Vec<EntryDiff>,
}

/// The differences between two setups.
#[derive(Debug, Clone, PartialEq)]
pub struct SetupDiff {
    pub left_name: String,
    pub right_name: String,
    /// Only groups with at least one differing entry, in left-setup order
    /// (then any group present only on the right, in right order).
    pub groups: Vec<GroupDiff>,
}

impl SetupDiff {
    /// Total differing entries.
    #[must_use]
    pub fn change_count(&self) -> usize {
        self.groups.iter().map(|g| g.entries.len()).sum()
    }

    /// True if the two setups are identical as far as this model can see.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.change_count() == 0
    }
}

/// Compare two setups, keeping only what differs.
#[must_use]
pub fn diff(left: &Setup, right: &Setup) -> SetupDiff {
    let mut groups = Vec::new();

    for lg in &left.groups {
        let Some(rg) = right.group(&lg.key) else {
            // A whole group missing on the right: every entry is one-sided.
            let entries = lg.entries.iter().map(one_sided_left).collect::<Vec<_>>();
            push_nonempty(&mut groups, lg.key.clone(), lg.name.clone(), entries);
            continue;
        };

        let mut entries = Vec::new();
        for le in &lg.entries {
            match rg.entry(&le.key) {
                Some(re) if re.value == le.value => {}
                Some(re) => entries.push(EntryDiff {
                    key: le.key.clone(),
                    label: le.label.clone(),
                    // The left sheet's unit wins: it's the reference side,
                    // and a unit disagreeing between two sheets of the same
                    // sim would be an adapter bug, not data to surface here.
                    unit: le.unit.clone(),
                    change: SetupChange::Changed {
                        left: le.value.clone(),
                        right: re.value.clone(),
                    },
                }),
                None => entries.push(one_sided_left(le)),
            }
        }
        // Entries the right side added, in right-side order.
        for re in &rg.entries {
            if lg.entry(&re.key).is_none() {
                entries.push(EntryDiff {
                    key: re.key.clone(),
                    label: re.label.clone(),
                    unit: re.unit.clone(),
                    change: SetupChange::OnlyRight(re.value.clone()),
                });
            }
        }

        push_nonempty(&mut groups, lg.key.clone(), lg.name.clone(), entries);
    }

    for rg in &right.groups {
        if left.group(&rg.key).is_some() {
            continue;
        }
        let entries = rg
            .entries
            .iter()
            .map(|re| EntryDiff {
                key: re.key.clone(),
                label: re.label.clone(),
                unit: re.unit.clone(),
                change: SetupChange::OnlyRight(re.value.clone()),
            })
            .collect::<Vec<_>>();
        push_nonempty(&mut groups, rg.key.clone(), rg.name.clone(), entries);
    }

    SetupDiff {
        left_name: left.name.clone(),
        right_name: right.name.clone(),
        groups,
    }
}

fn one_sided_left(entry: &SetupEntry) -> EntryDiff {
    EntryDiff {
        key: entry.key.clone(),
        label: entry.label.clone(),
        unit: entry.unit.clone(),
        change: SetupChange::OnlyLeft(entry.value.clone()),
    }
}

fn push_nonempty(groups: &mut Vec<GroupDiff>, key: String, name: String, entries: Vec<EntryDiff>) {
    if !entries.is_empty() {
        groups.push(GroupDiff { key, name, entries });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SetupGroup, SetupValue};

    fn entry(key: &str, value: SetupValue) -> SetupEntry {
        SetupEntry {
            key: key.into(),
            label: key.into(),
            value,
            unit: Some("N/m".into()),
        }
    }

    fn setup(name: &str, groups: Vec<(&str, Vec<SetupEntry>)>) -> Setup {
        Setup {
            name: name.into(),
            source: "test".into(),
            car: None,
            groups: groups
                .into_iter()
                .map(|(key, entries)| SetupGroup {
                    key: key.into(),
                    name: key.into(),
                    entries,
                })
                .collect(),
        }
    }

    #[test]
    fn identical_setups_diff_to_nothing() {
        let a = setup(
            "a",
            vec![("Car", vec![entry("ARB", SetupValue::Number(1.0))])],
        );
        let b = a.clone();
        let d = diff(&a, &b);
        assert!(d.is_empty());
        assert!(d.groups.is_empty());
    }

    #[test]
    fn only_differing_entries_are_reported() {
        let a = setup(
            "a",
            vec![(
                "Car",
                vec![
                    entry("ARB", SetupValue::Number(16000.0)),
                    entry("Lock", SetupValue::Number(0.621)),
                ],
            )],
        );
        let b = setup(
            "b",
            vec![(
                "Car",
                vec![
                    entry("ARB", SetupValue::Number(21000.0)),
                    entry("Lock", SetupValue::Number(0.621)),
                ],
            )],
        );

        let d = diff(&a, &b);
        assert_eq!(d.change_count(), 1);
        let changed = &d.groups[0].entries[0];
        assert_eq!(changed.key, "ARB");
        assert_eq!(changed.delta(), Some(5000.0));
        assert_eq!(changed.summary(), "16000 -> 21000 N/m (+5000)");
    }

    #[test]
    fn negative_deltas_and_percentages() {
        let a = setup(
            "a",
            vec![("Car", vec![entry("ARB", SetupValue::Number(20000.0))])],
        );
        let b = setup(
            "b",
            vec![("Car", vec![entry("ARB", SetupValue::Number(15000.0))])],
        );
        let d = diff(&a, &b);
        let e = &d.groups[0].entries[0];
        assert_eq!(e.delta(), Some(-5000.0));
        assert_eq!(e.percent_change(), Some(-25.0));
        assert_eq!(e.summary(), "20000 -> 15000 N/m (-5000)");
    }

    #[test]
    fn percent_change_sign_follows_the_delta_for_negative_values() {
        let a = setup(
            "a",
            vec![("Car", vec![entry("Camber", SetupValue::Number(-0.04))])],
        );
        let b = setup(
            "b",
            vec![("Car", vec![entry("Camber", SetupValue::Number(-0.05))])],
        );
        let e = &diff(&a, &b).groups[0].entries[0];
        // Exact equality would fail on the binary representation of these
        // decimals (-0.05 - -0.04 is -0.010000000000000002).
        assert!((e.delta().unwrap() + 0.01).abs() < 1e-12);
        assert!((e.percent_change().unwrap() + 25.0).abs() < 1e-9);
    }

    #[test]
    fn percent_change_is_absent_when_the_left_value_is_zero() {
        let a = setup(
            "a",
            vec![("Car", vec![entry("ARB", SetupValue::Number(0.0))])],
        );
        let b = setup(
            "b",
            vec![("Car", vec![entry("ARB", SetupValue::Number(5.0))])],
        );
        let e = &diff(&a, &b).groups[0].entries[0];
        assert_eq!(e.delta(), Some(5.0));
        assert_eq!(e.percent_change(), None);
    }

    #[test]
    fn one_sided_entries_and_groups_are_reported_not_dropped() {
        let a = setup(
            "a",
            vec![
                ("Car", vec![entry("ARB", SetupValue::Number(1.0))]),
                ("Gone", vec![entry("X", SetupValue::Number(2.0))]),
            ],
        );
        let b = setup(
            "b",
            vec![
                ("Car", vec![entry("NewKey", SetupValue::Number(3.0))]),
                ("Added", vec![entry("Y", SetupValue::Number(4.0))]),
            ],
        );

        let d = diff(&a, &b);
        assert_eq!(
            d.groups.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(),
            ["Car", "Gone", "Added"]
        );
        // Within `Car`: the left-only entry first, then the right-only one.
        assert_eq!(
            d.groups[0]
                .entries
                .iter()
                .map(|e| e.change.clone())
                .collect::<Vec<_>>(),
            [
                SetupChange::OnlyLeft(SetupValue::Number(1.0)),
                SetupChange::OnlyRight(SetupValue::Number(3.0)),
            ]
        );
        assert_eq!(d.groups[0].entries[0].summary(), "1 N/m -> (absent)");
        assert_eq!(d.groups[0].entries[1].summary(), "(absent) -> 3 N/m");
        // No arithmetic against a value that isn't there.
        assert_eq!(d.groups[0].entries[0].delta(), None);
    }

    #[test]
    fn entries_match_by_key_not_position() {
        let a = setup(
            "a",
            vec![(
                "Car",
                vec![
                    entry("ARB", SetupValue::Number(1.0)),
                    entry("Lock", SetupValue::Number(2.0)),
                ],
            )],
        );
        // Same values, reversed order, plus one extra in between.
        let b = setup(
            "b",
            vec![(
                "Car",
                vec![
                    entry("Lock", SetupValue::Number(2.0)),
                    entry("Extra", SetupValue::Number(9.0)),
                    entry("ARB", SetupValue::Number(1.0)),
                ],
            )],
        );

        let d = diff(&a, &b);
        assert_eq!(d.change_count(), 1);
        assert_eq!(d.groups[0].entries[0].key, "Extra");
    }

    #[test]
    fn vector_and_text_values_diff_as_a_whole_without_a_delta() {
        let a = setup(
            "a",
            vec![(
                "WheelLF",
                vec![
                    entry("Pos", SetupValue::Vector(vec![0.592, -2.557, 0.74])),
                    entry("Compound", SetupValue::Text("Tarmac Dry".into())),
                ],
            )],
        );
        let b = setup(
            "b",
            vec![(
                "WheelLF",
                vec![
                    entry("Pos", SetupValue::Vector(vec![0.592, -2.557, 0.80])),
                    entry("Compound", SetupValue::Text("Gravel".into())),
                ],
            )],
        );

        let d = diff(&a, &b);
        assert_eq!(d.change_count(), 2);
        assert_eq!(d.groups[0].entries[0].delta(), None);
        assert_eq!(
            d.groups[0].entries[0].summary(),
            "0.592, -2.557, 0.74 -> 0.592, -2.557, 0.8 N/m"
        );
        assert_eq!(d.groups[0].entries[1].summary(), "Tarmac Dry -> Gravel N/m");
    }

    #[test]
    fn group_and_entry_matching_is_case_insensitive() {
        let a = setup(
            "a",
            vec![("Car", vec![entry("ARB", SetupValue::Number(1.0))])],
        );
        let b = setup(
            "b",
            vec![("car", vec![entry("arb", SetupValue::Number(1.0))])],
        );
        assert!(diff(&a, &b).is_empty());
    }
}
