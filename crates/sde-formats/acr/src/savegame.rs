//! Parser for Assetto Corsa Rally's `CarSetupsDataSaveSlot.sav` — the game's
//! own store of saved car setups (`%LOCALAPPDATA%\acr\Saved\SaveGames\`).
//!
//! The file is an Unreal `GVAS` save, but it carries no tagged properties:
//! after the standard header and an empty property list (`"None"`), the game
//! writes a custom binary payload. The layout below was worked out from a
//! real game 0.7 file (4 cars, 8 setups, parsed to the last byte) and is
//! *not* documented by the game, so every field whose meaning is a guess is
//! skipped rather than exposed.
//!
//! ```text
//! GVAS header .. class "/Script/dmengine.SaveGameSlot" .. "None" .. opaque
//! FString  "CarSetupsSaveGameData"
//! u32      opaque (0x70d5), u32 opaque (1)
//! u32      car count, then per car:
//!   FString  car id (e.g. "SkodaFabiaRSRally2")
//!   u32      setup count, then per setup:
//!     u32 opaque, u8 0xff, u32 opaque (1)
//!     FString name, FString author (often empty)
//!     i64     .NET DateTime ticks (UTC) of the save
//!     FString game version, FString car id, FString stage id
//!     u32     has-surface flag, then FString surface when 1 ("Gravel"/"Tarmac")
//!     8 bytes opaque (zero)
//!     u32     entry count, then per entry: FString key, FString value
//! ```
//!
//! Every entry value is stored as text: numbers as `"0.500000"`, parts as
//! `"(Pads:Type00_Soft)"`. Keys are dotted paths (`Brakes.FrontLeft.Disc`).
//! The parser leaves both untouched; interpreting them is the job of the
//! `sde-setup` adapter.

use std::fs;
use std::path::Path;

use crate::AcrError;

/// The FString that introduces the payload. The bytes between the GVAS
/// header and this string are opaque (a hash, a GUID and some counters), so
/// the parser locates the payload by it instead of by a fixed offset.
const PAYLOAD_TYPE: &str = "CarSetupsSaveGameData";

/// Seconds between 0001-01-01 (the .NET tick epoch) and 1970-01-01.
const UNIX_EPOCH_SECS_SINCE_YEAR_1: i64 = 62_135_596_800;

/// One saved setup: its identity plus the raw key/value pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSetup {
    /// The name the player gave it (`"Gravel Short"`). The game truncates
    /// names, so `"Greece Bumpy Te"` is not a typo in this parser.
    pub name: String,
    /// Player name stored with the setup. Empty for older saves.
    pub author: String,
    /// Time of the save as .NET `DateTime` ticks (100 ns units since
    /// 0001-01-01 UTC), exactly as stored.
    pub saved_ticks: i64,
    /// Game build that wrote it (`"0.6.0.100866"`).
    pub game_version: String,
    /// Car id (`"SkodaFabiaRSRally2"`).
    pub car_id: String,
    /// Stage id the setup was saved for (`"GreeceS4Loutraki"`).
    pub stage: String,
    /// Surface the setup was tagged with (`"Gravel"`, `"Tarmac"`), when set.
    pub surface: Option<String>,
    /// `(dotted key, text value)` in file order.
    pub entries: Vec<(String, String)>,
}

impl SavedSetup {
    /// Seconds since the Unix epoch for [`Self::saved_ticks`].
    #[must_use]
    pub fn saved_unix_secs(&self) -> i64 {
        self.saved_ticks / 10_000_000 - UNIX_EPOCH_SECS_SINCE_YEAR_1
    }

    /// Value for a dotted key, first match.
    #[must_use]
    pub fn value(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// All setups saved for one car.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarSetups {
    pub car_id: String,
    pub setups: Vec<SavedSetup>,
}

/// A parsed `CarSetupsDataSaveSlot.sav`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CarSetupsSave {
    pub cars: Vec<CarSetups>,
}

impl CarSetupsSave {
    /// Every setup across every car, in file order.
    pub fn setups(&self) -> impl Iterator<Item = &SavedSetup> {
        self.cars.iter().flat_map(|c| c.setups.iter())
    }

    /// First setup with this name (names are not unique across cars, so
    /// pass `car_id` to disambiguate).
    #[must_use]
    pub fn find(&self, name: &str, car_id: Option<&str>) -> Option<&SavedSetup> {
        self.cars
            .iter()
            .filter(|c| car_id.is_none_or(|id| c.car_id.eq_ignore_ascii_case(id)))
            .flat_map(|c| c.setups.iter())
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }
}

/// Read and parse a `CarSetupsDataSaveSlot.sav`.
///
/// # Errors
///
/// [`AcrError::Io`] if the file can't be read, [`AcrError::NotSetupSave`] if
/// it isn't a GVAS file holding car setups, and [`AcrError::TruncatedSave`]
/// if the payload ends early or disagrees with its own counts.
pub fn parse_car_setups(path: &Path) -> Result<CarSetupsSave, AcrError> {
    let bytes = fs::read(path).map_err(|source| AcrError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file_name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    parse_car_setups_bytes(&bytes, &file_name)
}

/// Parse the bytes of a `CarSetupsDataSaveSlot.sav`. `file_name` is only
/// used in error messages.
///
/// # Errors
///
/// See [`parse_car_setups`].
pub fn parse_car_setups_bytes(bytes: &[u8], file_name: &str) -> Result<CarSetupsSave, AcrError> {
    if !bytes.starts_with(b"GVAS") {
        return Err(AcrError::NotSetupSave {
            file_name: file_name.to_string(),
        });
    }
    let needle = fstring_bytes(PAYLOAD_TYPE);
    let Some(start) = find(bytes, &needle) else {
        return Err(AcrError::NotSetupSave {
            file_name: file_name.to_string(),
        });
    };

    let mut r = Reader {
        bytes,
        pos: start + needle.len(),
        file_name,
    };
    r.skip(8)?; // two opaque u32s
    let car_count = r.count(4)?;
    let mut cars = Vec::with_capacity(car_count);
    for _ in 0..car_count {
        let car_id = r.fstring()?;
        let setup_count = r.count(1)?;
        let mut setups = Vec::with_capacity(setup_count);
        for _ in 0..setup_count {
            setups.push(read_setup(&mut r)?);
        }
        cars.push(CarSetups { car_id, setups });
    }
    Ok(CarSetupsSave { cars })
}

fn read_setup(r: &mut Reader<'_>) -> Result<SavedSetup, AcrError> {
    r.skip(4 + 1 + 4)?; // opaque u32, 0xff marker, opaque u32
    let name = r.fstring()?;
    let author = r.fstring()?;
    let saved_ticks = r.i64()?;
    let game_version = r.fstring()?;
    let car_id = r.fstring()?;
    let stage = r.fstring()?;
    let surface = if r.u32()? == 0 {
        None
    } else {
        Some(r.fstring()?)
    };
    r.skip(8)?;
    let entry_count = r.count(8)?;
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        entries.push((r.fstring()?, r.fstring()?));
    }
    Ok(SavedSetup {
        name,
        author,
        saved_ticks,
        game_version,
        car_id,
        stage,
        surface,
        entries,
    })
}

/// An Unreal `FString` as it appears on disk: `i32` length (including the
/// NUL) followed by Latin-1 bytes.
fn fstring_bytes(s: &str) -> Vec<u8> {
    let len = i32::try_from(s.len() + 1).unwrap_or(i32::MAX);
    let mut out = len.to_le_bytes().to_vec();
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    file_name: &'a str,
}

impl Reader<'_> {
    fn truncated(&self) -> AcrError {
        AcrError::TruncatedSave {
            file_name: self.file_name.to_string(),
            offset: self.pos,
        }
    }

    fn take(&mut self, n: usize) -> Result<&[u8], AcrError> {
        let end = self.pos.checked_add(n).ok_or_else(|| self.truncated())?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| self.truncated())?;
        self.pos = end;
        Ok(slice)
    }

    fn skip(&mut self, n: usize) -> Result<(), AcrError> {
        self.take(n).map(|_| ())
    }

    fn u32(&mut self) -> Result<u32, AcrError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i64(&mut self) -> Result<i64, AcrError> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// A count of items that each occupy at least `min_item_bytes`. Rejects
    /// counts the remaining bytes could not possibly hold, so a corrupt
    /// count cannot trigger a huge allocation.
    fn count(&mut self, min_item_bytes: usize) -> Result<usize, AcrError> {
        let n = self.u32()? as usize;
        let remaining = self.bytes.len().saturating_sub(self.pos);
        if n.saturating_mul(min_item_bytes) > remaining {
            return Err(self.truncated());
        }
        Ok(n)
    }

    fn fstring(&mut self) -> Result<String, AcrError> {
        let b = self.take(4)?;
        let len = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        if len == 0 {
            return Ok(String::new());
        }
        if len > 0 {
            let raw = self.take(len as usize)?;
            let text = raw.strip_suffix(&[0]).unwrap_or(raw);
            return Ok(text.iter().map(|&c| char::from(c)).collect());
        }
        let units = len.unsigned_abs() as usize;
        let raw = self.take(units.checked_mul(2).ok_or_else(|| self.truncated())?)?;
        let utf16: Vec<u16> = (0..units)
            .map(|i| u16::from_le_bytes([raw[2 * i], raw[2 * i + 1]]))
            .collect();
        let text = utf16.strip_suffix(&[0]).unwrap_or(&utf16);
        Ok(String::from_utf16_lossy(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fstring(s: &str) -> Vec<u8> {
        fstring_bytes(s)
    }

    /// A minimal file with the same shape as a real one: a GVAS prefix with
    /// opaque filler, then the payload.
    fn build(cars: &[(&str, Vec<SavedSetup>)]) -> Vec<u8> {
        let mut out = b"GVAS".to_vec();
        out.extend_from_slice(&[0xAA; 37]); // stand-in for header + opaque bytes
        out.extend(fstring(PAYLOAD_TYPE));
        out.extend(0x70d5_u32.to_le_bytes());
        out.extend(1_u32.to_le_bytes());
        out.extend((cars.len() as u32).to_le_bytes());
        for (car, setups) in cars {
            out.extend(fstring(car));
            out.extend((setups.len() as u32).to_le_bytes());
            for s in setups {
                out.extend(2_u32.to_le_bytes());
                out.push(0xff);
                out.extend(1_u32.to_le_bytes());
                out.extend(fstring(&s.name));
                out.extend(if s.author.is_empty() {
                    0_i32.to_le_bytes().to_vec()
                } else {
                    fstring(&s.author)
                });
                out.extend(s.saved_ticks.to_le_bytes());
                out.extend(fstring(&s.game_version));
                out.extend(fstring(&s.car_id));
                out.extend(fstring(&s.stage));
                match &s.surface {
                    Some(surface) => {
                        out.extend(1_u32.to_le_bytes());
                        out.extend(fstring(surface));
                    }
                    None => out.extend(0_u32.to_le_bytes()),
                }
                out.extend([0; 8]);
                out.extend((s.entries.len() as u32).to_le_bytes());
                for (k, v) in &s.entries {
                    out.extend(fstring(k));
                    out.extend(fstring(v));
                }
            }
        }
        out
    }

    fn setup(name: &str, surface: Option<&str>) -> SavedSetup {
        SavedSetup {
            name: name.to_string(),
            author: "Driver".to_string(),
            saved_ticks: 639_000_000_000_000_000,
            game_version: "0.6.0.100866".to_string(),
            car_id: "SkodaFabiaRSRally2".to_string(),
            stage: "SwedenS1VargasenFullForward".to_string(),
            surface: surface.map(ToString::to_string),
            entries: vec![
                (
                    "Brakes.BrakesMain.FrontBias".to_string(),
                    "0.500000".to_string(),
                ),
                (
                    "Brakes.FrontLeft.PadCompound".to_string(),
                    "(Pads:Type00_Soft)".to_string(),
                ),
            ],
        }
    }

    #[test]
    fn round_trips_cars_setups_and_entries() {
        let a = setup("Gravel A", Some("Gravel"));
        let mut b = setup("Dry B", None);
        b.author.clear();
        let bytes = build(&[("SkodaFabiaRSRally2", vec![a.clone(), b.clone()])]);

        let save = parse_car_setups_bytes(&bytes, "t.sav").unwrap();
        assert_eq!(save.cars.len(), 1);
        assert_eq!(save.cars[0].car_id, "SkodaFabiaRSRally2");
        assert_eq!(save.cars[0].setups, vec![a, b]);
        assert_eq!(save.setups().count(), 2);
    }

    #[test]
    fn find_is_case_insensitive_and_can_filter_by_car() {
        let mut other = setup("Gravel A", None);
        other.car_id = "Peugeot206".to_string();
        let bytes = build(&[
            (
                "SkodaFabiaRSRally2",
                vec![setup("Gravel A", Some("Gravel"))],
            ),
            ("Peugeot206", vec![other]),
        ]);
        let save = parse_car_setups_bytes(&bytes, "t.sav").unwrap();
        assert_eq!(
            save.find("gravel a", Some("peugeot206")).unwrap().car_id,
            "Peugeot206"
        );
        assert_eq!(
            save.find("gravel a", None).unwrap().car_id,
            "SkodaFabiaRSRally2"
        );
        assert!(save.find("missing", None).is_none());
    }

    #[test]
    fn converts_dotnet_ticks_to_unix_seconds() {
        let s = setup("x", None);
        // 639000000000000000 ticks = 63_900_000_000 s since year 1.
        assert_eq!(s.saved_unix_secs(), 63_900_000_000 - 62_135_596_800);
        let epoch = SavedSetup {
            saved_ticks: UNIX_EPOCH_SECS_SINCE_YEAR_1 * 10_000_000,
            ..s
        };
        assert_eq!(epoch.saved_unix_secs(), 0);
    }

    #[test]
    fn utf16_strings_are_decoded() {
        let mut bytes = Vec::new();
        bytes.extend((-3_i32).to_le_bytes());
        for u in ['h' as u16, 'é' as u16, 0] {
            bytes.extend(u.to_le_bytes());
        }
        let mut r = Reader {
            bytes: &bytes,
            pos: 0,
            file_name: "t",
        };
        assert_eq!(r.fstring().unwrap(), "hé");
    }

    #[test]
    fn rejects_files_that_are_not_setup_saves() {
        assert!(matches!(
            parse_car_setups_bytes(b"not a save", "x.sav"),
            Err(AcrError::NotSetupSave { .. })
        ));
        // A GVAS file for something else (e.g. player data).
        assert!(matches!(
            parse_car_setups_bytes(b"GVAS\0\0\0\0PlayerSaveGameData", "p.sav"),
            Err(AcrError::NotSetupSave { .. })
        ));
    }

    #[test]
    fn truncation_is_an_error_not_a_panic() {
        let bytes = build(&[("SkodaFabiaRSRally2", vec![setup("A", None)])]);
        for cut in [bytes.len() - 1, bytes.len() - 30, bytes.len() / 2] {
            assert!(
                matches!(
                    parse_car_setups_bytes(&bytes[..cut], "t.sav"),
                    Err(AcrError::TruncatedSave { .. })
                ),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn absurd_counts_do_not_allocate() {
        let mut bytes = build(&[("SkodaFabiaRSRally2", vec![])]);
        // Overwrite the car count (right after the two opaque u32s).
        let at = find(&bytes, &fstring(PAYLOAD_TYPE)).unwrap() + fstring(PAYLOAD_TYPE).len() + 8;
        bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            parse_car_setups_bytes(&bytes, "t.sav"),
            Err(AcrError::TruncatedSave { .. })
        ));
    }
}
