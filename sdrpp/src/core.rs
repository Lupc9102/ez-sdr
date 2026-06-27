use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::config::ConfigManager;
use crate::module::ModuleManager;

/// Minimal command-line argument store.
/// Mirrors the C++ `CommandArgsParser` used in `core.cpp`.
#[derive(Default)]
pub struct CommandArgs {
    values: HashMap<String, ArgValue>,
}

#[derive(Default, Clone)]
pub enum ArgValue {
    #[default]
    None,
    Bool(bool),
    String(String),
    Float(f64),
}

impl CommandArgs {
    pub fn define_all(&mut self) {
        self.values.insert("server".to_string(), ArgValue::Bool(false));
        self.values.insert("root".to_string(), ArgValue::String(".".to_string()));
        self.values.insert("help".to_string(), ArgValue::Bool(false));
        #[cfg(target_os = "windows")]
        self.values.insert("con".to_string(), ArgValue::Bool(false));
    }

    pub fn parse(&mut self, _args: &[String]) -> i32 {
        // Production code would iterate args and populate `values`.
        0
    }

    pub fn bool(&self, key: &str) -> bool {
        matches!(self.values.get(key), Some(ArgValue::Bool(true)))
    }

    pub fn string(&self, key: &str) -> String {
        match self.values.get(key) {
            Some(ArgValue::String(s)) => s.clone(),
            _ => String::new(),
        }
    }
}

/// Central application state.
/// Translates the C++ `core` namespace globals (`configManager`, `moduleManager`, `args`)
/// and the `sdrpp_main` init/shutdown logic into a single `Core` struct.
pub struct Core {
    pub config: ConfigManager,
    pub modules: ModuleManager,
    pub args: CommandArgs,
    pub server_mode: bool,
    pub root_dir: PathBuf,
}

impl Core {
    pub fn new() -> Self {
        Self {
            config: ConfigManager::new(),
            modules: ModuleManager::new(),
            args: CommandArgs::default(),
            server_mode: false,
            root_dir: PathBuf::from("."),
        }
    }

    /// Full initialization sequence translated from `sdrpp_main`.
    /// Loads configuration, repairs missing keys, validates directories,
    /// and prepares subsystems (with placeholders for GUI/backend calls).
    pub fn init(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.args.define_all();
        // In a real binary: let cli: Vec<String> = std::env::args().collect();
        // self.args.parse(&cli[1..]);

        self.server_mode = self.args.bool("server");

        #[cfg(target_os = "macos")]
        {
            if let Ok(exec_path) = std::env::current_exe() {
                if let Some(parent) = exec_path.parent() {
                    let _ = std::env::set_current_dir(parent);
                }
            }
        }

        // Validate / create root directory
        let root = self.args.string("root");
        let root_path = Path::new(&root);
        if !root_path.exists() {
            eprintln!("Root directory {} does not exist, creating it", root);
            std::fs::create_dir_all(root_path)?;
        }
        if !root_path.is_dir() {
            return Err(format!("{} is not a directory", root).into());
        }
        self.root_dir = root_path.canonicalize().unwrap_or_else(|_| root_path.to_path_buf());

        // ======== DEFAULT CONFIG ========
        let mut def = json!({
            "bandColors": {
                "amateur": "#FF0000FF",
                "aviation": "#00FF00FF",
                "broadcast": "#0000FFFF",
                "marine": "#00FFFFFF",
                "military": "#FFFF00FF"
            },
            "bandPlan": "General",
            "bandPlanEnabled": true,
            "bandPlanPos": 0,
            "centerTuning": false,
            "colorMap": "Classic",
            "fftHold": false,
            "fftHoldSpeed": 60,
            "fftSmoothing": false,
            "fftSmoothingSpeed": 100,
            "snrSmoothing": false,
            "snrSmoothingSpeed": 20,
            "fastFFT": false,
            "fftHeight": 300,
            "fftRate": 20,
            "fftSize": 65536,
            "fftWindow": 2,
            "frequency": 100000000.0,
            "fullWaterfallUpdate": false,
            "max": 0.0,
            "maximized": false,
            "fullscreen": false,
            "menuElements": [
                {"name": "Source",            "open": true},
                {"name": "Radio",             "open": true},
                {"name": "Recorder",          "open": true},
                {"name": "Sinks",             "open": true},
                {"name": "Frequency Manager", "open": true},
                {"name": "VFO Color",         "open": true},
                {"name": "Band Plan",         "open": true},
                {"name": "Display",           "open": true}
            ],
            "menuWidth": 300,
            "min": -120.0,
            "theme": "Dark",
            "uiScale": if cfg!(target_os = "android") { 3.0 } else { 1.0 },
            "modules": [],
            "offsets": {
                "SpyVerter": 120000000.0,
                "Ham-It-Up": 125000000.0,
                "MMDS S-band (1998MHz)": -1998000000.0,
                "DK5AV X-Band": -6800000000.0,
                "Ku LNB (9750MHz)": -9750000000.0,
                "Ku LNB (10700MHz)": -10700000000.0
            },
            "selectedOffset": "None",
            "manualOffset": 0.0,
            "showMenu": true,
            "showWaterfall": true,
            "source": "",
            "decimation": 1,
            "iqCorrection": false,
            "invertIQ": false,
            "streams": {
                "Radio": { "muted": false, "sink": "Audio", "volume": 1.0 }
            },
            "windowSize": { "h": 720, "w": 1280 },
            "vfoOffsets": {},
            "vfoColors": { "Radio": "#FFFFFF" },
            "lockMenuOrder": cfg!(target_os = "android"),
        });

        let (mods_dir, res_dir): (String, String) = if cfg!(target_os = "windows") {
            ("./modules".into(), "./res".into())
        } else if cfg!(target_os = "macos") {
            ("../Plugins".into(), "../Resources".into())
        } else if cfg!(target_os = "android") {
            (
                format!("{}/modules", self.root_dir.display()),
                format!("{}/res", self.root_dir.display()),
            )
        } else {
            ("/usr/lib/sdrpp/plugins".into(), "/usr/share/sdrpp".into())
        };
        def["modulesDirectory"] = json!(mods_dir);
        def["resourcesDirectory"] = json!(res_dir);

        // Default module instances (mirroring original C++ block)
        let instances = json!({
            "Airspy Source":           { "module": "airspy_source",       "enabled": true },
            "AirspyHF+ Source":        { "module": "airspyhf_source",     "enabled": true },
            "Audio Source":            { "module": "audio_source",        "enabled": true },
            "BladeRF Source":          { "module": "bladerf_source",      "enabled": true },
            "Dragon Labs Source":      { "module": "dragonlabs_source",   "enabled": true },
            "File Source":             { "module": "file_source",         "enabled": true },
            "FobosSDR Source":         { "module": "fobossdr_source",     "enabled": true },
            "HackRF Source":           { "module": "hackrf_source",       "enabled": true },
            "Harogic Source":          { "module": "harogic_source",      "enabled": true },
            "Hermes Source":           { "module": "hermes_source",       "enabled": true },
            "HydraSDR Source":         { "module": "hydrasdr_source",     "enabled": true },
            "LimeSDR Source":          { "module": "limesdr_source",      "enabled": true },
            "Network Source":          { "module": "network_source",      "enabled": true },
            "PerseusSDR Source":       { "module": "perseus_source",      "enabled": true },
            "PlutoSDR Source":         { "module": "plutosdr_source",     "enabled": true },
            "RFNM Source":             { "module": "rfnm_source",         "enabled": true },
            "RFspace Source":          { "module": "rfspace_source",      "enabled": true },
            "RTL-SDR Source":          { "module": "rtl_sdr_source",      "enabled": true },
            "RTL-TCP Source":          { "module": "rtl_tcp_source",      "enabled": true },
            "SDRplay Source":          { "module": "sdrplay_source",      "enabled": true },
            "SDR++ Server Source":     { "module": "sdrpp_server_source", "enabled": true },
            "Spectran HTTP Source":    { "module": "spectran_http_source","enabled": true },
            "SpyServer Source":        { "module": "spyserver_source",    "enabled": true },
            "USRP Source":             { "module": "usrp_source",         "enabled": true },
            "Audio Sink":              "audio_sink",
            "Network Sink":            "network_sink",
            "Radio":                   "radio",
            "Frequency Manager":       "frequency_manager",
            "Recorder":                "recorder",
            "Rigctl Server":           "rigctl_server"
        });
        def["moduleInstances"] = instances;

        // Load config
        self.config.set_path(self.root_dir.join("config.json"));
        self.config.load(def.clone());
        self.config.enable_auto_save();

        // Repair and migration pass
        {
            let mut handle = self.config.acquire();

            #[cfg(target_os = "android")]
            {
                let android_mods = vec![
                    "airspy_source.so",
                    "airspyhf_source.so",
                    "hackrf_source.so",
                    "hermes_source.so",
                    "hydrasdr_source.so",
                    "plutosdr_source.so",
                    "rfspace_source.so",
                    "rtl_sdr_source.so",
                    "rtl_tcp_source.so",
                    "sdrpp_server_source.so",
                    "spyserver_source.so",
                    "network_sink.so",
                    "audio_sink.so",
                    "m17_decoder.so",
                    "meteor_demodulator.so",
                    "radio.so",
                    "frequency_manager.so",
                    "recorder.so",
                    "rigctl_server.so",
                    "scanner.so",
                ];
                handle.config_mut()["modules"] = json!(android_mods);
            }

            // Repair missing / remove unused keys
            let conf = handle.config_mut();
            if let (Some(def_obj), Some(conf_obj)) = (def.as_object(), conf.as_object_mut()) {
                for (key, value) in def_obj {
                    if !conf_obj.contains_key(key) {
                        eprintln!("Missing key in config {key}, repairing");
                        conf_obj.insert(key.clone(), value.clone());
                    }
                }
                let stale: Vec<String> = conf_obj
                    .keys()
                    .filter(|k| !def_obj.contains_key(*k))
                    .cloned()
                    .collect();
                for key in stale {
                    eprintln!("Unused key in config {key}, repairing");
                    conf_obj.remove(&key);
                }
            }

            // Migrate old moduleInstances format: string -> {module, enabled}
            if let Some(map) = conf.get_mut("moduleInstances").and_then(|v| v.as_object_mut()) {
                for (_name, inst) in map.iter_mut() {
                    if inst.is_string() {
                        let mod_name = inst.as_str().unwrap_or("").to_string();
                        *inst = json!({ "module": mod_name, "enabled": true });
                    }
                }
            }

            handle.release(true);
        }

        if self.server_mode {
            // Original: `return server::main();`
            return Ok(());
        }

        // Validate resource directory
        let res_dir = {
            let handle = self.config.acquire();
            let dir = handle.config()["resourcesDirectory"]
                .as_str()
                .unwrap_or(".")
                .to_string();
            handle.release(false);
            let p = Path::new(&dir).to_path_buf();
            p.canonicalize().unwrap_or(p)
        };

        if !res_dir.is_dir() {
            return Err(
                "Resource directory doesn't exist! Please make sure that you've configured it correctly in config.json"
                    .into(),
            );
        }

        // Following original order:
        // backend::init(&res_dir);
        // SmGui::init(false);
        // style::load_fonts(&res_dir);
        // icons::load(&res_dir);
        // bandplan::load_from_dir(res_dir.join("bandplans"));
        // gui::main_window.init();
        // backend::render_loop();

        eprintln!("Core initialized.");
        Ok(())
    }

    /// Graceful shutdown sequence translated from the tail of `sdrpp_main`.
    /// Stops modules, tears down backends, disables auto-save, and persists config.
    pub fn shutdown(&mut self) {
        self.modules.shutdown_all();

        // Original: backend::end();
        // Original: sigpath::iqFrontEnd.stop();

        self.config.disable_auto_save();
        self.config.save();

        eprintln!("Core shut down.");
    }

    /// Forward samplerate changes to the DSP chain.
    /// Mirrors C++ `core::setInputSampleRate`.
    pub fn set_input_sample_rate(&self, sample_rate: f64) {
        if self.server_mode {
            // server::set_input_sample_rate(sample_rate);
            return;
        }
        // sigpath::iq_frontend.set_sample_rate(sample_rate);
        // let effective = sigpath::iq_frontend.effective_samplerate();
        // gui::waterfall.set_bandwidth(effective);
        eprintln!("New DSP samplerate: {sample_rate}");
    }
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}
