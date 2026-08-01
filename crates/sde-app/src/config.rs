//! Tiny persisted app config: today, just the RBR/RSF install root path,
//! so it doesn't have to be re-picked (via "RBR install root..." in
//! `main.rs`) every time the app launches.
//!
//! Deliberately minimal — a single plain-text file holding one path, not a
//! structured config format — since there's exactly one setting to
//! persist. Read/write are pure functions over an explicit `config_dir`
//! (OS-specific resolution of *where* that directory is lives in
//! `main.rs`, which is the one place that already knows about the host
//! platform), so both are unit testable against a temp directory without
//! touching the user's real `%APPDATA%`.

use std::io;
use std::path::{Path, PathBuf};

const CONFIG_FILE_NAME: &str = "install_root.txt";

/// Read a previously saved install root from
/// `config_dir/install_root.txt`, if present.
///
/// Returns `None` (not an error) for any failure to read or an empty
/// file — a missing or corrupt config file just means "ask the user
/// again next time they click the button," not a hard failure the app
/// should surface.
#[must_use]
pub fn load_install_root(config_dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config_dir.join(CONFIG_FILE_NAME)).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// Persist `root` to `config_dir/install_root.txt`, creating `config_dir`
/// (and any missing parents) if needed.
///
/// # Errors
///
/// Returns [`std::io::Error`] if `config_dir` can't be created or the file
/// can't be written (e.g. a read-only filesystem). Callers treat this as
/// best-effort — a failure here just means the root won't be remembered
/// next launch, not that the just-picked root stops working this session.
pub fn save_install_root(config_dir: &Path, root: &Path) -> io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    std::fs::write(
        config_dir.join(CONFIG_FILE_NAME),
        root.to_string_lossy().as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sde-app-config-test-{label}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn round_trips_a_saved_root() {
        let dir = temp_dir("roundtrip");
        save_install_root(&dir, Path::new("C:\\Richard Burns Rally")).unwrap();

        assert_eq!(
            load_install_root(&dir),
            Some(PathBuf::from("C:\\Richard Burns Rally"))
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_config_file_is_none_not_an_error() {
        let dir = temp_dir("missing");
        assert_eq!(load_install_root(&dir), None);
    }

    #[test]
    fn empty_config_file_is_none() {
        let dir = temp_dir("empty");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CONFIG_FILE_NAME), "   \n").unwrap();

        assert_eq!(load_install_root(&dir), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saving_again_overwrites_the_previous_root() {
        let dir = temp_dir("overwrite");
        save_install_root(&dir, Path::new("C:\\First")).unwrap();
        save_install_root(&dir, Path::new("D:\\Second")).unwrap();

        assert_eq!(load_install_root(&dir), Some(PathBuf::from("D:\\Second")));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
