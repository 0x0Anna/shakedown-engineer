//! Parser for MoTeC `.ld` binary telemetry log files.
//!
//! This crate is deliberately UI-free and dependency-light (per
//! PROJECT_PLAN.md's modularity principles) so it can be reused outside
//! the `tda-app` GUI — as a CLI tool, or in another project entirely.
//!
//! Field names on [`LdFile`] / [`LdChannel`] intentionally mirror
//! `TrackDataAnalysis`'s `data/base.py` `LogFile`/`Channel` dataclasses,
//! since `tda-core` builds its `Session` model directly on top of this
//! crate's output.
//!
//! The binary layout and the physical-value conversion formula
//! (`value = raw * mul / (scale * 10^dec_pts) + offset`) are documented
//! in `PROJECT_PLAN.md`'s "Validation findings" section; that formula is
//! `TrackDataAnalysis`'s (not `ldparser`'s — the two differ whenever
//! `shift`/`offset != 0`).

// clippy::pedantic/nursery notes (not part of the default lint set the
// project otherwise keeps clean), applying crate-wide:
// - doc_markdown fires repeatedly on plain-English mentions of MoTeC,
//   `PROJECT_PLAN.md`, TDA, etc. across this crate's doc comments;
//   backtick-wrapping every proper noun isn't worth the churn.
// - too_long_first_doc_paragraph fires on doc comments that deliberately
//   front-load full context in one paragraph rather than splitting off a
//   terse one-line summary — that's this crate's chosen doc style.
#![allow(clippy::doc_markdown, clippy::too_long_first_doc_paragraph)]

mod bytes;
mod error;
mod ldx;
mod raw;

use std::fs;
use std::io::Cursor;
use std::path::Path;

use binrw::BinReaderExt;

pub use error::{LdError, LdxError};
pub use ldx::{parse_ldx, parse_ldx_str, LdxFile};
use raw::{RawChannelRecord, RawHeader};

/// A single telemetry channel decoded from the log file.
#[derive(Debug, Clone, PartialEq)]
pub struct LdChannel {
    /// Full channel name, e.g. `"Ground Speed"`.
    pub name: String,
    /// MoTeC's separate abbreviated name field (rarely used downstream).
    pub short_name: String,
    /// Physical unit string, e.g. `"km/h"`, `"m"`, `""`.
    pub unit: String,
    /// Logging rate in Hz.
    pub sample_rate: u16,
    /// Decimal places, as recorded in the file (clamped to >= 0, matching
    /// TDA's `max(dec_pts, 0)` — the raw field can be negative, meaning
    /// "round to a coarser-than-integer precision", which isn't a valid
    /// display precision).
    pub dec_pts: i16,
    /// `false` means "hold the previous value until the next timecode"
    /// rather than interpolate — TDA's heuristic: channels with unit `s`
    /// or no unit at all are treated as discrete/event channels.
    ///
    /// That heuristic assumes real MoTeC hardware/software populates unit
    /// strings for genuine analog channels. Some game-exported `.ld` files
    /// (observed from Assetto Corsa Competizione) leave *every* channel's
    /// unit blank, which would otherwise mark all of them (including
    /// clearly continuous ones like speed/RPM/G-force) as non-interpolating.
    /// See `decode_channels`' file-level fallback for the correction applied
    /// in that case.
    pub interpolate: bool,
    /// Sample timecodes in milliseconds since the start of the log,
    /// i.e. `i * (1000 / sample_rate)` for sample index `i`.
    pub timecodes: Vec<f64>,
    /// Physical (converted) sample values, one per timecode.
    pub values: Vec<f64>,
}

impl LdChannel {
    #[must_use]
    pub const fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Header/event/venue/vehicle metadata, as key/value strings — mirrors
/// TDA's `LogFile.metadata: Dict[str, str]` in shape, but is exposed as
/// a struct here since Rust field access is nicer than stringly-typed
/// dict keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LdMetadata {
    pub device_serial: u32,
    pub device_type: String,
    /// Formatted like TDA's `"%.2f" % (raw / 100)`, e.g. `"4.20"`.
    pub device_version: String,
    pub log_date: String,
    pub log_time: String,
    pub driver: String,
    pub vehicle: String,
    pub venue: String,
    pub session: String,
    pub short_comment: String,

    // Populated only if the file has an event sub-record (event_addr != 0).
    pub event_name: Option<String>,
    pub event_session: Option<String>,
    pub long_comment: Option<String>,

    // Populated only if the event sub-record points at a venue record.
    pub venue_name: Option<String>,

    // Populated only if the venue sub-record points at a vehicle record.
    pub vehicle_id: Option<String>,
    pub vehicle_desc: Option<String>,
    pub vehicle_weight: Option<u32>,
    pub vehicle_type: Option<String>,
    pub vehicle_comment: Option<String>,
    pub diff_ratio: Option<f64>,
    /// Gear ratios for gears 1..=9, in order, skipping any that were zero
    /// in the file (matching TDA's `_set_if`, which only records nonzero
    /// values).
    pub gear_ratios: Vec<(u8, f64)>,
    pub vehicle_wheelbase_mm: Option<u16>,
}

/// A fully parsed MoTeC `.ld` file.
#[derive(Debug, Clone, PartialEq)]
pub struct LdFile {
    pub metadata: LdMetadata,
    /// Channels in the order they appear in the file's linked list.
    pub channels: Vec<LdChannel>,
    pub file_name: String,
}

impl LdFile {
    /// Look up a channel by name (first match), analogous to TDA's
    /// `LogFile.channels[name]` dict access.
    #[must_use]
    pub fn channel(&self, name: &str) -> Option<&LdChannel> {
        self.channels.iter().find(|c| c.name == name)
    }
}

/// Parse a MoTeC `.ld` file from disk.
///
/// # Errors
///
/// Returns [`LdError::Io`] if the file can't be read, or any of the
/// parse-related [`LdError`] variants ([`LdError::BadMagic`],
/// [`LdError::Binrw`], [`LdError::Truncated`],
/// [`LdError::TruncatedSampleData`], [`LdError::UnknownElemType`],
/// [`LdError::UnsupportedElemSize`]) if `data` isn't a well-formed MoTeC
/// `.ld` file.
pub fn parse(path: &Path) -> Result<LdFile, LdError> {
    let data = fs::read(path).map_err(|source| LdError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    parse_bytes(&data, file_name)
}

/// Parse a MoTeC `.ld` file already loaded into memory.
///
/// # Errors
///
/// See [`parse`]'s `# Errors` section — the same failure modes apply
/// here, minus [`LdError::Io`] (there's no filesystem access on this
/// path).
pub fn parse_bytes(data: &[u8], file_name: impl Into<String>) -> Result<LdFile, LdError> {
    let mut cursor = Cursor::new(data);
    let raw_header: RawHeader = cursor.read_le()?;

    if raw_header.ld_marker != 0x40 {
        return Err(LdError::BadMagic(raw_header.ld_marker));
    }

    let metadata = decode_metadata(data, &raw_header)?;
    let channels = decode_channels(data, raw_header.channel_meta_addr, raw_header.num_channels)?;

    Ok(LdFile {
        metadata,
        channels,
        file_name: file_name.into(),
    })
}

fn decode_metadata(data: &[u8], header: &RawHeader) -> Result<LdMetadata, LdError> {
    use bytes::{ascii_at, decode_ascii, u16_at, u32_at};

    let mut metadata = LdMetadata {
        device_serial: header.device_serial,
        device_type: decode_ascii(&header.device_type),
        device_version: format!("{:.2}", f64::from(header.device_version_raw) / 100.0),
        log_date: decode_ascii(&header.log_date),
        log_time: decode_ascii(&header.log_time),
        driver: decode_ascii(&header.driver),
        vehicle: decode_ascii(&header.vehicle),
        venue: decode_ascii(&header.venue),
        session: decode_ascii(&header.session),
        short_comment: decode_ascii(&header.short_comment),
        ..Default::default()
    };

    let event_addr = header.event_addr as usize;
    if event_addr != 0 {
        metadata.event_name = Some(ascii_at(data, event_addr, 64));
        metadata.event_session = Some(ascii_at(data, event_addr + 64, 64));
        metadata.long_comment = Some(ascii_at(data, event_addr + 128, 1024));

        let venue_addr = u32_at(data, event_addr + 1152)? as usize;
        if venue_addr != 0 {
            metadata.venue_name = Some(ascii_at(data, venue_addr, 64));

            let vehicle_addr = u32_at(data, venue_addr + 1098)? as usize;
            if vehicle_addr != 0 {
                metadata.vehicle_id = Some(ascii_at(data, vehicle_addr, 64));
                metadata.vehicle_desc = Some(ascii_at(data, vehicle_addr + 64, 64));

                let weight = u32_at(data, vehicle_addr + 192)?;
                if weight != 0 {
                    metadata.vehicle_weight = Some(weight);
                }
                let vtype = ascii_at(data, vehicle_addr + 196, 32);
                if !vtype.is_empty() {
                    metadata.vehicle_type = Some(vtype);
                }
                let vcomment = ascii_at(data, vehicle_addr + 228, 32);
                if !vcomment.is_empty() {
                    metadata.vehicle_comment = Some(vcomment);
                }

                let diff_raw = u16_at(data, vehicle_addr + 260)?;
                if diff_raw != 0 {
                    metadata.diff_ratio = Some(f64::from(diff_raw) / 1000.0);
                }

                for gear in 1u8..=9 {
                    let gear_raw = u16_at(data, vehicle_addr + 260 + gear as usize * 2)?;
                    if gear_raw != 0 {
                        metadata
                            .gear_ratios
                            .push((gear, f64::from(gear_raw) / 1000.0));
                    }
                }

                let wheelbase = u16_at(data, vehicle_addr + 284)?;
                if wheelbase != 0 {
                    metadata.vehicle_wheelbase_mm = Some(wheelbase);
                }
            }
        }
    }

    Ok(metadata)
}

fn decode_channels(
    data: &[u8],
    channel_meta_addr: u32,
    num_channels: u16,
) -> Result<Vec<LdChannel>, LdError> {
    let mut channels = Vec::with_capacity(num_channels as usize);
    let mut addr = channel_meta_addr;

    for _ in 0..num_channels {
        if addr == 0 {
            // Linked list ended early; trust num_channels loosely and
            // just stop rather than erroring, matching real-world files
            // that might not perfectly agree.
            break;
        }

        let slice = data.get(addr as usize..).ok_or(LdError::Truncated {
            what: "channel meta record",
            offset: addr as usize,
        })?;
        let mut cursor = Cursor::new(slice);
        let record: RawChannelRecord = cursor.read_le()?;

        channels.push(decode_channel(data, &record)?);
        addr = record.next_addr;
    }

    // Some game-exported .ld files (observed from Assetto Corsa Competizione)
    // never populate the unit field for any channel, which would otherwise
    // leave every channel — including obviously continuous ones like speed,
    // RPM, and G-force — marked non-interpolating under TDA's unit-based
    // heuristic. Only apply the correction when the *whole file* lacks
    // units (a strong signal the exporter just doesn't record them), so
    // genuine MoTeC hardware/software files keep their original behavior.
    if !channels.is_empty() && channels.iter().all(|c| c.unit.is_empty()) {
        for channel in &mut channels {
            channel.interpolate = !is_discrete_channel_name(&channel.name);
        }
    }

    Ok(channels)
}

/// Name-based fallback for channels that are clearly discrete/event
/// signals (gear number, beacon/lap markers, ABS/TC intervention flags)
/// even though they carry no unit to say so. Matched against `_`-split
/// tokens (case-insensitive) so e.g. `"LAP_BEACON"` matches via its
/// `"BEACON"` token without also matching unrelated channels that merely
/// contain the substring.
fn is_discrete_channel_name(name: &str) -> bool {
    const DISCRETE_TOKENS: &[&str] = &["GEAR", "BEACON", "ABS", "TC", "DRS", "FLAG"];
    name.split('_')
        .any(|tok| DISCRETE_TOKENS.contains(&tok.to_ascii_uppercase().as_str()))
}

fn decode_channel(data: &[u8], record: &RawChannelRecord) -> Result<LdChannel, LdError> {
    let name = bytes::decode_ascii(&record.name);
    let short_name = bytes::decode_ascii(&record.short_name);
    let unit = bytes::decode_ascii(&record.unit);

    let sample_rate = record.sample_rate.max(1);
    let data_count = record.data_count as usize;
    let elem_size = record.elem_size as usize;
    let byte_len = data_count * elem_size;

    let raw_bytes = data
        .get(record.data_addr as usize..record.data_addr as usize + byte_len)
        .ok_or_else(|| LdError::TruncatedSampleData {
            name: name.clone(),
            offset: record.data_addr,
            len: byte_len,
            file_len: data.len(),
        })?;

    // TDA's formula (see PROJECT_PLAN.md "Conversion formula discrepancy"):
    //   value = raw * mul / (scale * 10^dec_pts) + offset
    // NOT ldparser's `(raw / scale * 10^-dec + shift) * mul` — the two
    // diverge whenever shift/offset != 0.
    let mul = f64::from(record.mul);
    let scale = f64::from(record.scale);
    let dec_pts = f64::from(record.dec_pts);
    let offset = f64::from(record.shift);
    let denom = scale * 10f64.powf(dec_pts);
    let convert = |raw: f64| raw * mul / denom + offset;

    // Single pass from raw bytes straight to converted physical values —
    // avoids materializing an intermediate `Vec<f64>` of raw samples
    // before mapping it into the final `values` Vec, which matters for
    // logs with hundreds of channels at tens of thousands of samples
    // each (halves peak transient allocation per channel during decode).
    let values: Vec<f64> = match (record.elem_type, record.elem_size) {
        (0 | 3 | 5, 2) => raw_bytes
            .chunks_exact(2)
            .map(|c| convert(f64::from(i16::from_le_bytes([c[0], c[1]]))))
            .collect(),
        (0 | 3 | 5, 4) => raw_bytes
            .chunks_exact(4)
            .map(|c| convert(f64::from(i32::from_le_bytes([c[0], c[1], c[2], c[3]]))))
            .collect(),
        (7, 4) => raw_bytes
            .chunks_exact(4)
            .map(|c| convert(f64::from(f32::from_le_bytes([c[0], c[1], c[2], c[3]]))))
            .collect(),
        (0 | 3 | 5 | 7, size) => {
            return Err(LdError::UnsupportedElemSize {
                name,
                elem_type: record.elem_type,
                elem_size: size,
            })
        }
        (other, _) => {
            return Err(LdError::UnknownElemType {
                name,
                elem_type: other,
            })
        }
    };

    // `i as f64`: `data_count` (sample count per channel) is realistically
    // in the tens of thousands, nowhere near f64's 2^52 exact-integer
    // range, so this cast never actually loses precision.
    #[allow(clippy::cast_precision_loss)]
    let timecodes: Vec<f64> = (0..data_count)
        .map(|i| i as f64 * (1000.0 / f64::from(sample_rate)))
        .collect();

    // Best-guess heuristic ported from TDA: `interpolate = units not in ('s', '')`.
    let interpolate = unit != "s" && !unit.is_empty();

    Ok(LdChannel {
        name,
        short_name,
        unit,
        sample_rate: record.sample_rate,
        dec_pts: record.dec_pts.max(0),
        interpolate,
        timecodes,
        values,
    })
}

#[cfg(test)]
mod error_path_tests {
    //! Deliberately-malformed byte buffers exercising each `LdError`
    //! variant, complementing the happy-path fixture-driven test in
    //! `tests/integration.rs`. These don't need real `.ld` files — just
    //! enough of the header/channel-record layout (see `raw.rs`'s
    //! documented offsets) to steer the parser into each error branch.
    use super::*;

    /// Header is 1636 bytes total (`short_comment` starts at 1572 and is
    /// 64 bytes long — the last field `raw::RawHeader` seeks to).
    const HEADER_LEN: u32 = 1636;

    fn header_bytes(ld_marker: u32, num_channels: u16, channel_meta_addr: u32) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_LEN as usize];
        buf[0..4].copy_from_slice(&ld_marker.to_le_bytes());
        buf[8..12].copy_from_slice(&channel_meta_addr.to_le_bytes());
        buf[86..88].copy_from_slice(&num_channels.to_le_bytes());
        buf
    }

    /// 84-byte channel meta record (the "contiguous real fields" prefix
    /// documented on `raw::RawChannelRecord`; the 40 bytes of trailing
    /// padding aren't needed since we never rely on sequential layout).
    #[allow(clippy::too_many_arguments)]
    fn channel_record_bytes(
        next_addr: u32,
        data_addr: u32,
        data_count: u32,
        elem_type: u16,
        elem_size: u16,
        sample_rate: u16,
        name: &str,
    ) -> Vec<u8> {
        let mut buf = vec![0u8; 84];
        buf[0..4].copy_from_slice(&0u32.to_le_bytes()); // prev_addr, unused
        buf[4..8].copy_from_slice(&next_addr.to_le_bytes());
        buf[8..12].copy_from_slice(&data_addr.to_le_bytes());
        buf[12..16].copy_from_slice(&data_count.to_le_bytes());
        buf[18..20].copy_from_slice(&elem_type.to_le_bytes());
        buf[20..22].copy_from_slice(&elem_size.to_le_bytes());
        buf[22..24].copy_from_slice(&sample_rate.to_le_bytes());
        let name_bytes = name.as_bytes();
        let n = name_bytes.len().min(32);
        buf[32..32 + n].copy_from_slice(&name_bytes[..n]);
        buf
    }

    #[test]
    fn bad_magic_is_rejected() {
        let data = header_bytes(0xff, 0, 0);
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(matches!(err, LdError::BadMagic(0xff)), "got {err:?}");
    }

    #[test]
    fn truncated_header_is_a_binrw_error_not_a_panic() {
        // Buffer far too short to hold the full header — must error
        // gracefully, not panic on an out-of-bounds seek/read.
        let data = vec![0x40, 0, 0, 0];
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(matches!(err, LdError::Binrw(_)), "got {err:?}");
    }

    #[test]
    fn channel_meta_addr_out_of_bounds_is_a_graceful_error_not_a_panic() {
        // channel_meta_addr points far past the end of the buffer —
        // regression test for a bug where `decode_channels` indexed
        // `&data[addr..]` directly and panicked on malformed input
        // instead of returning `LdError::Truncated`.
        let data = header_bytes(0x40, 1, 999_999);
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(matches!(err, LdError::Truncated { .. }), "got {err:?}");
    }

    #[test]
    fn unknown_elem_type_is_rejected() {
        let mut data = header_bytes(0x40, 1, HEADER_LEN);
        data.extend_from_slice(&channel_record_bytes(0, 0, 0, 99, 2, 1, "Weird"));
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(
            matches!(err, LdError::UnknownElemType { elem_type: 99, .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn unsupported_elem_size_is_rejected() {
        // elem_type 7 (float) only supports elem_size 4; 8 is bogus.
        let mut data = header_bytes(0x40, 1, HEADER_LEN);
        data.extend_from_slice(&channel_record_bytes(0, 0, 0, 7, 8, 1, "Weird"));
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(
            matches!(err, LdError::UnsupportedElemSize { elem_size: 8, .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn truncated_sample_data_is_rejected() {
        let record_addr = HEADER_LEN;
        let mut data = header_bytes(0x40, 1, record_addr);
        // data_addr points past the end of the buffer we're about to
        // build, with data_count=10 float32 samples (40 bytes) that
        // don't actually exist.
        let bogus_data_addr = record_addr + 84;
        data.extend_from_slice(&channel_record_bytes(
            0,
            bogus_data_addr,
            10,
            7,
            4,
            1,
            "Ground Speed",
        ));
        let err = parse_bytes(&data, "x.ld").unwrap_err();
        assert!(
            matches!(err, LdError::TruncatedSampleData { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn io_error_on_missing_file() {
        let err = parse(std::path::Path::new("/definitely/does/not/exist/nope.ld")).unwrap_err();
        assert!(matches!(err, LdError::Io { .. }), "got {err:?}");
    }
}
