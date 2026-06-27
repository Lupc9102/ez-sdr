use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Metadata for a loadable module.
/// Translates `ModuleManager::ModuleInfo_t`.
pub struct ModuleInfo {
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: (u32, u32, u32),
    pub max_instances: i32,
}

/// Trait requested by the user: name, init, start, stop, menu.
/// Also includes `is_enabled` to faithfully translate C++ `Instance::isEnabled()`.
pub trait Module: Send {
    fn name(&self) -> &str;
    fn init(&mut self);
    fn start(&mut self);
    fn stop(&mut self);
    fn menu(&mut self);
    fn is_enabled(&self) -> bool {
        false
    }
}

/// Factory interface for a dynamic module.
/// Mirrors the C++ shared-object symbols `_INIT_`, `_CREATE_INSTANCE_`, and `_END_`.
pub trait ModuleFactory: Send + Sync {
    fn info(&self) -> &ModuleInfo;
    fn init(&mut self);
    fn create_instance(&self, name: &str) -> Box<dyn Module>;
    fn end(&mut self);
}

struct InstanceEntry {
    module_name: String,
    instance: Box<dyn Module>,
}

/// Registry that owns module factories and active instances.
/// Translates `ModuleManager` logic into idiomatic Rust (HashMap instead of std::map, etc.).
pub struct ModuleManager {
    factories: HashMap<String, Arc<Mutex<Box<dyn ModuleFactory>>>>,
    instances: HashMap<String, InstanceEntry>,
}

impl ModuleManager {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
            instances: HashMap::new(),
        }
    }

    /// Register a new module factory, calling its one-time `init()`.
    /// Equivalent to the successful branch of C++ `loadModule()`.
    pub fn register_factory(&mut self, mut factory: Box<dyn ModuleFactory>) -> Result<(), String> {
        let info = factory.info();
        let name = info.name.clone();
        if self.factories.contains_key(&name) {
            return Err(format!(
                "'{name}' has the same name as an already loaded module"
            ));
        }
        factory.init();
        self.factories.insert(name, Arc::new(Mutex::new(factory)));
        Ok(())
    }

    /// Create a named instance from a registered module.
    /// Mirrors `ModuleManager::createInstance()`.
    pub fn create_instance(&mut self, name: &str, module: &str) -> Result<(), String> {
        let factory = self
            .factories
            .get(module)
            .ok_or_else(|| format!("Module '{module}' doesn't exist"))?;

        if self.instances.contains_key(name) {
            return Err(format!(
                "A module instance with the name '{name}' already exists"
            ));
        }

        let guard = factory.lock().unwrap();
        let max = guard.info().max_instances;
        if max > 0 && self.count_module_instances(module) >= max as usize {
            return Err(format!(
                "Maximum number of instances reached for '{module}'"
            ));
        }

        let mut instance = guard.create_instance(name);
        drop(guard);
        instance.init();

        self.instances.insert(
            name.to_string(),
            InstanceEntry {
                module_name: module.to_string(),
                instance,
            },
        );
        Ok(())
    }

    /// Destroy a named instance. Mirrors `ModuleManager::deleteInstance(std::string)`.
    pub fn delete_instance(&mut self, name: &str) -> Result<(), String> {
        self.instances
            .remove(name)
            .ok_or_else(|| format!("Tried to remove non-existent instance '{name}'"))?;
        Ok(())
    }

    /// Start (enable) an existing instance. Mirrors `ModuleManager::enableInstance`.
    pub fn enable_instance(&mut self, name: &str) -> Result<(), String> {
        let entry = self
            .instances
            .get_mut(name)
            .ok_or_else(|| format!("Cannot enable '{name}', instance doesn't exist"))?;
        entry.instance.start();
        Ok(())
    }

    /// Stop (disable) an existing instance. Mirrors `ModuleManager::disableInstance`.
    pub fn disable_instance(&mut self, name: &str) -> Result<(), String> {
        let entry = self
            .instances
            .get_mut(name)
            .ok_or_else(|| format!("Cannot disable '{name}', instance doesn't exist"))?;
        entry.instance.stop();
        Ok(())
    }

    /// Query whether an instance is currently enabled.
    /// Mirrors `ModuleManager::instanceEnabled`.
    pub fn instance_enabled(&self, name: &str) -> Result<bool, String> {
        let entry = self
            .instances
            .get(name)
            .ok_or_else(|| {
                format!("Cannot check if '{name}' is enabled, instance doesn't exist")
            })?;
        Ok(entry.instance.is_enabled())
    }

    /// Run post-init for a single instance. Mirrors `ModuleManager::postInit`.
    pub fn post_init(&mut self, name: &str) -> Result<(), String> {
        let entry = self
            .instances
            .get_mut(name)
            .ok_or_else(|| format!("Cannot post-init '{name}', instance doesn't exist"))?;
        entry.instance.init();
        Ok(())
    }

    /// Run post-init for every instance. Mirrors `ModuleManager::doPostInitAll`.
    pub fn post_init_all(&mut self) {
        for (name, entry) in &mut self.instances {
            eprintln!("Running post-init for {name}");
            entry.instance.init();
        }
    }

    /// Stop all instances and call `end()` on every factory.
    /// Mirrors the shutdown sequence at the end of `sdrpp_main`.
    pub fn shutdown_all(&mut self) {
        for (_name, entry) in &mut self.instances {
            entry.instance.stop();
        }
        for (_name, factory) in &self.factories {
            if let Ok(mut f) = factory.try_lock() {
                f.end();
            }
        }
        self.instances.clear();
        self.factories.clear();
    }

    /// Return the registered module type for a given instance.
    /// Mirrors `ModuleManager::getInstanceModuleName`.
    pub fn get_instance_module_name(&self, name: &str) -> Result<&str, String> {
        let entry = self.instances.get(name).ok_or_else(|| {
            format!("Cannot get module name of '{name}', instance doesn't exist")
        })?;
        Ok(&entry.module_name)
    }

    /// Count how many instances exist for a given module type.
    /// Mirrors `ModuleManager::countModuleInstances`.
    pub fn count_module_instances(&self, module: &str) -> usize {
        self.instances
            .values()
            .filter(|e| e.module_name == module)
            .count()
    }
}

impl Default for ModuleManager {
    fn default() -> Self {
        Self::new()
    }
}
