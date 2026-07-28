//! `binrw`-derived structs for the fixed, sequential parts of the `.ibt`
//! binary layout: the 144-byte file header (including the trailing
//! `irsdk_diskSubHeader`) and the 144-byte-per-record variable-header
//! array. The strided sample buffer that follows isn't a single
//! contiguous struct (its record width and per-variable byte offset are
//! only known once the header/var-headers are read), so that part is
//! decoded by hand in `lib.rs` instead.
//!
//! Byte offsets are documented in full in `PROJECT_PLAN.md`'s "IBT
//! (iRacing) format findings" section, cross-checked against a real
//! `.ibt` capture before this was written.

use binrw::binrw;

/// One of the four `varBuf` slots in the file header. Only slot 0 is ever
/// used (matching `TrackDataAnalysis/data/iracing.py`'s `_decode`, which
/// reads exactly one buffer and treats a file with more as unsupported).
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy)]
pub struct RawVarBuf {
    pub tick_count: i32,
    pub buf_offset: i32,
    pub _pad: [i32; 2],
}

/// Fixed 144-byte header at the start of the file: `irsdk_header` followed
/// immediately by `irsdk_diskSubHeader`.
#[binrw]
#[brw(little)]
#[derive(Debug, Clone)]
pub struct RawHeader {
    pub ver: i32,
    pub status: i32,
    pub tick_rate: i32,
    pub session_info_update: i32,
    pub session_info_len: i32,
    pub session_info_offset: i32,
    pub num_vars: i32,
    pub var_header_offset: i32,
    pub num_buf: i32,
    pub buf_len: i32,
    pub _pad1: [i32; 2],
    pub var_buf: [RawVarBuf; 4],
    // irsdk_diskSubHeader
    pub session_start_date: u32,
    pub _pad2: u32,
    pub session_start_time: f64,
    pub session_end_time: f64,
    pub session_lap_count: i32,
    pub session_record_count: i32,
}

/// One 144-byte variable-header record, `num_vars` of which start at
/// `RawHeader::var_header_offset`.
#[binrw]
#[brw(little)]
#[derive(Debug, Clone)]
pub struct RawVarHeader {
    /// Index into the type table: `0`=char, `1`=bool, `2`=int32,
    /// `3`=bitfield(u32), `4`=float32, `5`=float64.
    pub var_type: i32,
    /// Byte offset of this variable within each sample record.
    pub offset: i32,
    /// Array length for vector variables. Ignored when decoding, matching
    /// the oracle (see `PROJECT_PLAN.md`) — only element 0 is read.
    pub count: i32,
    pub count_as_time: u8,
    pub _pad: [u8; 3],
    pub name: [u8; 32],
    pub desc: [u8; 64],
    pub unit: [u8; 32],
}
