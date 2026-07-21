//! `binrw`-derived structs mirroring the fixed-record parts of the MoTeC
//! LD binary layout. Byte offsets are taken from `TrackDataAnalysis`'s
//! `data/motec.py` `_decode()` (the primary oracle per PROJECT_PLAN.md),
//! cross-checked against `gotzl/ldparser`'s `ldHead.fmt` / `ldChan.fmt`
//! struct format strings.
//!
//! The file header's fields of interest are scattered/sparse (lots of
//! unknown/reserved bytes in between), so each field seeks to its own
//! absolute offset rather than being modeled as one tightly-packed
//! struct. The per-channel meta record, by contrast, *is* a tightly
//! packed, repeating fixed-size record, so it's modeled as a normal
//! sequential `binrw` struct.

use std::io::SeekFrom;

use binrw::BinRead;

/// Raw (not-yet-decoded) file header fields, read directly at their
/// documented absolute byte offsets.
#[derive(BinRead, Debug)]
#[br(little)]
pub struct RawHeader {
    #[br(seek_before = SeekFrom::Start(0))]
    pub ld_marker: u32,

    #[br(seek_before = SeekFrom::Start(8))]
    pub channel_meta_addr: u32,

    #[br(seek_before = SeekFrom::Start(12))]
    pub _channel_data_addr: u32,

    #[br(seek_before = SeekFrom::Start(36))]
    pub event_addr: u32,

    #[br(seek_before = SeekFrom::Start(70))]
    pub device_serial: u32,

    #[br(seek_before = SeekFrom::Start(74))]
    pub device_type: [u8; 8],

    #[br(seek_before = SeekFrom::Start(82))]
    pub device_version_raw: u16,

    #[br(seek_before = SeekFrom::Start(86))]
    pub num_channels: u16,

    #[br(seek_before = SeekFrom::Start(94))]
    pub log_date: [u8; 16],

    #[br(seek_before = SeekFrom::Start(126))]
    pub log_time: [u8; 16],

    #[br(seek_before = SeekFrom::Start(158))]
    pub driver: [u8; 64],

    #[br(seek_before = SeekFrom::Start(222))]
    pub vehicle: [u8; 64],

    #[br(seek_before = SeekFrom::Start(350))]
    pub venue: [u8; 64],

    #[br(seek_before = SeekFrom::Start(1508))]
    pub session: [u8; 64],

    #[br(seek_before = SeekFrom::Start(1572))]
    pub short_comment: [u8; 64],
}

/// Raw channel meta record. Contiguous 84 bytes of real fields; the
/// record occupies 124 bytes total on disk (40 bytes of trailing
/// padding we don't need to model since the next record is always
/// reached by following `next_addr`, not by sequential position).
#[derive(BinRead, Debug)]
#[br(little)]
pub struct RawChannelRecord {
    pub _prev_addr: u32,
    pub next_addr: u32,
    pub data_addr: u32,
    pub data_count: u32,
    pub _counter: u16,
    pub elem_type: u16,
    pub elem_size: u16,
    pub sample_rate: u16,
    pub shift: i16,
    pub mul: i16,
    pub scale: i16,
    pub dec_pts: i16,
    pub name: [u8; 32],
    pub short_name: [u8; 8],
    pub unit: [u8; 12],
}
