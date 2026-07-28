use std::path::PathBuf;

/// Errors that can occur while parsing a `shtep`-exported `.tsv` +
/// `.meta.json` sidecar pair.
#[derive(Debug, thiserror::Error)]
pub enum ShtepError {
    #[error("failed to read {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The `.tsv`'s matching `{base}.meta.json` sidecar doesn't exist.
    /// Per `SCHEMA.md`, a converter should only ever look at a `.tsv`
    /// once its sidecar has landed — the plugin writes the sidecar
    /// first, so a lone `.tsv` with no sidecar means the recording is
    /// still in progress (or was never renamed out of `TempDir`), not a
    /// malformed file.
    #[error("missing sidecar {path:?} — the .tsv's matching .meta.json wasn't found")]
    MissingSidecar { path: PathBuf },

    #[error("failed to parse sidecar JSON: {0}")]
    MalformedSidecar(#[from] serde_json::Error),

    /// Refused per `SCHEMA.md`'s compatibility policy: only reject when
    /// the file's `schemaVersion` is *newer* than what this parser
    /// understands, never merely for missing/extra fields.
    #[error(
        "sidecar declares schemaVersion {found}, newer than the {supported} this parser understands"
    )]
    UnsupportedSchemaVersion { found: u32, supported: u32 },

    #[error("{file_name:?} has no header row")]
    EmptyFile { file_name: String },

    #[error("{file_name:?}'s header row must start with \"Time_s\" (found {found:?})")]
    MissingTimeColumn { file_name: String, found: String },

    #[error(
        "{file_name:?} line {line}: expected {expected_columns} columns (per the header), found {found_columns}"
    )]
    MalformedRow {
        file_name: String,
        line: usize,
        expected_columns: usize,
        found_columns: usize,
    },

    #[error("{file_name:?} line {line}, column {column:?}: {value:?} is not a valid number")]
    MalformedNumber {
        file_name: String,
        line: usize,
        column: String,
        value: String,
    },
}
