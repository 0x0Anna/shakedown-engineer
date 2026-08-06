use std::path::PathBuf;

/// Errors that can occur while parsing an `acr_telemetry` `acr_export`
/// MoTeC-style CSV file.
#[derive(Debug, thiserror::Error)]
pub enum AcrError {
    #[error("failed to read {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{file_name:?} is empty")]
    EmptyFile { file_name: String },

    /// The first row's first two quoted fields aren't `"Format"`,`"MoTeC
    /// CSV File"` — this is `acr_export`'s (and its pyxrk-dev-derived
    /// header convention's) fixed signature, so a file failing this check
    /// almost certainly isn't an `acr_export` CSV at all (a different
    /// tool's export, or a non-telemetry `.csv` picked by mistake) rather
    /// than a malformed one.
    #[error(
        "{file_name:?} doesn't look like an acr_export CSV (expected a \"Format\",\"MoTeC CSV File\" first row)"
    )]
    NotAcrCsv { file_name: String },

    /// No row whose first field is exactly `"Time"` was found before EOF
    /// — that row is the channel-names header `acr_export` always writes
    /// right after its metadata preamble (see `motec_csv.rs`'s
    /// `header_rows`), so its absence means the file is truncated or
    /// otherwise not a real export.
    #[error("{file_name:?} has no channel-names row (expected a row starting with \"Time\")")]
    MissingTimeColumn { file_name: String },

    /// The channel-names row was the last line in the file — there's no
    /// units row (and therefore no data) to read.
    #[error("{file_name:?} has a channel-names row but no units row after it")]
    MissingUnitsRow { file_name: String },

    #[error("{file_name:?} column {column:?}: {value:?} is not a valid number")]
    MalformedNumber {
        file_name: String,
        column: String,
        value: String,
    },
}
