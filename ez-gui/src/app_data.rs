//! Platform-specific writable paths and migration from portable installs.
//!
//! Resolve paths once at startup and pass them to consumers. The process working
//! directory is never changed, so launch shortcuts and read-only installs do not
//! redirect persistent data or bundled resources.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static PROBE_ID: AtomicU64 = AtomicU64::new(0);
const CONFIG_FILE: &str = "ez_sdr_config.json";
const BOOKMARKS_FILE: &str = "ez_sdr_bookmarks.json";
const HISTORY_FILE: &str = "signal_history.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

/// Environment inputs are explicit so path selection can be tested without
/// mutating process-global environment variables.
#[derive(Debug, Clone, Default)]
pub struct PathEnvironment {
    pub home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub xdg_cache_home: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDataPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// The working directory captured before any app initialization.
    pub launch_dir: PathBuf,
    /// Base used by existing relative custom paths before migration.
    pub relative_settings_base: PathBuf,
}

impl AppDataPaths {
    pub fn resolve(
        platform: Platform,
        environment: &PathEnvironment,
        launch_dir: &Path,
    ) -> io::Result<Self> {
        require_absolute(launch_dir, "launch directory")?;
        let (config_dir, data_dir, cache_dir, relative_settings_base) = match platform {
            Platform::Windows => {
                let root =
                    absolute_env_path(environment.local_app_data.as_deref(), "LOCALAPPDATA")?
                        .join("EZ-SDR");
                (root.clone(), root.clone(), root.clone(), root)
            }
            Platform::MacOs => {
                let home = absolute_env_path(environment.home.as_deref(), "HOME")?;
                let support = home.join("Library/Application Support/EZ-SDR");
                let cache = home.join("Library/Caches/EZ-SDR");
                (support.clone(), support, cache, launch_dir.to_path_buf())
            }
            Platform::Linux => {
                let home = absolute_env_path(environment.home.as_deref(), "HOME")?;
                let config =
                    valid_xdg_or_home(environment.xdg_config_home.as_deref(), &home, ".config")
                        .join("ez-sdr");
                let data =
                    valid_xdg_or_home(environment.xdg_data_home.as_deref(), &home, ".local/share")
                        .join("ez-sdr");
                let cache =
                    valid_xdg_or_home(environment.xdg_cache_home.as_deref(), &home, ".cache")
                        .join("ez-sdr");
                (config, data, cache, launch_dir.to_path_buf())
            }
        };
        Ok(Self {
            config_dir,
            data_dir,
            cache_dir,
            launch_dir: launch_dir.to_path_buf(),
            relative_settings_base,
        })
    }

    pub fn from_process(launch_dir: &Path) -> io::Result<Self> {
        let platform = if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        };
        let environment = PathEnvironment {
            home: std::env::var_os("HOME").map(PathBuf::from),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            xdg_data_home: std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            xdg_cache_home: std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
            local_app_data: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        };
        Self::resolve(platform, &environment, launch_dir)
    }

    /// Resolve platform paths, prepare them for writes, then copy missing legacy
    /// preferences from the captured launch or executable directory.
    pub fn prepare(&self, legacy: &[PathBuf]) -> io::Result<()> {
        let mut roots = vec![&self.config_dir, &self.data_dir, &self.cache_dir];
        roots.sort();
        roots.dedup();
        for root in roots {
            prepare_writable_directory(root)?;
        }
        self.migrate_legacy(legacy)
    }

    fn migrate_legacy(&self, legacy: &[PathBuf]) -> io::Result<()> {
        migrate_file(
            &self.config_dir.join(CONFIG_FILE),
            CONFIG_FILE,
            legacy,
            true,
        )?;
        migrate_file(
            &self.data_dir.join(BOOKMARKS_FILE),
            BOOKMARKS_FILE,
            legacy,
            false,
        )?;
        migrate_file(
            &self.data_dir.join(HISTORY_FILE),
            HISTORY_FILE,
            legacy,
            false,
        )
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE)
    }

    pub fn bookmarks_file(&self) -> PathBuf {
        self.data_dir.join(BOOKMARKS_FILE)
    }

    pub fn history_file(&self) -> PathBuf {
        self.data_dir.join(HISTORY_FILE)
    }

    pub fn recordings_dir(&self) -> PathBuf {
        self.data_dir.join("recordings")
    }

    pub fn tile_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("tile_cache")
    }

    /// Anchor old relative settings at the launch location before passing them
    /// to consumers whose persistent files now live elsewhere.
    pub fn normalize_config_paths(&self, config: &mut crate::config::AppConfig) {
        if matches!(
            config.output_directory.as_str(),
            "" | "./recordings" | "recordings"
        ) {
            config.output_directory = self.recordings_dir().to_string_lossy().into_owned();
        } else {
            config.output_directory =
                absolutize_saved_path(&config.output_directory, &self.relative_settings_base);
        }
        if let Some(path) = config
            .source_preferences
            .replay_file
            .as_mut()
            .filter(|path| !path.is_empty())
        {
            *path = absolutize_saved_path(path, &self.relative_settings_base);
        }
    }

    /// Compatibility paths for unit tests and library callers that construct
    /// panels directly. The desktop entry point always passes prepared paths.
    pub fn current_directory_compatibility() -> Self {
        let launch_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            config_dir: launch_dir.clone(),
            data_dir: launch_dir.clone(),
            cache_dir: launch_dir.clone(),
            relative_settings_base: launch_dir.clone(),
            launch_dir,
        }
    }
}

pub fn legacy_directories(launch_dir: &Path, executable_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut directories = vec![launch_dir.to_path_buf()];
    if let Some(directory) = executable_dir {
        if !directories.iter().any(|existing| existing == directory) {
            directories.push(directory.to_path_buf());
        }
    }
    directories
}

fn absolute_env_path(path: Option<&Path>, variable: &str) -> io::Result<PathBuf> {
    let path = path.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{variable} is not available"),
        )
    })?;
    require_absolute(path, variable)?;
    Ok(path.to_path_buf())
}

fn require_absolute(path: &Path, label: &str) -> io::Result<()> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} must name an absolute directory"),
        ))
    }
}

fn valid_xdg_or_home<'a>(xdg: Option<&'a Path>, home: &'a Path, fallback: &str) -> PathBuf {
    xdg.filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(fallback))
}

fn prepare_writable_directory(directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
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
    let _ = fs::remove_file(probe);
    result
}

fn migrate_file(
    destination: &Path,
    name: &str,
    legacy: &[PathBuf],
    is_config: bool,
) -> io::Result<()> {
    if destination.try_exists()? {
        return Ok(());
    }
    for origin in legacy {
        let bytes = match fs::read(origin.join(name)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let bytes = if is_config {
            migrate_config_paths(bytes, origin)
        } else {
            bytes
        };
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(destination) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
                    drop(file);
                    let _ = fs::remove_file(destination);
                    return Err(error);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
            Err(error) => return Err(error),
        }
        break;
    }
    Ok(())
}

fn migrate_config_paths(bytes: Vec<u8>, origin: &Path) -> Vec<u8> {
    let Ok(mut config) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        // Keep malformed legacy data available for manual recovery.
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

fn absolutize_saved_path(path: &str, origin: &Path) -> String {
    let path = Path::new(path);
    if path.is_relative() {
        origin.join(path).to_string_lossy().into_owned()
    } else {
        path.to_string_lossy().into_owned()
    }
}

/// Kept for callers that still need the historical Windows directory helper.
/// New desktop startup uses [`AppDataPaths`] and does not change process cwd.
pub fn prepare_user_directory(local_app_data: &Path, legacy: &[PathBuf]) -> io::Result<PathBuf> {
    require_absolute(local_app_data, "LOCALAPPDATA")?;
    let directory = local_app_data.join("EZ-SDR");
    prepare_writable_directory(&directory)?;
    for name in [CONFIG_FILE, BOOKMARKS_FILE, HISTORY_FILE] {
        migrate_file(&directory.join(name), name, legacy, name == CONFIG_FILE)?;
    }
    Ok(directory)
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
    fn resolves_xdg_paths_and_ignores_relative_xdg_roots() {
        let env = PathEnvironment {
            home: Some(PathBuf::from("/home/tester")),
            xdg_config_home: Some(PathBuf::from("relative/config")),
            xdg_data_home: Some(PathBuf::from("/mnt/data")),
            xdg_cache_home: Some(PathBuf::from("relative/cache")),
            ..Default::default()
        };
        let paths = AppDataPaths::resolve(Platform::Linux, &env, Path::new("/opt/ez-sdr")).unwrap();
        assert_eq!(
            paths.config_dir,
            PathBuf::from("/home/tester/.config/ez-sdr")
        );
        assert_eq!(paths.data_dir, PathBuf::from("/mnt/data/ez-sdr"));
        assert_eq!(paths.cache_dir, PathBuf::from("/home/tester/.cache/ez-sdr"));
        assert_eq!(paths.launch_dir, PathBuf::from("/opt/ez-sdr"));
        let another_launch =
            AppDataPaths::resolve(Platform::Linux, &env, Path::new("/tmp/shortcut-launch"))
                .unwrap();
        assert_eq!(paths.config_dir, another_launch.config_dir);
        assert_eq!(paths.data_dir, another_launch.data_dir);
        assert_eq!(paths.cache_dir, another_launch.cache_dir);
    }

    #[test]
    fn resolves_macos_and_existing_windows_roots() {
        let env = PathEnvironment {
            home: Some(PathBuf::from("/Users/tester")),
            local_app_data: Some(PathBuf::from("/Users/tester/AppData/Local")),
            ..Default::default()
        };
        let mac =
            AppDataPaths::resolve(Platform::MacOs, &env, Path::new("/Applications/EZ-SDR.app"))
                .unwrap();
        assert_eq!(
            mac.config_dir,
            PathBuf::from("/Users/tester/Library/Application Support/EZ-SDR")
        );
        assert_eq!(mac.data_dir, mac.config_dir);
        assert_eq!(
            mac.cache_dir,
            PathBuf::from("/Users/tester/Library/Caches/EZ-SDR")
        );
        let windows = AppDataPaths::resolve(Platform::Windows, &env, Path::new("/EZ-SDR")).unwrap();
        let root = PathBuf::from("/Users/tester/AppData/Local/EZ-SDR");
        assert_eq!(windows.config_dir, root);
        assert_eq!(windows.data_dir, root);
        assert_eq!(windows.cache_dir, root);
        assert_eq!(windows.relative_settings_base, root);
    }

    #[test]
    fn invalid_home_and_launch_roots_are_rejected() {
        let env = PathEnvironment {
            home: Some(PathBuf::from("relative-home")),
            ..Default::default()
        };
        assert!(AppDataPaths::resolve(Platform::Linux, &env, Path::new("/launch")).is_err());
        let env = PathEnvironment {
            home: Some(PathBuf::from("/home/tester")),
            ..Default::default()
        };
        assert!(AppDataPaths::resolve(Platform::Linux, &env, Path::new("relative")).is_err());
    }

    #[test]
    fn normalizes_default_recordings_and_custom_paths_from_launch_directory() {
        let paths = AppDataPaths {
            config_dir: PathBuf::from("/home/tester/.config/ez-sdr"),
            data_dir: PathBuf::from("/home/tester/.local/share/ez-sdr"),
            cache_dir: PathBuf::from("/home/tester/.cache/ez-sdr"),
            launch_dir: PathBuf::from("/opt/portable"),
            relative_settings_base: PathBuf::from("/opt/portable"),
        };
        let mut config = crate::config::AppConfig::default();
        config.source_preferences.replay_file = Some("captures/pass.cf32".into());
        paths.normalize_config_paths(&mut config);
        assert_eq!(
            config.output_directory,
            "/home/tester/.local/share/ez-sdr/recordings"
        );
        assert_eq!(
            config.source_preferences.replay_file.as_deref(),
            Some("/opt/portable/captures/pass.cf32")
        );
        config.output_directory = "custom-captures".into();
        paths.normalize_config_paths(&mut config);
        assert_eq!(config.output_directory, "/opt/portable/custom-captures");
    }

    #[test]
    fn migration_is_repeatable_non_overwriting_and_keeps_relative_meaning() {
        let root = Folder::new();
        let launch = root.0.join("launch");
        let home = root.0.join("home");
        fs::create_dir_all(&launch).unwrap();
        fs::create_dir_all(&home).unwrap();
        let original = br#"{"output_directory":"custom-captures","source_preferences":{"replay_file":"captures/pass.cf32"}}"#;
        fs::write(launch.join(CONFIG_FILE), original).unwrap();
        fs::write(launch.join(BOOKMARKS_FILE), b"[\"portable\"]").unwrap();
        let env = PathEnvironment {
            home: Some(home.clone()),
            ..Default::default()
        };
        let paths = AppDataPaths::resolve(Platform::Linux, &env, &launch).unwrap();
        let legacy = legacy_directories(&launch, Some(&launch.join("bin")));
        paths.prepare(&legacy).unwrap();
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(paths.config_file()).unwrap()).unwrap();
        assert_eq!(
            migrated["output_directory"],
            launch.join("custom-captures").to_string_lossy().as_ref()
        );
        assert_eq!(
            migrated["source_preferences"]["replay_file"],
            launch.join("captures/pass.cf32").to_string_lossy().as_ref()
        );
        assert_eq!(fs::read(launch.join(CONFIG_FILE)).unwrap(), original);
        fs::write(paths.config_file(), b"new user settings").unwrap();
        paths.prepare(&legacy).unwrap();
        assert_eq!(fs::read(paths.config_file()).unwrap(), b"new user settings");
    }

    #[test]
    fn legacy_windows_helper_preserves_user_precedence_and_old_files() {
        let root = Folder::new();
        let legacy = root.0.join("portable");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(
            legacy.join(CONFIG_FILE),
            br#"{"theme":"light","output_directory":"./recordings","source_preferences":{"replay_file":"captures/pass.cf32"},"future_setting":42}"#,
        )
        .unwrap();
        fs::write(legacy.join(BOOKMARKS_FILE), b"[\"FM\"]").unwrap();
        let user =
            prepare_user_directory(&root.0.join("local"), std::slice::from_ref(&legacy)).unwrap();
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(user.join(CONFIG_FILE)).unwrap()).unwrap();
        assert_eq!(migrated["theme"], "light");
        assert_eq!(migrated["future_setting"], 42);
        assert_eq!(migrated["output_directory"], "./recordings");
        assert_eq!(
            migrated["source_preferences"]["replay_file"],
            legacy.join("captures/pass.cf32").to_string_lossy().as_ref()
        );
        fs::write(user.join(BOOKMARKS_FILE), b"new user bookmarks").unwrap();
        fs::write(legacy.join(BOOKMARKS_FILE), b"old bookmarks").unwrap();
        prepare_user_directory(&root.0.join("local"), std::slice::from_ref(&legacy)).unwrap();
        assert_eq!(
            fs::read(user.join(BOOKMARKS_FILE)).unwrap(),
            b"new user bookmarks"
        );
        assert!(fs::read(legacy.join(CONFIG_FILE)).is_ok());
    }

    #[test]
    fn invalid_or_unwritable_storage_fails_without_falling_back_to_launch_directory() {
        assert!(prepare_user_directory(Path::new("relative"), &[]).is_err());
        let root = Folder::new();
        fs::write(root.0.join("blocked"), b"blocked").unwrap();
        let paths = AppDataPaths {
            config_dir: root.0.join("blocked"),
            data_dir: root.0.join("data"),
            cache_dir: root.0.join("cache"),
            launch_dir: root.0.clone(),
            relative_settings_base: root.0.clone(),
        };
        assert!(paths.prepare(&[]).is_err());
        assert!(fs::read_dir(&root.0).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".write-check")));
    }
}
