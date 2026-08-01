//! Parser for RBR/RSF car setup sheets (`.lsp`).
//!
//! These are Lisp-style s-expressions written by RBR itself into
//! `SavedGames\<CarPhysicsFolder>\`, and embedded verbatim inside `.rpl`
//! replays. This parser reads the standalone file (see PROJECT_PLAN.md's
//! "Decide where the `.lsp` setup parser lives" entry for why the embedded
//! copy is deliberately not the first target).
//!
//! Shape, from the two real captures in `.sample-data/`:
//!
//! ```text
//! (("CarSetup"
//!  Car             ("Car"
//!  MaxSteeringLock 0.621000
//!                   FrontRollBarStiffness 16000.000000
//!                                    MaxSteeringLock
//!                   FrontRollBarStiffness
//!                   )
//!  ...
//!  ))
//! ```
//!
//! Two quirks the parser has to handle, both confirmed against real files:
//!
//! - **Every section repeats its key list as a trailer with no values.**
//!   A key followed by no numbers is a trailer entry and is dropped — the
//!   only reliable way to tell the two apart, since the trailer uses the
//!   same names in the same order.
//! - **A few keys carry more than one value** (`vecTopMountPosition` has
//!   three), so an entry holds a list of values, not one.
//!
//! Whitespace and line breaks carry no meaning — the file is tokenized as
//! s-expressions rather than line-by-line, so a differently-formatted
//! writer (or the copy embedded in a `.rpl`) parses the same way. One
//! sample file ends with three NUL bytes; stray bytes outside the
//! expression are ignored.

use std::path::Path;

use crate::error::RbrError;
use crate::ini::decode_text;

/// One `Key value...` pair inside a setup section.
#[derive(Debug, Clone, PartialEq)]
pub struct LspEntry {
    /// Key exactly as written, e.g. `"FrontRollBarStiffness"`.
    pub key: String,
    /// One value for almost every key; three for `vecTopMountPosition`.
    /// Never empty — a key with no values is a trailer entry and is
    /// dropped during parsing.
    pub values: Vec<f64>,
}

impl LspEntry {
    /// The single value, or `None` for a multi-value entry.
    #[must_use]
    pub fn scalar(&self) -> Option<f64> {
        match self.values.as_slice() {
            [v] => Some(*v),
            _ => None,
        }
    }
}

/// One `Name ("tag" ...)` group, e.g. `Drive`, `SpringDamperLF`, `TyreRB`.
#[derive(Debug, Clone, PartialEq)]
pub struct LspSection {
    /// Section name as written, e.g. `"SpringDamperLF"`.
    pub name: String,
    /// The quoted string the section body opens with — `"Car"` for the
    /// first section and `":-D"` for every other one in both real files.
    /// Kept verbatim rather than interpreted; nothing is known to depend
    /// on it.
    pub tag: Option<String>,
    /// Entries in file order.
    pub entries: Vec<LspEntry>,
}

impl LspSection {
    /// First entry with this key (case-insensitive).
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&LspEntry> {
        self.entries
            .iter()
            .find(|e| e.key.eq_ignore_ascii_case(key))
    }
}

/// A parsed `.lsp` setup sheet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LspSetup {
    /// The document's own leading string, `"CarSetup"` in both real files.
    pub title: Option<String>,
    /// Sections in file order.
    pub sections: Vec<LspSection>,
}

impl LspSetup {
    /// First section with this name (case-insensitive).
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&LspSection> {
        self.sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Total number of value-carrying entries across every section — the
    /// figure to sanity-check a parse against (274 for both real files).
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.sections.iter().map(|s| s.entries.len()).sum()
    }
}

/// Parse a setup `.lsp` from disk.
///
/// # Errors
///
/// Returns [`RbrError::Io`] if the file can't be read. Parsing itself
/// doesn't fail: a truncated or unexpectedly-shaped file yields whatever
/// sections were readable, matching this crate's convention that only the
/// read itself is fallible.
pub fn parse_lsp(path: &Path) -> Result<LspSetup, RbrError> {
    let bytes = std::fs::read(path).map_err(|source| RbrError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(parse_lsp_str(&decode_text(&bytes)))
}

/// Parse setup `.lsp` text already in memory.
#[must_use]
pub fn parse_lsp_str(text: &str) -> LspSetup {
    let tokens = tokenize(text);
    let mut pos = 0usize;

    // The document is `(( "CarSetup" <sections> ))`. Descend through the
    // outer parens to the body rather than asserting the exact nesting, so
    // a writer that omits the outer wrapper still parses.
    while matches!(tokens.get(pos), Some(Token::Open)) {
        pos += 1;
        if matches!(tokens.get(pos), Some(Token::Str(_) | Token::Atom(_))) {
            break;
        }
    }

    let title = match tokens.get(pos) {
        Some(Token::Str(s)) => {
            pos += 1;
            Some(s.clone())
        }
        _ => None,
    };

    let mut sections = Vec::new();
    while pos < tokens.len() {
        match &tokens[pos] {
            // `Name (` opens a section; anything else at this level is
            // structure (the closing parens) or unexpected, and is skipped.
            Token::Atom(name) if matches!(tokens.get(pos + 1), Some(Token::Open)) => {
                let name = name.clone();
                pos += 2;
                let (section, next) = parse_section(name, &tokens, pos);
                sections.push(section);
                pos = next;
            }
            _ => pos += 1,
        }
    }

    LspSetup { title, sections }
}

/// Parse one section body, starting just after its `(`. Returns the
/// section and the index just past its closing `)`.
fn parse_section(name: String, tokens: &[Token], mut pos: usize) -> (LspSection, usize) {
    let tag = match tokens.get(pos) {
        Some(Token::Str(s)) => {
            pos += 1;
            Some(s.clone())
        }
        _ => None,
    };

    let mut entries: Vec<LspEntry> = Vec::new();
    while pos < tokens.len() {
        match &tokens[pos] {
            Token::Close => {
                pos += 1;
                break;
            }
            Token::Atom(atom) => {
                pos += 1;
                match parse_number(atom) {
                    // A number belongs to the entry currently being built.
                    // One with no entry to attach to (a malformed file)
                    // is dropped rather than inventing a key for it.
                    Some(v) => {
                        if let Some(last) = entries.last_mut() {
                            last.values.push(v);
                        }
                    }
                    None => entries.push(LspEntry {
                        key: atom.clone(),
                        values: Vec::new(),
                    }),
                }
            }
            // Strings and nested lists aren't part of any observed section
            // body; skip rather than abandoning the section.
            _ => pos += 1,
        }
    }

    // Drop the trailing repeat of the key list (every key, no values).
    entries.retain(|e| !e.values.is_empty());

    (LspSection { name, tag, entries }, pos)
}

/// Parse a value atom. RBR writes plain decimals, some explicitly signed
/// (`+0.592000`), and bare integers for gear ids — all of which
/// `f64::from_str` accepts. Deliberately *not* a general number parser:
/// anything else (including the `inf`/`nan` spellings `from_str` would
/// otherwise accept) is treated as a key, since a key that looked like a
/// number would silently corrupt the entry before it.
fn parse_number(atom: &str) -> Option<f64> {
    let digits = atom.strip_prefix(['+', '-']).unwrap_or(atom);
    if digits.is_empty() || !digits.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    if !digits
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-')
    {
        return None;
    }
    atom.parse().ok()
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Open,
    Close,
    /// A double-quoted string.
    Str(String),
    /// Any other run of non-whitespace, non-delimiter characters.
    Atom(String),
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '(' => tokens.push(Token::Open),
            ')' => tokens.push(Token::Close),
            '"' => {
                // Unterminated strings run to end of input rather than
                // failing; there's no escape syntax in these files.
                let mut s = String::new();
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    s.push(c);
                }
                tokens.push(Token::Str(s));
            }
            c if c.is_whitespace() || c == '\0' => {}
            c => {
                let mut atom = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || matches!(next, '(' | ')' | '"' | '\0') {
                        break;
                    }
                    atom.push(next);
                    chars.next();
                }
                tokens.push(Token::Atom(atom));
            }
        }
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped exactly like the real files, trailer duplicates included.
    const SAMPLE: &str = r#"(("CarSetup"
 Car             ("Car"
 MaxSteeringLock 0.621000
                  FrontRollBarStiffness 16000.000000
                                   MaxSteeringLock
                  FrontRollBarStiffness
                  )
 WheelLF         (":-D"
 vecTopMountPosition +0.592000 -2.557000 +0.740000
                  TopMountSlot    3.000000
                                   vecTopMountPosition
                  TopMountSlot
                  )
 Drive           (":-D"
 GearId0         1
                  DropGearId      12
                                   GearId0
                  DropGearId
                  )
 ))"#;

    #[test]
    fn parses_sections_entries_and_title() {
        let setup = parse_lsp_str(SAMPLE);

        assert_eq!(setup.title.as_deref(), Some("CarSetup"));
        assert_eq!(
            setup
                .sections
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["Car", "WheelLF", "Drive"]
        );
        assert_eq!(setup.section("Car").unwrap().tag.as_deref(), Some("Car"));
        assert_eq!(
            setup.section("WheelLF").unwrap().tag.as_deref(),
            Some(":-D")
        );
        assert_eq!(
            setup
                .section("Car")
                .unwrap()
                .entry("FrontRollBarStiffness")
                .and_then(LspEntry::scalar),
            Some(16000.0)
        );
        // Integers parse as values, not as keys.
        assert_eq!(
            setup
                .section("Drive")
                .unwrap()
                .entry("DropGearId")
                .and_then(LspEntry::scalar),
            Some(12.0)
        );
    }

    #[test]
    fn trailing_key_list_is_dropped_not_counted_twice() {
        let setup = parse_lsp_str(SAMPLE);
        // 2 + 2 + 2 entries, each exactly once despite the trailers.
        assert_eq!(setup.entry_count(), 6);
        assert_eq!(setup.section("Car").unwrap().entries.len(), 2);
    }

    #[test]
    fn multi_value_entries_keep_every_value() {
        let setup = parse_lsp_str(SAMPLE);
        let entry = setup
            .section("WheelLF")
            .unwrap()
            .entry("vecTopMountPosition")
            .unwrap();
        assert_eq!(entry.values, [0.592, -2.557, 0.740]);
        // Multi-value entries have no scalar reading.
        assert_eq!(entry.scalar(), None);
    }

    #[test]
    fn trailing_nul_bytes_and_stray_whitespace_are_ignored() {
        // Run1's real file ends with three NULs after the closing parens.
        let setup = parse_lsp_str(&format!("{SAMPLE}\0\0\0"));
        assert_eq!(setup.entry_count(), 6);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let setup = parse_lsp_str(SAMPLE);
        assert!(setup.section("springdamperlf").is_none());
        assert!(setup
            .section("CAR")
            .unwrap()
            .entry("maxsteeringlock")
            .is_some());
    }

    #[test]
    fn malformed_input_yields_what_was_readable() {
        // Truncated mid-section: no closing parens at all.
        let setup = parse_lsp_str("((\"CarSetup\"\n Car (\"Car\"\n SpringLength 0.230000");
        assert_eq!(
            setup
                .section("Car")
                .unwrap()
                .entry("SpringLength")
                .and_then(LspEntry::scalar),
            Some(0.23)
        );
    }

    #[test]
    fn number_atoms_are_recognized_but_word_atoms_are_not() {
        assert_eq!(parse_number("0.621000"), Some(0.621));
        assert_eq!(parse_number("+0.592000"), Some(0.592));
        assert_eq!(parse_number("-2.557000"), Some(-2.557));
        assert_eq!(parse_number("12"), Some(12.0));
        assert_eq!(parse_number("GearId0"), None);
        assert_eq!(parse_number("HandbrakePercentage_NGP"), None);
        // `f64::from_str` accepts these; a key spelled this way must not
        // be swallowed as a value of the preceding entry.
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("NaN"), None);
    }

    /// Runs against the real captures when present, skips in CI — same
    /// pattern as `replay.rs`'s on-disk test.
    #[test]
    fn parses_the_real_sample_setups_when_available() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.sample-data/RBR/MINI JCW - Gabiria-Legazpi 2004");
        for rel in [
            "Run1/setup/Tarmac Bumpy.lsp",
            "Run2/setup/my tarmac test.lsp",
        ] {
            let path = root.join(rel);
            if !path.exists() {
                continue;
            }
            let setup = parse_lsp(&path).expect("sample setup should read");
            assert_eq!(setup.title.as_deref(), Some("CarSetup"));
            // 16 sections / 274 entries — the counts confirmed against
            // both captures (PROJECT_PLAN.md's "274 key/value pairs").
            assert_eq!(setup.sections.len(), 16);
            assert_eq!(setup.entry_count(), 274);
            // Tyre pressures are in Pa and differ between the two runs
            // (195 kPa vs 200 kPa), so check the magnitude, not a value.
            let pressure = setup
                .section("TyreLF")
                .unwrap()
                .entry("Pressure")
                .and_then(LspEntry::scalar)
                .expect("tyre pressure should parse");
            assert!(
                (100_000.0..=300_000.0).contains(&pressure),
                "implausible tyre pressure {pressure} Pa in {rel}"
            );
        }
    }
}
