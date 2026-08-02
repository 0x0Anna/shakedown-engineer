//! Slint-free logic behind the app's setup panel: locating the `.lsp`
//! setup a loaded run used, and flattening a [`sde_setup::Setup`] or a
//! [`sde_setup::SetupDiff`] into display rows.
//!
//! Kept out of `main.rs` for the same reason `graph.rs` and
//! `replay_check.rs` are: it's ordinary logic with real edge cases (RSF
//! writes Windows-style relative paths regardless of host OS), and it's
//! worth unit testing without a display.

use std::path::{Path, PathBuf};

use sde_setup::{Setup, SetupChange, SetupDiff};

/// One line in the setup panel.
///
/// Group headers and value rows share one flat list rather than nesting,
/// because that's what Slint's `ListView` consumes — the panel renders a
/// header differently based on `is_group`, it doesn't lay out a tree.
#[derive(Debug, Clone, PartialEq)]
pub struct SetupRow {
    /// True for a group header (`label` is the group name, the value
    /// fields are empty).
    pub is_group: bool,
    pub label: String,
    /// The value, or `"old -> new"` for a diff row.
    pub value: String,
    /// Secondary annotation — the percentage change on a diff row, empty
    /// otherwise.
    pub detail: String,
}

impl SetupRow {
    fn group(label: impl Into<String>) -> Self {
        Self {
            is_group: true,
            label: label.into(),
            value: String::new(),
            detail: String::new(),
        }
    }
}

/// Locate the `.lsp` file a replay `.ini`'s `SetupName` refers to.
///
/// RSF writes these install-root-relative with Windows separators (e.g.
/// `savedgames\Mini_JCW_WRC_ngp6\Tarmac Bumpy.lsp`) regardless of what
/// wrote them, so the path is split on both separators and rejoined
/// rather than handed to `Path` as-is.
///
/// Tries, in order: the path relative to the install root; the part after
/// a leading `savedgames` component rejoined onto
/// [`sde_rbr::InstallPaths::saved_games_dir`] (so a relocated saved-games
/// folder via `PathOverrides` still resolves); and the value as an
/// absolute path. Returns the first that exists, or `None` — a setup file
/// that's been renamed or deleted since the run isn't an error, just
/// missing context.
#[must_use]
pub fn resolve_setup_path(paths: &sde_rbr::InstallPaths, setup_name: &str) -> Option<PathBuf> {
    let components: Vec<&str> = setup_name
        .split(['\\', '/'])
        .filter(|c| !c.is_empty() && *c != ".")
        .collect();
    if components.is_empty() {
        return None;
    }

    let mut candidates = vec![join_all(&paths.root, &components)];

    if components[0].eq_ignore_ascii_case("savedgames") && components.len() > 1 {
        candidates.push(join_all(&paths.saved_games_dir, &components[1..]));
    }

    let as_given = PathBuf::from(setup_name);
    if as_given.is_absolute() {
        candidates.push(as_given);
    }

    candidates.into_iter().find(|p| p.is_file())
}

fn join_all(base: &Path, components: &[&str]) -> PathBuf {
    let mut path = base.to_path_buf();
    for component in components {
        path.push(component);
    }
    path
}

/// Flatten a setup into display rows: one header per group, one row per
/// entry.
#[must_use]
pub fn rows_for_setup(setup: &Setup) -> Vec<SetupRow> {
    let mut rows = Vec::with_capacity(setup.entry_count() + setup.groups.len());
    for group in &setup.groups {
        rows.push(SetupRow::group(&group.name));
        for entry in &group.entries {
            rows.push(SetupRow {
                is_group: false,
                label: entry.label.clone(),
                value: entry.display(),
                detail: String::new(),
            });
        }
    }
    rows
}

/// Flatten a diff into display rows. Only differing entries appear (see
/// [`sde_setup::diff`]), so a group header here means "this group changed".
#[must_use]
pub fn rows_for_diff(diff: &SetupDiff) -> Vec<SetupRow> {
    let mut rows = Vec::with_capacity(diff.change_count() + diff.groups.len());
    for group in &diff.groups {
        rows.push(SetupRow::group(&group.name));
        for entry in &group.entries {
            let value = match &entry.change {
                SetupChange::Changed { left, right } => {
                    let unit = entry
                        .unit
                        .as_ref()
                        .map_or(String::new(), |u| format!(" {u}"));
                    format!("{} → {}{unit}", left.display(), right.display())
                }
                // The one-sided cases already read as sentences in
                // `summary()`; no point restating them differently here.
                _ => entry.summary(),
            };
            rows.push(SetupRow {
                is_group: false,
                label: entry.label.clone(),
                value,
                detail: entry
                    .percent_change()
                    .map_or(String::new(), |p| format!("{p:+.1}%")),
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use sde_setup::{Setup, SetupEntry, SetupGroup, SetupValue};

    fn install_paths(root: &Path) -> sde_rbr::InstallPaths {
        sde_rbr::InstallConfig::new(root.to_path_buf()).resolve()
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"()").unwrap();
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sde-app-setup-view-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolves_an_rsf_style_relative_setup_path() {
        let root = temp_dir("relative");
        let setup = root.join("savedgames/Mini_JCW_WRC_ngp6/Tarmac Bumpy.lsp");
        touch(&setup);

        let found = resolve_setup_path(
            &install_paths(&root),
            "savedgames\\Mini_JCW_WRC_ngp6\\Tarmac Bumpy.lsp",
        );
        assert_eq!(found, Some(setup));
    }

    #[test]
    fn resolves_through_a_relocated_saved_games_folder() {
        // The install root has no `savedgames\` at all; the real one lives
        // elsewhere and is pointed at via a path override.
        let root = temp_dir("override-root");
        let elsewhere = temp_dir("override-saves");
        let setup = elsewhere.join("Mini_JCW_WRC_ngp6/Tarmac Bumpy.lsp");
        touch(&setup);

        let paths = sde_rbr::InstallConfig {
            root,
            overrides: sde_rbr::PathOverrides {
                saved_games_dir: Some(elsewhere),
                ..sde_rbr::PathOverrides::default()
            },
        }
        .resolve();

        let found = resolve_setup_path(&paths, "savedgames\\Mini_JCW_WRC_ngp6\\Tarmac Bumpy.lsp");
        assert_eq!(found, Some(setup));
    }

    #[test]
    fn missing_or_empty_setup_names_resolve_to_nothing() {
        let root = temp_dir("missing");
        let paths = install_paths(&root);
        assert_eq!(
            resolve_setup_path(&paths, "savedgames\\Nope\\Gone.lsp"),
            None
        );
        assert_eq!(resolve_setup_path(&paths, ""), None);
    }

    fn sample_setup(name: &str, arb: f64, lock: f64) -> Setup {
        Setup {
            name: name.into(),
            source: "test".into(),
            car: None,
            groups: vec![SetupGroup {
                key: "Car".into(),
                name: "Car".into(),
                entries: vec![
                    SetupEntry {
                        key: "FrontRollBarStiffness".into(),
                        label: "Front Roll Bar Stiffness".into(),
                        value: SetupValue::Number(arb),
                        unit: Some("N/m".into()),
                    },
                    SetupEntry {
                        key: "MaxSteeringLock".into(),
                        label: "Max Steering Lock".into(),
                        value: SetupValue::Number(lock),
                        unit: None,
                    },
                ],
            }],
        }
    }

    #[test]
    fn setup_rows_carry_a_header_per_group_and_a_row_per_entry() {
        let rows = rows_for_setup(&sample_setup("a", 16000.0, 0.621));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], SetupRow::group("Car"));
        assert_eq!(rows[1].label, "Front Roll Bar Stiffness");
        assert_eq!(rows[1].value, "16000 N/m");
        assert!(!rows[1].is_group);
        // No unit known, so none shown.
        assert_eq!(rows[2].value, "0.621");
    }

    #[test]
    fn diff_rows_show_only_what_changed_with_a_percentage() {
        let diff = sde_setup::diff(
            &sample_setup("a", 16000.0, 0.621),
            &sample_setup("b", 21000.0, 0.621),
        );
        let rows = rows_for_diff(&diff);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], SetupRow::group("Car"));
        assert_eq!(rows[1].value, "16000 → 21000 N/m");
        assert_eq!(rows[1].detail, "+31.2%");
    }

    #[test]
    fn identical_setups_produce_no_rows_at_all() {
        let diff = sde_setup::diff(
            &sample_setup("a", 16000.0, 0.621),
            &sample_setup("b", 16000.0, 0.621),
        );
        assert!(rows_for_diff(&diff).is_empty());
    }
}
