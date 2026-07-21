use std::path::PathBuf;

/// Errors that can occur while parsing a MoTeC `.ld` file.
#[derive(Debug, thiserror::Error)]
pub enum LdError {
    #[error("failed to read {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse binary structure: {0}")]
    Binrw(#[from] binrw::Error),

    #[error("not a MoTeC LD file (bad magic marker: expected 0x40, got {0:#x})")]
    BadMagic(u32),

    #[error("channel {name:?} has unknown element type {elem_type} (expected 0, 3, 5 for integer or 7 for float)")]
    UnknownElemType { name: String, elem_type: u16 },

    #[error(
        "channel {name:?} has unsupported element size {elem_size} for element type {elem_type}"
    )]
    UnsupportedElemSize {
        name: String,
        elem_type: u16,
        elem_size: u16,
    },

    #[error("channel {name:?} sample data at offset {offset} (len {len}) runs past end of file (size {file_len})")]
    TruncatedSampleData {
        name: String,
        offset: u32,
        len: usize,
        file_len: usize,
    },

    #[error("unexpected end of data while reading {what} at offset {offset}")]
    Truncated { what: &'static str, offset: usize },
}

/// Errors that can occur while parsing a MoTeC-style `.ldx` sidecar file
/// (the XML companion some exporters — e.g. Assetto Corsa Competizione —
/// write alongside a `.ld` file, carrying lap/marker data that isn't
/// present in the `.ld`'s own channel data).
#[derive(Debug, thiserror::Error)]
pub enum LdxError {
    #[error("failed to read {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse XML: {0}")]
    Xml(#[from] roxmltree::Error),

    #[error("marker {index} has an unparseable Time attribute {value:?}")]
    BadMarkerTime { index: usize, value: String },
}
