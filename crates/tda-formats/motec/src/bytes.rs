//! Small helpers for reading fixed-offset fields directly out of the
//! in-memory file buffer.
//!
//! The MoTeC LD "event -> venue -> vehicle" metadata sub-records form a
//! sparse, pointer-chased chain (see `TrackDataAnalysis/data/motec.py`
//! `_decode()`), which doesn't map cleanly onto a single contiguous
//! `binrw` struct. Reading them by explicit offset (mirroring TDA's own
//! approach) is simpler and just as robust.

use crate::error::LdError;

pub fn u16_at(data: &[u8], offset: usize) -> Result<u16, LdError> {
    let bytes = data.get(offset..offset + 2).ok_or(LdError::Truncated {
        what: "u16",
        offset,
    })?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

pub fn u32_at(data: &[u8], offset: usize) -> Result<u32, LdError> {
    let bytes = data.get(offset..offset + 4).ok_or(LdError::Truncated {
        what: "u32",
        offset,
    })?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Decode an ASCII, NUL-padded string of `len` bytes starting at `offset`.
/// Returns an empty string if the region is out of bounds (some optional
/// trailing sub-records may not be present in shorter/older files).
pub fn ascii_at(data: &[u8], offset: usize, len: usize) -> String {
    data.get(offset..offset + len)
        .map_or_else(String::new, decode_ascii)
}

/// Decode a NUL-terminated (or NUL-padded) ASCII byte slice into a
/// trimmed `String`, matching TDA's `_dec_str`.
pub fn decode_ascii(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}
