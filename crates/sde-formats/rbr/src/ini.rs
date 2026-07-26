//! Minimal INI reader for RBR/RSF companion files.
//!
//! Hand-rolled rather than pulled from a crate to keep `sde-formats`
//! dependency-light (per PROJECT_PLAN.md's modularity principles). The
//! dialect these files use is small and regular: `; comment` lines,
//! `[Section]` headers, and `Key = Value` pairs with optional whitespace
//! around the `=` and possibly-empty values.

use std::collections::BTreeMap;

/// A parsed INI document: section name -> (key -> value).
///
/// Section and key lookup is case-insensitive (keys are folded to
/// lowercase on insert), since RSF's own writers aren't perfectly
/// consistent about casing across versions. The original casing is not
/// preserved — nothing downstream needs it.
///
/// Later duplicates win, matching how Windows' own INI APIs behave.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ini {
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl Ini {
    /// Parse INI text. Lines that are blank, comments (`;` or `#`), or
    /// otherwise unrecognized are skipped — these files are written by
    /// several different RSF/plugin versions, so being strict about stray
    /// content would break on data that's otherwise perfectly usable.
    ///
    /// Keys appearing before any `[Section]` header are filed under `""`.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut current = String::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }

            if let Some(rest) = line.strip_prefix('[') {
                if let Some(name) = rest.strip_suffix(']') {
                    current = name.trim().to_ascii_lowercase();
                    sections.entry(current.clone()).or_default();
                }
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                sections
                    .entry(current.clone())
                    .or_default()
                    .insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }

        Self { sections }
    }

    /// Raw string value for `key` in `section`, or `None` if either is
    /// absent. An explicitly empty value (`RallyName =`) returns
    /// `Some("")` — distinguishing "written but blank" from "not written
    /// at all", which is exactly the difference between a hotlap with no
    /// rally name and an older file that predates the field.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(&section.to_ascii_lowercase())?
            .get(&key.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// [`Ini::get`], but `None` for values that are present-but-empty.
    #[must_use]
    pub fn get_nonempty(&self, section: &str, key: &str) -> Option<&str> {
        self.get(section, key).filter(|v| !v.is_empty())
    }

    /// Parse a value as `T`, returning `None` if the key is missing, empty,
    /// or doesn't parse. Malformed numbers are treated as absent rather
    /// than as an error: one unreadable field shouldn't cost the caller
    /// every other field in the file.
    #[must_use]
    pub fn get_parsed<T: std::str::FromStr>(&self, section: &str, key: &str) -> Option<T> {
        self.get_nonempty(section, key)?.parse().ok()
    }

    /// True if the document has a `[section]` header at all, even an empty
    /// one. Run2's replay `.ini` has no `[RunkiSpots]` section, and that
    /// absence is meaningful (no vehicle recoveries).
    #[must_use]
    pub fn has_section(&self, section: &str) -> bool {
        self.sections.contains_key(&section.to_ascii_lowercase())
    }

    /// All key/value pairs in `section`, or an empty map if absent.
    #[must_use]
    pub fn section(&self, section: &str) -> Option<&BTreeMap<String, String>> {
        self.sections.get(&section.to_ascii_lowercase())
    }
}

/// Decode file bytes to text.
///
/// RBR predates widespread UTF-8 and its tooling writes Windows-1252, but
/// newer RSF components emit UTF-8. Valid UTF-8 is taken as-is; anything
/// else falls back to a byte-per-char Latin-1 mapping, which is lossless
/// for the accented characters that actually appear in European stage and
/// driver names. Never fails, so a stray high byte can't cost us the file.
#[must_use]
pub fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_keys_and_comments() {
        let ini = Ini::parse(
            "; a comment\n\
             [Replay]\n\
             Name = Gabiria-Legazpi 2004\n\
             MapID = 450\n\
             RallyName = \n\
             \n\
             [RunkiSpots]\n\
             Count = 2\n",
        );

        assert_eq!(ini.get("Replay", "Name"), Some("Gabiria-Legazpi 2004"));
        assert_eq!(ini.get_parsed::<u32>("Replay", "MapID"), Some(450));
        assert_eq!(ini.get_parsed::<u32>("RunkiSpots", "Count"), Some(2));
        assert_eq!(ini.get("Replay", "Nope"), None);
        assert!(!ini.has_section("Missing"));
    }

    #[test]
    fn present_but_empty_is_distinct_from_absent() {
        let ini = Ini::parse("[Replay]\nRallyName = \n");
        assert_eq!(ini.get("Replay", "RallyName"), Some(""));
        assert_eq!(ini.get_nonempty("Replay", "RallyName"), None);
        assert_eq!(ini.get("Replay", "Missing"), None);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let ini = Ini::parse("[Replay]\nMapID = 450\n");
        assert_eq!(ini.get_parsed::<u32>("REPLAY", "mapid"), Some(450));
    }

    #[test]
    fn values_may_contain_equals_and_backslashes() {
        // Paths are common values here, and `TrackSetting` style values can
        // carry an `=`; only the first `=` separates key from value.
        let ini = Ini::parse("[Replay]\nSetupName = savedgames\\Mini\\a=b.lsp\n");
        assert_eq!(
            ini.get("Replay", "SetupName"),
            Some("savedgames\\Mini\\a=b.lsp")
        );
    }

    #[test]
    fn later_duplicate_keys_win() {
        let ini = Ini::parse("[R]\nK = 1\nK = 2\n");
        assert_eq!(ini.get("R", "K"), Some("2"));
    }

    #[test]
    fn malformed_numbers_are_treated_as_absent() {
        let ini = Ini::parse("[R]\nK = notanumber\n");
        assert_eq!(ini.get_parsed::<u32>("R", "K"), None);
        assert_eq!(ini.get("R", "K"), Some("notanumber"));
    }

    #[test]
    fn decodes_utf8_and_falls_back_to_latin1() {
        assert_eq!(decode_text("Gabiria".as_bytes()), "Gabiria");
        assert_eq!(decode_text("Jyväskylä".as_bytes()), "Jyväskylä");
        // 0xE4 alone is invalid UTF-8; Latin-1 reads it as 'ä'.
        assert_eq!(decode_text(&[b'J', b'y', b'v', 0xE4]), "Jyvä");
    }
}
