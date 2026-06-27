use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

struct ConfigData {
    path: PathBuf,
    conf: Value,
}

/// Thread-safe JSON configuration manager with background auto-save.
/// Translates C++ `ConfigManager` into idiomatic Rust.
pub struct ConfigManager {
    data: Arc<Mutex<ConfigData>>,
    changed: Arc<AtomicBool>,
    auto_save_enabled: Arc<AtomicBool>,
    term: Arc<(Mutex<bool>, Condvar)>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

/// RAII guard returned by `ConfigManager::acquire`.
/// Calling `release(modified)` mirrors C++ `ConfigManager::release(bool)`.
pub struct ConfigHandle<'a> {
    guard: std::sync::MutexGuard<'a, ConfigData>,
    changed: Arc<AtomicBool>,
    modified: bool,
}

impl<'a> ConfigHandle<'a> {
    pub fn config(&self) -> &Value {
        &self.guard.conf
    }

    pub fn config_mut(&mut self) -> &mut Value {
        &mut self.guard.conf
    }

    pub fn release(mut self, modified: bool) {
        self.modified = modified;
        // Moving `self` causes `Drop` to fire, committing the changed flag.
    }
}

impl<'a> Drop for ConfigHandle<'a> {
    fn drop(&mut self) {
        if self.modified {
            self.changed.store(true, Ordering::Relaxed);
        }
    }
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(ConfigData {
                path: PathBuf::new(),
                conf: Value::Null,
            })),
            changed: Arc::new(AtomicBool::new(false)),
            auto_save_enabled: Arc::new(AtomicBool::new(false)),
            term: Arc::new((Mutex::new(false), Condvar::new())),
            handle: Mutex::new(None),
        }
    }

    pub fn set_path<P: AsRef<Path>>(&self, file: P) {
        let mut data = self.data.lock().unwrap();
        let path = file.as_ref();
        data.path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        };
    }

    /// Load config from disk. If the file is missing or corrupt, write `default` instead.
    pub fn load(&self, default: Value) {
        let mut data = self.data.lock().unwrap();
        if data.path.as_os_str().is_empty() {
            eprintln!("Config manager tried to load file with no path specified");
            return;
        }
        if !data.path.exists() {
            eprintln!("Config file '{}' does not exist, creating it", data.path.display());
            data.conf = default;
            Self::save_data(&data);
            return;
        }
        if !data.path.is_file() {
            eprintln!("Config file '{}' isn't a file", data.path.display());
            return;
        }

        match fs::read_to_string(&data.path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(val) => data.conf = val,
                Err(e) => {
                    eprintln!(
                        "Config file '{}' is corrupted, resetting it: {}",
                        data.path.display(),
                        e
                    );
                    data.conf = default;
                    Self::save_data(&data);
                }
            },
            Err(e) => {
                eprintln!(
                    "Config file '{}' is corrupted, resetting it: {}",
                    data.path.display(),
                    e
                );
                data.conf = default;
                Self::save_data(&data);
            }
        }
    }

    /// Synchronous save under the internal mutex.
    pub fn save(&self) {
        let data = self.data.lock().unwrap();
        Self::save_data(&data);
    }

    fn save_data(data: &ConfigData) {
        if data.path.as_os_str().is_empty() {
            return;
        }
        if let Some(parent) = data.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&data.conf) {
            let _ = fs::write(&data.path, text);
        }
    }

    /// Spawn a background thread that saves the config every second when `changed` is set.
    pub fn enable_auto_save(&self) {
        if self.auto_save_enabled.swap(true, Ordering::Relaxed) {
            return;
        }
        let data = Arc::clone(&self.data);
        let changed = Arc::clone(&self.changed);
        let auto_save_enabled = Arc::clone(&self.auto_save_enabled);
        let term = Arc::clone(&self.term);

        let handle = thread::spawn(move || {
            let (term_lock, term_cvar) = &*term;
            loop {
                if !auto_save_enabled.load(Ordering::Relaxed) {
                    break;
                }

                if changed.swap(false, Ordering::Relaxed) {
                    if let Ok(locked_data) = data.try_lock() {
                        Self::save_data(&locked_data);
                    } else {
                        eprintln!("ConfigManager locked, waiting...");
                        changed.store(true, Ordering::Relaxed);
                    }
                }

                let exit = term_lock.lock().unwrap();
                if *exit {
                    break;
                }
                let _ = term_cvar.wait_timeout(exit, Duration::from_secs(1));
            }
        });

        *self.handle.lock().unwrap() = Some(handle);
    }

    /// Signal the auto-save worker to stop and join it.
    pub fn disable_auto_save(&self) {
        if !self.auto_save_enabled.swap(false, Ordering::Relaxed) {
            return;
        }
        {
            let (term_lock, _) = &*self.term;
            *term_lock.lock().unwrap() = true;
        }
        let (term_lock, term_cvar) = &*self.term;
        term_cvar.notify_one();
        if let Some(handle) = self.handle.lock().unwrap().take() {
            let _ = handle.join();
        }
        *term_lock.lock().unwrap() = false;
    }

    /// Lock the config and return a guard. Call `release(modified)` when done.
    pub fn acquire(&self) -> ConfigHandle<'_> {
        let guard = self.data.lock().unwrap();
        ConfigHandle {
            guard,
            changed: Arc::clone(&self.changed),
            modified: false,
        }
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ConfigManager {
    fn drop(&mut self) {
        self.disable_auto_save();
    }
}
