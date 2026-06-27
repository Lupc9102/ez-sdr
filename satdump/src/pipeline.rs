//! Processing pipeline — idiomatic Rust rewrite of SatDump's `src-core/pipeline/`.
//!
//! Translated logic from:
//! - `pipeline.h / pipeline.cpp`        – JSON config loading & parsing
//! - `module.h / module.cpp`            – module registry & plugin loading
//! - `pipeline_run.cpp`                 – file-based pipeline runner
//! - `live_pipeline.h / live_pipeline.cpp` – live streaming pipeline runner

use anyhow::{anyhow, bail, Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use num_complex::Complex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

// =============================================================================
// Errors
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("module not found: {0}")]
    ModuleNotFound(String),
    #[error("module '{module}' does not support {io} type {dtype}")]
    UnsupportedDataType {
        module: String,
        io: &'static str,
        dtype: DataType,
    },
    #[error("invalid pipeline step: index {0} out of bounds")]
    InvalidStep(usize),
    #[error("pipeline '{0}' does not support {1} mode")]
    UnsupportedLiveMode(String, String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// =============================================================================
// Data types flowing between modules
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    /// On-disk file.
    File,
    /// Generic byte stream (FIFO).
    Stream,
    /// Complex-f32 DSP sample stream.
    DspStream,
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataType::File => write!(f, "File"),
            DataType::Stream => write!(f, "Stream"),
            DataType::DspStream => write!(f, "DspStream"),
        }
    }
}

// =============================================================================
// Streaming primitives (replacing dsp::RingBuffer / dsp::stream)
// =============================================================================

/// Thread-safe bounded byte FIFO with an explicit activity flag.
pub struct ByteFifo {
    pub tx: Sender<u8>,
    pub rx: Receiver<u8>,
    pub active: Arc<AtomicBool>,
}

impl ByteFifo {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = bounded(capacity);
        Self {
            tx,
            rx,
            active: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Handle to a bounded DSP sample stream.
#[derive(Clone)]
pub struct DspStreamHandle {
    pub tx: Sender<Complex<f32>>,
    pub rx: Receiver<Complex<f32>>,
    pub active: Arc<AtomicBool>,
}

impl DspStreamHandle {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = bounded(capacity);
        Self {
            tx,
            rx,
            active: Arc::new(AtomicBool::new(false)),
        }
    }
}

// =============================================================================
// Module trait & registry
// =============================================================================

pub type ModuleStats = Value;

/// Core trait for every pipeline module.
///
/// Mirrors the C++ `ProcessingModule` base class. All methods that mutate the
/// module require `&mut self` so the runner can wrap the object in a `Mutex`
/// when it needs to spawn it on a worker thread.
pub trait ProcessingModule: Send + 'static {
    fn id(&self) -> &str;

    fn input_types(&self) -> Vec<DataType>;
    fn output_types(&self) -> Vec<DataType>;

    fn set_input_type(&mut self, dtype: DataType) -> Result<(), PipelineError>;
    fn set_output_type(&mut self, dtype: DataType) -> Result<(), PipelineError>;

    fn input_type(&self) -> DataType;
    fn output_type(&self) -> DataType;

    fn init(&mut self) -> Result<(), PipelineError>;
    fn process(&mut self) -> Result<(), PipelineError>;
    fn stop(&mut self) -> Result<(), PipelineError>;

    fn output_file(&self) -> Option<&Path>;

    fn stats(&self) -> ModuleStats {
        Value::Null
    }
    fn draw_ui(&mut self, _window: bool) {}

    // --- plumbing injected by the runner ---
    fn set_input_fifo(&mut self, _fifo: Option<Arc<ByteFifo>>) {}
    fn set_output_fifo(&mut self, _fifo: Option<Arc<ByteFifo>>) {}
    fn set_input_dsp_stream(&mut self, _stream: Option<DspStreamHandle>) {}
    fn set_output_dsp_stream(&mut self, _stream: Option<DspStreamHandle>) {}

    fn input_active(&self) -> bool {
        false
    }
    fn set_input_active(&mut self, _active: bool) {}
}

/// Factory for creating a named module instance.
pub type ModuleFactory = Arc<
    dyn Fn(&Path, &Path, &Value) -> Result<Box<dyn ProcessingModule>, PipelineError> + Send + Sync,
>;

pub struct ModuleEntry {
    pub id: String,
    pub default_params: Value,
    pub factory: ModuleFactory,
}

/// Central registry of all known module types.
pub struct ModuleRegistry {
    modules: Mutex<Vec<ModuleEntry>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self {
            modules: Mutex::new(Vec::new()),
        }
    }

    pub fn register(&self, entry: ModuleEntry) {
        self.modules.lock().unwrap().push(entry);
    }

    pub fn exists(&self, id: &str) -> bool {
        self.modules.lock().unwrap().iter().any(|e| e.id == id)
    }

    pub fn instantiate(
        &self,
        id: &str,
        input_file: &Path,
        output_hint: &Path,
        params: &Value,
    ) -> Result<Box<dyn ProcessingModule>, PipelineError> {
        let mods = self.modules.lock().unwrap();
        for entry in mods.iter() {
            if entry.id == id {
                return (entry.factory)(input_file, output_hint, params);
            }
        }
        Err(PipelineError::ModuleNotFound(id.to_string()))
    }

    /// Load dynamic plugin libraries from a directory. Each library must export
    /// `extern "C" fn register_modules(registry: &ModuleRegistry)`.
    #[cfg(feature = "plugins")]
    pub fn load_plugins_from_dir(&self, dir: &Path) -> Result<()> {
        use libloading::{Library, Symbol};
        if !dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some(std::env::consts::DLL_EXTENSION) {
                println!("Loading plugin {}", path.display());
                unsafe {
                    let lib = Library::new(&path)?;
                    let register: Symbol<fn(&ModuleRegistry)> =
                        lib.get(b"register_modules\0")?;
                    register(self);
                    std::mem::forget(lib); // keep symbols alive
                }
            }
        }
        Ok(())
    }
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_REGISTRY: OnceLock<ModuleRegistry> = OnceLock::new();

pub fn global_registry() -> &'static ModuleRegistry {
    GLOBAL_REGISTRY.get_or_init(ModuleRegistry::new)
}

pub fn register_module(entry: ModuleEntry) {
    global_registry().register(entry);
}

/// Register all built-in modules (demods, decoders, network, products, …).
///
/// In the C++ original this is `registerModules()` in `module.cpp`.
/// Each concrete module would supply its factory here.
pub fn register_builtin_modules() {
    let reg = global_registry();

    // ------------------------------------------------------------------
    // Demods
    // ------------------------------------------------------------------
    // Example stub:
    // reg.register(ModuleEntry {
    //     id: "psk_demod".into(),
    //     default_params: serde_json::json!({"symbolrate": 72000}),
    //     factory: Arc::new(|in_file, out_hint, params| {
    //         Ok(Box::new(demod::PskDemod::new(in_file, out_hint, params)?))
    //     }),
    // });

    // ------------------------------------------------------------------
    // Network
    // ------------------------------------------------------------------
    // reg.register(ModuleEntry { id: "network_server".into(), ... });
    // reg.register(ModuleEntry { id: "network_client".into(), ... });

    // ------------------------------------------------------------------
    // CCSDS decoders
    // ------------------------------------------------------------------
    // reg.register(ModuleEntry { id: "ccsds_turbo_decoder".into(), ... });
    // reg.register(ModuleEntry { id: "ccsds_ldpc_decoder".into(), ... });
    // ...

    // ------------------------------------------------------------------
    // Products processor
    // ------------------------------------------------------------------
    // reg.register(ModuleEntry { id: "products_processor".into(), ... });

    println!("Built-in module registration complete ({} modules)", reg.modules.lock().unwrap().len());
}

// =============================================================================
// Pipeline configuration (serde)
// =============================================================================

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PipelinePreset {
    #[serde(default)]
    pub exists: bool,
    #[serde(default)]
    pub samplerate: u64,
    #[serde(default)]
    pub frequencies: Vec<(String, u64)>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LivePipelineCfg {
    #[serde(default)]
    pub normal_live: Vec<usize>,
    #[serde(default)]
    pub server_live: Vec<usize>,
    #[serde(default)]
    pub client_live: Vec<usize>,
    #[serde(default = "default_pkt_size")]
    pub pkt_size: isize,
}

fn default_pkt_size() -> isize {
    -1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    pub level: String,
    pub module: String,
    #[serde(default)]
    pub parameters: Value,
    #[serde(default)]
    pub input_override: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub editable_parameters: Value,
    #[serde(default)]
    pub preset: PipelinePreset,
    #[serde(default)]
    pub live: bool,
    #[serde(default)]
    pub live_cfg: LivePipelineCfg,
    #[serde(default)]
    pub steps: Vec<PipelineStep>,
}

// =============================================================================
// Pipeline loader / saver
// =============================================================================

static PIPELINES: OnceLock<Mutex<Vec<Pipeline>>> = OnceLock::new();

fn global_pipelines() -> &'static Mutex<Vec<Pipeline>> {
    PIPELINES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Resolve `.json.inc` includes inside a raw JSON string.
fn resolve_includes(raw: &str, base_dir: &Path) -> Result<String> {
    let mut result = raw.to_string();
    let mut replacements: HashMap<String, String> = HashMap::new();

    for (i, _) in raw.match_indices(".json.inc") {
        // walk backwards to find opening quote
        let quote = raw[..i + ".json.inc".len()]
            .rfind('"')
            .context(" malformed include")?;
        let token = &raw[quote..=i + ".json.inc".len() - 1];
        let filename = token.trim_matches('"');
        let path = base_dir.join(filename);

        if path.exists() {
            let contents = fs::read_to_string(&path)?;
            replacements.insert(token.to_string(), contents);
        } else {
            eprintln!("Could not include {}", path.display());
        }
    }

    for (token, contents) in replacements {
        result = result.replace(&token, &contents);
    }
    Ok(result)
}

fn load_one_pipeline_file(path: &Path) -> Result<Value> {
    let raw = fs::read_to_string(path)?;
    let resolved = resolve_includes(&raw, path.parent().unwrap_or_else(|| Path::new(".")))?;
    Ok(serde_json::from_str(&resolved)?)
}

/// Load all pipeline definitions from a directory tree and merge with the
/// user's local overrides (`pipelines.json`).
pub fn load_pipelines(system_dir: &Path, user_path: Option<&Path>) -> Result<()> {
    if !system_dir.exists() {
        bail!("Pipeline directory {} does not exist", system_dir.display());
    }

    let mut system_json = Value::Object(Default::default());

    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(system_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            p.is_file()
                && p.extension().and_then(|s| s.to_str()) == Some("json")
                && !p.to_string_lossy().contains(".json.inc")
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    files.sort();

    for f in &files {
        println!("Found system pipeline file {}", f.display());
        match load_one_pipeline_file(f) {
            Ok(v) => merge_json_values(&mut system_json, v),
            Err(e) => eprintln!("Error loading {}: {}", f.display(), e),
        }
    }

    let effective_user = user_path
        .map(|p| p.to_path_buf())
        .or_else(|| {
            let local = PathBuf::from("pipelines.json");
            if local.exists() {
                Some(local)
            } else {
                None
            }
        });

    let final_user = match effective_user {
        Some(p) if p.exists() => {
            match fs::read_to_string(&p) {
                Ok(s) => match serde_json::from_str(&s) {
                    Ok(v) => {
                        println!("Loaded user pipelines from {}", p.display());
                        Some((v, p))
                    }
                    Err(e) => {
                        eprintln!("Bad user pipelines JSON: {}", e);
                        None
                    }
                },
                Err(e) => {
                    eprintln!("Cannot read {}: {}", p.display(), e);
                    None
                }
            }
        }
        _ => None,
    };

    let merged = match &final_user {
        Some((u, _)) => json_diff_merge(&system_json, u),
        None => system_json,
    };

    parse_pipelines(&merged)?;
    Ok(())
}

/// Parse a merged JSON object into the global pipeline list.
fn parse_pipelines(json: &Value) -> Result<()> {
    let obj = json.as_object().context("root pipeline JSON is not an object")?;
    let mut out = Vec::new();

    for (key, value) in obj {
        let mut pipeline: Pipeline = match serde_json::from_value(value.clone()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Couldn't load pipeline {}: {}", key, e);
                continue;
            }
        };
        pipeline.id = key.clone();

        // Extract preset samplerate from editable_parameters if present.
        if let Some(sr) = pipeline.editable_parameters.get("samplerate") {
            pipeline.preset.samplerate = sr.get("value").and_then(|v| v.as_u64()).unwrap_or(0);
            pipeline.preset.exists = true;
        }

        let mut has_module = false;
        let mut all_present = true;
        let reg = global_registry();

        for step in &pipeline.steps {
            if !step.module.is_empty() {
                has_module = true;
                if !reg.exists(&step.module) {
                    eprintln!("Module {} not loaded; skipping pipeline {}", step.module, pipeline.id);
                    all_present = false;
                    break;
                }
            }
        }

        if !has_module {
            eprintln!("Pipeline {} has no modules!", pipeline.id);
        }

        if all_present && has_module {
            out.push(pipeline);
        }
    }

    out.sort_by(|a, b| {
        a.name.to_lowercase().cmp(&b.name.to_lowercase())
    });

    *global_pipelines().lock().unwrap() = out;
    Ok(())
}

pub fn get_pipeline_by_id(id: &str) -> Result<Pipeline> {
    let list = global_pipelines().lock().unwrap();
    list.iter()
        .find(|p| p.id == id)
        .cloned()
        .with_context(|| format!("Pipeline {id} not found!"))
}

pub fn list_pipelines() -> Vec<Pipeline> {
    global_pipelines().lock().unwrap().clone()
}

// =============================================================================
// JSON helpers (shallow merges for pipeline overrides)
// =============================================================================

fn merge_json_values(base: &mut Value, extra: Value) {
    match (base, extra) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                if let Some(existing) = a.get_mut(&k) {
                    merge_json_values(existing, v);
                } else {
                    a.insert(k, v);
                }
            }
        }
        (base, extra) => *base = extra,
    }
}

fn json_diff_merge(system: &Value, user: &Value) -> Value {
    // In the original code this computes a diff before saving.
    // For loading we just overlay user on top of system.
    let mut result = system.clone();
    merge_json_values(&mut result, user.clone());
    result
}

// =============================================================================
// Pipeline execution — file-based (from pipeline_run.cpp)
// =============================================================================

impl Pipeline {
    /// Merge pipeline-level parameters into module-level defaults.
    pub fn prepare_parameters(module_params: &Value, pipeline_params: &Value) -> Value {
        let mut result = module_params.clone();
        if let (Some(r_obj), Some(p_obj)) = (result.as_object_mut(), pipeline_params.as_object()) {
            for (k, v) in p_obj {
                r_obj.insert(k.clone(), v.clone());
            }
        } else if pipeline_params.is_object() {
            result = pipeline_params.clone();
        }
        println!("Parameters: {}", serde_json::to_string_pretty(&result).unwrap_or_default());
        result
    }

    /// Run the pipeline against a recorded file.
    ///
    /// Translates `Pipeline::run` from `pipeline_run.cpp`.
    pub fn run(
        &self,
        input_file: &Path,
        output_directory: &Path,
        parameters: &Value,
        input_level: &str,
    ) -> Result<PathBuf> {
        if !input_file.exists() {
            bail!("Input file {} does not exist!", input_file.display());
        }

        println!("Starting pipeline {}", self.id);
        let reg = global_registry();

        let mut last_file: Option<PathBuf> = None;
        let mut current_step = 0usize;
        let mut found_level = false;

        // ------------------------------------------------------------------
        // Optimisation: if we start from baseband, try to run the first two
        // modules (indices 1 & 2) in parallel when they support streaming.
        // ------------------------------------------------------------------
        if input_level == "baseband"
            && parameters.get("disable_multi_modules").is_none()
            && self.steps.len() > 2
            && !self.steps[1].module.is_empty()
            && !self.steps[2].module.is_empty()
        {
            println!("Checking the first two modules for parallel run...");

            let s1 = &self.steps[1];
            let s2 = &self.steps[2];

            if !reg.exists(&s1.module) || !reg.exists(&s2.module) {
                bail!(
                    "Module {} or {} is not registered; cancelling pipeline.",
                    s1.module,
                    s2.module
                );
            }

            let p1 = Self::prepare_parameters(&s1.parameters, parameters);
            let p2 = Self::prepare_parameters(&s2.parameters, parameters);

            let in1 = if s1.input_override.is_empty() {
                input_file.to_path_buf()
            } else {
                output_directory.join(&s1.input_override)
            };
            let in2 = if s2.input_override.is_empty() {
                input_file.to_path_buf()
            } else {
                output_directory.join(&s2.input_override)
            };

            let mut m1 = reg.instantiate(&s1.module, &in1, &output_directory.join(&self.id), &p1)?;
            let mut m2 = reg.instantiate(&s2.module, &in2, &output_directory.join(&self.id), &p2)?;

            let m1_can_stream = m1.output_types().contains(&DataType::Stream);
            let m2_can_stream = m2.input_types().contains(&DataType::Stream);

            if m1_can_stream && m2_can_stream {
                println!("Both first modules can run in parallel!");

                let fifo = Arc::new(ByteFifo::new(1_000_000));

                m1.set_input_type(DataType::File)?;
                m1.set_output_type(DataType::Stream)?;
                m1.set_output_fifo(Some(Arc::clone(&fifo)));
                m1.init()?;

                m2.set_input_type(DataType::Stream)?;
                m2.set_input_fifo(Some(Arc::clone(&fifo)));
                m2.set_output_type(DataType::File)?;
                m2.set_input_active(true);
                m2.init()?;

                let m1 = Arc::new(Mutex::new(m1));
                let m2 = Arc::new(Mutex::new(m2));

                let h1 = {
                    let m = Arc::clone(&m1);
                    thread::spawn(move || {
                        let mut g = m.lock().unwrap();
                        println!("Start processing module {}", g.id());
                        g.process()
                    })
                };

                let h2 = {
                    let m = Arc::clone(&m2);
                    thread::spawn(move || {
                        let mut g = m.lock().unwrap();
                        println!("Start processing module {}", g.id());
                        g.process()
                    })
                };

                join_module(h1)?;

                // Wait for FIFO to drain (mirrors original while-readable sleep).
                while !fifo.rx.is_empty() {
                    thread::sleep(Duration::from_secs(1));
                }

                {
                    let mut g = m2.lock().unwrap();
                    g.set_input_active(false);
                    g.stop()?;
                }

                join_module(h2)?;

                let g = m2.lock().unwrap();
                if let Some(out) = g.output_file() {
                    last_file = Some(out.to_path_buf());
                }
                current_step += 2;
                // In C++ the next loop iteration effectively starts at step 3,
                // with the data level considered equal to steps[2].level.
                // We therefore mark the level as already found if it matches.
                if self.steps.get(2).map(|s| s.level.as_str()) == Some(input_level) {
                    found_level = true;
                }
            }
        }

        // ------------------------------------------------------------------
        // Sequential stages
        // ------------------------------------------------------------------
        for step in self.steps.iter().skip(current_step) {
            if !found_level {
                if step.level == input_level {
                    found_level = true;
                    println!("Data is already at level {}, skipping", step.level);
                }
                continue;
            }

            println!("Processing data to level {}", step.level);

            if !step.module.is_empty() && !reg.exists(&step.module) {
                bail!("Module {} is not registered; cancelling pipeline.", step.module);
            }

            let final_params = Self::prepare_parameters(&step.parameters, parameters);

            let input_path: PathBuf = if step.input_override.is_empty() {
                last_file.as_ref().map(|p| p.clone()).unwrap_or_else(|| input_file.to_path_buf())
            } else {
                output_directory.join(&step.input_override)
            };

            let mut module = reg.instantiate(
                &step.module,
                &input_path,
                &output_directory.join(&self.id),
                &final_params,
            )?;

            module.set_input_type(DataType::File)?;
            module.set_output_type(DataType::File)?;
            module.init()?;

            println!("Running module {}", module.id());
            module.process()?;

            if let Some(out) = module.output_file() {
                last_file = Some(out.to_path_buf());
            }
        }

        // ------------------------------------------------------------------
        // Auto-process products if a dataset is present.
        // ------------------------------------------------------------------
        let dataset_json = output_directory.join("dataset.json");
        let input_is_dataset = input_file.file_stem() == Some(OsStr::new("dataset"))
            && input_file.extension() == Some(OsStr::new("json"));

        if dataset_json.exists() || input_is_dataset {
            println!("Products processing enabled! Starting products_processor.");
            let ds = if input_is_dataset { input_file } else { &dataset_json };

            let mut pp = reg.instantiate(
                "products_processor",
                ds,
                &output_directory.join(&self.id),
                &Value::Null,
            )?;

            pp.set_input_type(DataType::File)?;
            pp.set_output_type(DataType::File)?;
            pp.init()?;
            pp.process()?;
        }

        on_pipeline_done(&self.id, output_directory);

        Ok(last_file.unwrap_or_else(|| input_file.to_path_buf()))
    }
}

fn join_module(handle: JoinHandle<Result<(), PipelineError>>) -> Result<()> {
    match handle.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
        Err(e) => Err(anyhow!("Module thread panicked: {:?}", e)),
    }
}

// =============================================================================
// Live pipeline (from live_pipeline.cpp)
// =============================================================================

pub struct LivePipeline {
    pipeline: Pipeline,
    parameters: Value,
    output_dir: PathBuf,
    modules: Vec<Arc<Mutex<Box<dyn ProcessingModule>>>>,
    handles: Vec<JoinHandle<Result<(), PipelineError>>>,
}

impl LivePipeline {
    pub fn new(pipeline: Pipeline, parameters: Value, output_dir: PathBuf) -> Self {
        Self {
            pipeline,
            parameters,
            output_dir,
            modules: Vec::new(),
            handles: Vec::new(),
        }
    }

    fn prepare_modules(&mut self, indices: &[usize]) -> Result<()> {
        let reg = global_registry();
        for &idx in indices {
            if idx >= self.pipeline.steps.len() {
                bail!(PipelineError::InvalidStep(idx));
            }
            let step = &self.pipeline.steps[idx];
            let params = Pipeline::prepare_parameters(&step.parameters, &self.parameters);
            println!("Module {}", step.module);
            println!("Parameters: {}", serde_json::to_string_pretty(&params)?);
            let m = reg.instantiate(
                &step.module,
                Path::new(""),
                &self.output_dir.join(&self.pipeline.id),
                &params,
            )?;
            self.modules.push(Arc::new(Mutex::new(m)));
        }
        Ok(())
    }

    fn prepare_module_by_id(&mut self, id: &str) -> Result<()> {
        let reg = global_registry();
        println!("Module {}", id);
        println!("Parameters: {}", serde_json::to_string_pretty(&self.parameters)?);
        let m = reg.instantiate(id, Path::new(""), &self.output_dir.join(&self.pipeline.id), &self.parameters)?;
        self.modules.push(Arc::new(Mutex::new(m)));
        Ok(())
    }

    /// Start locally (or in server mode) with a real-time DSP input stream.
    pub fn start(&mut self, stream: DspStreamHandle, server: bool) -> Result<()> {
        let server_live = self.pipeline.live_cfg.server_live.clone();
        let normal_live = self.pipeline.live_cfg.normal_live.clone();
        let pkt_size = self.pipeline.live_cfg.pkt_size;

        if server {
            if server_live.is_empty() {
                bail!(PipelineError::UnsupportedLiveMode(self.pipeline.id.clone(), "server".into()));
            }
            self.prepare_modules(&server_live)?;
            if let Some(obj) = self.parameters.as_object_mut() {
                obj.insert("pkt_size".into(), pkt_size.into());
            }
            self.prepare_module_by_id("network_server")?;
        } else {
            self.prepare_modules(&normal_live)?;
        }

        if self.modules.is_empty() {
            bail!("No modules prepared for live pipeline.");
        }

        // We build a Vec of intermediate FIFOs so we can hand them out.
        let mut fifos: Vec<Arc<ByteFifo>> = Vec::new();

        // First module
        {
            let mut g = self.modules[0].lock().unwrap();
            g.set_input_dsp_stream(Some(stream));
            g.set_input_type(DataType::DspStream)?;
            let out_ty = if self.modules.len() > 1 { DataType::Stream } else { DataType::File };
            g.set_output_type(out_ty)?;
            if self.modules.len() > 1 {
                let fifo = Arc::new(ByteFifo::new(1_000_000));
                g.set_output_fifo(Some(Arc::clone(&fifo)));
                fifos.push(fifo);
            }
            g.init()?;
            g.set_input_active(true);
        }

        let m = Arc::clone(&self.modules[0]);
        self.handles.push(thread::spawn(move || {
            let mut g = m.lock().unwrap();
            println!("Start processing module {}", g.id());
            g.process()
        }));

        // Middle modules
        for i in 1..self.modules.len().saturating_sub(1) {
            let input_fifo = Arc::clone(&fifos[i - 1]);
            let output_fifo = Arc::new(ByteFifo::new(1_000_000));
            {
                let mut g = self.modules[i].lock().unwrap();
                g.set_input_fifo(Some(input_fifo));
                g.set_output_fifo(Some(Arc::clone(&output_fifo)));
                g.set_input_type(DataType::Stream)?;
                g.set_output_type(DataType::Stream)?;
                g.init()?;
                g.set_input_active(true);
            }
            fifos.push(output_fifo);

            let m = Arc::clone(&self.modules[i]);
            self.handles.push(thread::spawn(move || {
                let mut g = m.lock().unwrap();
                println!("Start processing module {}", g.id());
                g.process()
            }));
        }

        // Last module
        if self.modules.len() > 1 {
            let last = self.modules.len() - 1;
            let input_fifo = Arc::clone(&fifos[last - 1]);
            {
                let mut g = self.modules[last].lock().unwrap();
                g.set_input_fifo(Some(input_fifo));
                g.set_input_type(DataType::Stream)?;
                g.set_output_type(DataType::File)?;
                g.init()?;
                g.set_input_active(true);
            }

            let m = Arc::clone(&self.modules[last]);
            self.handles.push(thread::spawn(move || {
                let mut g = m.lock().unwrap();
                println!("Start processing module {}", g.id());
                g.process()
            }));
        }

        Ok(())
    }

    /// Start in client mode (no local input; first module receives from network).
    pub fn start_client(&mut self) -> Result<()> {
        let client_live = self.pipeline.live_cfg.client_live.clone();
        let pkt_size = self.pipeline.live_cfg.pkt_size;

        if client_live.is_empty() {
            bail!(PipelineError::UnsupportedLiveMode(
                self.pipeline.id.clone(),
                "client".into(),
            ));
        }

        if let Some(obj) = self.parameters.as_object_mut() {
            obj.insert("pkt_size".into(), pkt_size.into());
        }
        self.prepare_module_by_id("network_client")?;
        self.prepare_modules(&client_live)?;

        if self.modules.is_empty() {
            bail!("No modules prepared for client live pipeline.");
        }

        let mut fifos: Vec<Arc<ByteFifo>> = Vec::new();

        // First and middle modules
        for i in 0..self.modules.len().saturating_sub(1) {
            let input_fifo = if i > 0 {
                Some(Arc::clone(&fifos[i - 1]))
            } else {
                None
            };
            let output_fifo = Arc::new(ByteFifo::new(1_000_000));
            {
                let mut g = self.modules[i].lock().unwrap();
                g.set_input_fifo(input_fifo);
                g.set_output_fifo(Some(Arc::clone(&output_fifo)));
                g.set_input_type(DataType::Stream)?;
                g.set_output_type(DataType::Stream)?;
                g.init()?;
                g.set_input_active(true);
            }
            fifos.push(output_fifo);

            let m = Arc::clone(&self.modules[i]);
            self.handles.push(thread::spawn(move || {
                let mut g = m.lock().unwrap();
                println!("Start processing module {}", g.id());
                g.process()
            }));
        }

        // Last module
        if self.modules.len() > 1 {
            let last = self.modules.len() - 1;
            let input_fifo = Arc::clone(&fifos[last - 1]);
            {
                let mut g = self.modules[last].lock().unwrap();
                g.set_input_fifo(Some(input_fifo));
                g.set_input_type(DataType::Stream)?;
                g.set_output_type(DataType::File)?;
                g.init()?;
                g.set_input_active(true);
            }

            let m = Arc::clone(&self.modules[last]);
            self.handles.push(thread::spawn(move || {
                let mut g = m.lock().unwrap();
                println!("Start processing module {}", g.id());
                g.process()
            }));
        }

        Ok(())
    }

    /// Stop all modules and wait for them to finish.
    pub fn stop(&mut self) -> Result<()> {
        println!("Stop processing");
        for (_i, m) in self.modules.iter().enumerate() {
            let mut g = m.lock().unwrap();
            g.set_input_active(false);
            match g.input_type() {
                DataType::DspStream => {
                    // Dropping the stream handle would close the channel.
                    g.set_input_dsp_stream(None);
                }
                DataType::Stream => {
                    g.set_input_fifo(None);
                }
                _ => {}
            }
            g.stop()?;
        }
        // Drain handles
        while let Some(h) = self.handles.pop() {
            join_module(h)?;
        }
        Ok(())
    }

    pub fn output_file(&self) -> Option<PathBuf> {
        self.modules.last().and_then(|m| {
            let g = m.lock().unwrap();
            g.output_file().map(|p| p.to_path_buf())
        })
    }

    pub fn modules_stats(&self) -> Value {
        let mut v = serde_json::Map::new();
        for m in &self.modules {
            let g = m.lock().unwrap();
            v.insert(g.id().to_string(), g.stats());
        }
        Value::Object(v)
    }

    pub fn draw_uis(&mut self) {
        for m in &self.modules {
            let mut g = m.lock().unwrap();
            g.draw_ui(true);
        }
    }
}

// =============================================================================
// Events
// =============================================================================

pub fn on_pipeline_done(pipeline_id: &str, output_directory: &Path) {
    println!(
        "Pipeline {} finished. Output in {}",
        pipeline_id,
        output_directory.display()
    );
    // In the C++ original this fires eventBus->fire_event<PipelineDoneProcessingEvent>().
    // Rust callers can attach callbacks or channels as desired.
}

// stub out walkdir usage since it is not in Cargo.toml
// A minimal recursive reader is provided instead.
mod walkdir {
    use std::fs;
    use std::path::Path;

    pub struct DirEntry {
        path: std::path::PathBuf,
        is_dir: bool,
    }

    impl DirEntry {
        pub fn path(&self) -> &Path {
            &self.path
        }
        pub fn is_file(&self) -> bool {
            !self.is_dir
        }
        pub fn is_dir(&self) -> bool {
            self.is_dir
        }
    }

    pub struct WalkDir {
        stack: Vec<std::path::PathBuf>,
    }

    impl WalkDir {
        pub fn new(root: &Path) -> Self {
            Self {
                stack: vec![root.to_path_buf()],
            }
        }
    }

    impl Iterator for WalkDir {
        type Item = Result<DirEntry, std::io::Error>;
        fn next(&mut self) -> Option<Self::Item> {
            while let Some(path) = self.stack.pop() {
                if let Ok(metadata) = fs::metadata(&path) {
                    let is_dir = metadata.is_dir();
                    if is_dir {
                        if let Ok(entries) = fs::read_dir(&path) {
                            for entry in entries.flatten() {
                                self.stack.push(entry.path());
                            }
                        }
                        return Some(Ok(DirEntry { path, is_dir: true }));
                    } else {
                        return Some(Ok(DirEntry { path, is_dir: false }));
                    }
                }
            }
            None
        }
    }

    pub fn read_dir(root: &Path) -> impl Iterator<Item = Result<DirEntry, std::io::Error>> {
        WalkDir::new(root)
    }
}
