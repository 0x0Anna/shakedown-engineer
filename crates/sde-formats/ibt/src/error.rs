use std::path::PathBuf;

/// Errors that can occur while parsing an iRacing `.ibt` file.
#[derive(Debug, thiserror::Error)]
pub enum IbtError {
    #[error("failed to read {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse binary structure: {0}")]
    Binrw(#[from] binrw::Error),

    #[error(
        "file declares {num_buf} sample buffers; only single-buffer .ibt files are supported"
    )]
    UnsupportedBufferCount { num_buf: i32 },

    #[error("variable {name:?} has unknown type {var_type} (expected 0..=5)")]
    UnknownVarType { name: String, var_type: i32 },

    #[error("variable header {index} at offset {offset} runs past end of file (size {file_len})")]
    TruncatedVarHeader {
        index: i32,
        offset: usize,
        file_len: usize,
    },

    #[error("sample buffer (offset {offset}, len {len}) runs past end of file (size {file_len})")]
    TruncatedSampleBuffer {
        offset: usize,
        len: usize,
        file_len: usize,
    },

    #[error("session info block (offset {offset}, len {len}) runs past end of file (size {file_len})")]
    TruncatedSessionInfo {
        offset: usize,
        len: usize,
        file_len: usize,
    },

    #[error("failed to parse session info YAML: {0}")]
    SessionInfoYaml(#[from] serde_yaml::Error),
}
