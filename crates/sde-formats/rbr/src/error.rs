use std::path::PathBuf;

/// Errors that can occur while reading Richard Burns Rally / RSF
/// companion files.
///
/// Deliberately narrow: the text formats this crate reads are written by
/// several different plugin versions, so unknown or malformed *fields* are
/// reported as absent values rather than as errors. Only failing to read
/// the file at all is an error.
#[derive(Debug, thiserror::Error)]
pub enum RbrError {
    #[error("failed to read {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
