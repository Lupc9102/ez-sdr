//! Writable Windows application data and migration from older portable installs.
//!
//! The native Windows entry point selects this directory before constructing any
//! panels. Relative configuration, bookmark, cache and default recording paths
//! therefore all share the same per-user home, regardless of shortcut location.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static PROBE_ID: AtomicU64 = AtomicU64::new(0);
const LEGACY_FILES: [&str; 3] = [
    "ez_sdr_config.json",
    "ez_sdr_bookmarks.json",
    "signal_history.json",
];

/// Prepare a per-user directory, copying missing legacy preferences only.
/// Legacy files and recordings remain in place; current per-user files win.
pub fn prepare_user_directory(local_app_data: &Path, legacy: &[PathBuf]) -> io::Result<PathBuf> {
    if !local_app_data.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LOCALAPPDATA must name an absolute directory",
        ));
    }
    let directory = local_app_data.join("EZ-SDR");
    fs::create_dir_all(&directory)?;
    let probe = directory.join(format!(
        ".write-check-{}-{}",
        std::process::id(),
        PROBE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)?;
    let result = file.write_all(b"EZ-SDR storage check");
    drop(file);
    // Remove only the probe created by this call, including a failed write.
    let _ = fs::remove_file(&probe);
    result?;

    for name in LEGACY_FILES {
        let destination = directory.join(name);
        if destination.try_exists()? {
            continue;
        }
        for origin in legacy {
            let bytes = match fs::read(origin.join(name)) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let bytes = if name == "ez_sdr_config.json" {
                migrate_config_paths(bytes, origin)
            } else {
                bytes
            };
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
            {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
                        drop(file);
                        let _ = fs::remove_file(&destination);
                        return Err(error);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            break;
        }
    }
    Ok(directory)
}

fn migrate_config_paths(bytes: Vec<u8>, origin: &Path) -> Vec<u8> {
    let Ok(mut config) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        // Preserve malformed legacy data too; config loading retains its normal
        // fallback behavior, and the original can still be recovered manually.
        return bytes;
    };
    if let Some(path) = config.get_mut("output_directory") {
        if !matches!(path.as_str(), Some("./recordings" | "recordings")) {
            rebase_path(path, origin);
        }
    }
    if let Some(path) = config
        .get_mut("source_preferences")
        .and_then(|preferences| preferences.get_mut("replay_file"))
    {
        rebase_path(path, origin);
    }
    serde_json::to_vec_pretty(&config).unwrap_or(bytes)
}

fn rebase_path(value: &mut serde_json::Value, origin: &Path) {
    if let Some(path) = value.as_str().filter(|path| !path.is_empty()) {
        if Path::new(path).is_relative() {
            *value = origin.join(path).to_string_lossy().into_owned().into();
        }
    }
}

/// Resolve the current Windows user's storage, then redirect relative app paths.
#[cfg(windows)]
pub fn initialize_windows_directory() -> io::Result<()> {
    let base = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not available"))?;
    let mut legacy = vec![std::env::current_dir()?];
    if let Some(parent) = std::env::current_exe()?.parent() {
        if !legacy.iter().any(|path| path == parent) {
            legacy.push(parent.to_path_buf());
        }
    }
    let directory = prepare_user_directory(Path::new(&base), &legacy)?;
    std::env::set_current_dir(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "ez-sdr-app-data-{}-{}",
                std::process::id(),
                PROBE_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn migration_preserves_preferences_and_replay_locations_without_touching_legacy() {
        let root = Folder::new();
        let legacy = root.0.join("installed-bin");
        fs::create_dir_all(&legacy).unwrap();
        let original = br#"{"theme":"light","output_directory":"./recordings","source_preferences":{"replay_file":"captures/pass.cf32"},"future_setting":42}"#;
        fs::write(legacy.join(LEGACY_FILES[0]), original).unwrap();
        fs::write(legacy.join(LEGACY_FILES[1]), b"[{\"label\":\"FM\"}]").unwrap();
        fs::write(legacy.join(LEGACY_FILES[2]), b"[]").unwrap();
        let user = prepare_user_directory(&root.0.join("user-data"), std::slice::from_ref(&legacy))
            .unwrap();
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(user.join(LEGACY_FILES[0])).unwrap()).unwrap();
        assert_eq!(migrated["theme"], "light");
        assert_eq!(migrated["future_setting"], 42);
        assert_eq!(migrated["output_directory"], "./recordings");
        assert_eq!(
            migrated["source_preferences"]["replay_file"],
            legacy.join("captures/pass.cf32").to_string_lossy().as_ref()
        );
        assert_eq!(fs::read(legacy.join(LEGACY_FILES[0])).unwrap(), original);
        assert_eq!(
            fs::read(user.join(LEGACY_FILES[1])).unwrap(),
            fs::read(legacy.join(LEGACY_FILES[1])).unwrap()
        );
        fs::create_dir(user.join("recordings")).unwrap();
        fs::write(user.join("recordings/sample.iq"), b"IQ").unwrap();
        assert!(!legacy.join("recordings").exists());
        assert!(fs::read_dir(user).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".write-check")));
    }

    #[test]
    fn newer_user_data_wins_and_custom_relative_output_keeps_its_location() {
        let root = Folder::new();
        let legacy = root.0.join("portable");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join(LEGACY_FILES[0]),
            br#"{"output_directory":"custom-captures"}"#,
        )
        .unwrap();
        let base = root.0.join("local");
        let user = prepare_user_directory(&base, std::slice::from_ref(&legacy)).unwrap();
        let config: serde_json::Value =
            serde_json::from_slice(&fs::read(user.join(LEGACY_FILES[0])).unwrap()).unwrap();
        assert_eq!(
            config["output_directory"],
            legacy.join("custom-captures").to_string_lossy().as_ref()
        );
        fs::write(user.join(LEGACY_FILES[0]), b"new user settings").unwrap();
        fs::write(user.join(LEGACY_FILES[1]), b"new user bookmarks").unwrap();
        fs::write(legacy.join(LEGACY_FILES[1]), b"old bookmarks").unwrap();
        prepare_user_directory(&base, &[legacy]).unwrap();
        assert_eq!(
            fs::read(user.join(LEGACY_FILES[0])).unwrap(),
            b"new user settings"
        );
        assert_eq!(
            fs::read(user.join(LEGACY_FILES[1])).unwrap(),
            b"new user bookmarks"
        );
    }

    #[test]
    fn invalid_or_unwritable_storage_fails_instead_of_using_install_directory() {
        assert!(prepare_user_directory(Path::new("relative"), &[]).is_err());
        let root = Folder::new();
        fs::write(root.0.join("EZ-SDR"), b"blocked").unwrap();
        assert!(prepare_user_directory(&root.0, &[]).is_err());
        assert_eq!(fs::read(root.0.join("EZ-SDR")).unwrap(), b"blocked");
    }
}
