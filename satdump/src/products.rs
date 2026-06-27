//! Product handlers - translated from src-core/products/

use crate::image::PixelType;
use std::collections::HashMap;
use std::path::Path;

pub type JsonValue = serde_json::Value;

#[derive(Debug, Clone)]
pub struct Product {
    pub contents: JsonValue,
    pub instrument_name: String,
    pub product_type: String,
    pub d_use_preset_cache: bool,
}

impl Default for Product {
    fn default() -> Self {
        Self {
            contents: JsonValue::Object(Default::default()),
            instrument_name: String::new(),
            product_type: String::new(),
            d_use_preset_cache: false,
        }
    }
}

impl Product {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_product_timestamp(&mut self, timestamp: f64) {
        self.contents["product_timestamp"] = timestamp.into();
    }

    pub fn has_product_timestamp(&self) -> bool {
        self.contents.get("product_timestamp").is_some()
    }

    pub fn get_product_timestamp(&self) -> f64 {
        self.contents["product_timestamp"].as_f64().unwrap_or(0.0)
    }

    pub fn set_product_source(&mut self, source: &str) {
        self.contents["product_source"] = source.into();
    }

    pub fn has_product_source(&self) -> bool {
        self.contents.get("product_source").is_some()
    }

    pub fn get_product_source(&self) -> String {
        self.contents["product_source"].as_str().unwrap_or("").to_string()
    }

    pub fn set_product_id(&mut self, id: &str) {
        self.contents["product_id"] = id.into();
    }

    pub fn has_product_id(&self) -> bool {
        self.contents.get("product_id").is_some()
    }

    pub fn get_product_id(&self) -> String {
        self.contents["product_id"].as_str().unwrap_or("").to_string()
    }

    pub fn save<P: AsRef<Path>>(&mut self, directory: P) -> anyhow::Result<()> {
        self.contents["instrument"] = self.instrument_name.clone().into();
        self.contents["type"] = self.product_type.clone().into();
        let path = directory.as_ref().join("product.json");
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, &self.contents)?;
        Ok(())
    }

    pub fn load<P: AsRef<Path>>(&mut self, file: P) -> anyhow::Result<()> {
        let data = std::fs::read_to_string(file.as_ref())?;
        self.contents = serde_json::from_str(&data)?;
        self.instrument_name = self.contents["instrument"].as_str().unwrap_or("").to_string();
        self.product_type = self.contents["type"].as_str().unwrap_or("").to_string();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelPolarization {
    None,
    Horizontal,
    Vertical,
    Rhcp,
    Lhcp,
    Any,
}

#[derive(Debug, Clone)]
pub struct ImageHolder {
    pub abs_index: i32,
    pub filename: String,
    pub channel_name: String,
    pub image: crate::image::Image<u16>,
    pub bit_depth: i32,
    pub wavenumber: f64,
    pub bandwidth: f64,
    pub calibration_type: String,
    pub polarization: ChannelPolarization,
}

impl Default for ImageHolder {
    fn default() -> Self {
        Self {
            abs_index: -1,
            filename: String::new(),
            channel_name: String::new(),
            image: crate::image::Image::new(1, 1, 1),
            bit_depth: 16,
            wavenumber: -1.0,
            bandwidth: -1.0,
            calibration_type: String::new(),
            polarization: ChannelPolarization::None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImageProduct {
    pub base: Product,
    pub images: Vec<ImageHolder>,
    pub save_as_matrix: bool,
    pub d_no_not_save_images: bool,
    pub d_no_not_load_images: bool,
}

impl Default for ImageProduct {
    fn default() -> Self {
        let mut base = Product::new();
        base.product_type = "image".into();
        Self {
            base,
            images: Vec::new(),
            save_as_matrix: false,
            d_no_not_save_images: false,
            d_no_not_load_images: false,
        }
    }
}

impl ImageProduct {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_proj_cfg(&mut self, cfg: JsonValue) {
        self.base.contents["projection_cfg"] = cfg.clone();
        if let Some(name) = cfg.get("tle").and_then(|v| v.get("name")).and_then(|v| v.as_str()) {
            if !self.base.has_product_source() {
                self.base.set_product_source(name);
            }
        }
        if cfg.get("timestamps").is_some() && !self.base.has_product_timestamp() {
            let timestamps: Vec<f64> = cfg["timestamps"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
                .unwrap_or_default();
            if let Some(&median) = timestamps.get(timestamps.len() / 2) {
                self.base.set_product_timestamp(median);
            }
        }
    }

    pub fn has_proj_cfg(&self) -> bool {
        self.base.contents.get("projection_cfg").is_some()
    }

    pub fn get_proj_cfg(&self) -> Option<&JsonValue> {
        self.base.contents.get("projection_cfg")
    }

    pub fn set_calibration(&mut self, calibrator: &str, cfg: JsonValue) {
        self.base.contents["calibration"] = cfg;
        self.base.contents["calibration"]["calibrator"] = calibrator.into();
    }

    pub fn has_calibration(&self) -> bool {
        self.base.contents.get("calibration").is_some()
    }

    pub fn get_calibration_raw(&self) -> JsonValue {
        self.base.contents.get("calibration").cloned().unwrap_or_default()
    }

    pub fn get_channel_image(&self, index: i32) -> Option<&ImageHolder> {
        self.images.iter().find(|img| img.abs_index == index)
    }

    pub fn get_channel_image_by_name(&self, name: &str) -> Option<&ImageHolder> {
        self.images.iter().find(|img| img.channel_name == name)
    }

    pub fn get_channel_image_idx(&self, name: &str) -> Option<usize> {
        self.images.iter().position(|img| img.channel_name == name)
    }

    pub fn set_channel_wavenumber(&mut self, index: i32, wavenumber: f64) {
        if let Some(img) = self.images.iter_mut().find(|img| img.abs_index == index) {
            img.wavenumber = wavenumber;
        }
    }

    pub fn set_channel_frequency(&mut self, index: i32, frequency: f64) {
        self.set_channel_wavenumber(index, freq_to_wavenumber(frequency));
    }

    pub fn get_channel_wavenumber(&self, index: i32) -> Option<f64> {
        self.images.iter().find(|img| img.abs_index == index).map(|img| img.wavenumber)
    }

    pub fn get_channel_frequency(&self, index: i32) -> Option<f64> {
        self.get_channel_wavenumber(index).map(wavenumber_to_freq)
    }

    pub fn set_channel_polarization(&mut self, index: i32, pol: ChannelPolarization) {
        if let Some(img) = self.images.iter_mut().find(|img| img.abs_index == index) {
            img.polarization = pol;
        }
    }

    pub fn get_channel_polarization(&self, index: i32) -> Option<ChannelPolarization> {
        self.images.iter().find(|img| img.abs_index == index).map(|img| img.polarization)
    }

    pub fn set_channel_bandwidth(&mut self, index: i32, bandwidth: f64) {
        if let Some(img) = self.images.iter_mut().find(|img| img.abs_index == index) {
            img.bandwidth = bandwidth;
        }
    }

    pub fn get_channel_bandwidth(&self, index: i32) -> Option<f64> {
        self.images.iter().find(|img| img.abs_index == index).map(|img| img.bandwidth)
    }

    pub fn set_channel_unit(&mut self, index: i32, unit: &str) {
        if let Some(img) = self.images.iter_mut().find(|img| img.abs_index == index) {
            img.calibration_type = unit.to_string();
        }
    }

    pub fn save<P: AsRef<Path>>(&mut self, directory: P) -> anyhow::Result<()> {
        let dir = directory.as_ref();
        let mut images_json = Vec::new();
        if !self.d_no_not_save_images {
            for img in &self.images {
                let mut path = dir.join(&img.filename);
                if path.extension().is_none() {
                    path.set_extension("png");
                }
                img.image.save(&path)?;
                let mut obj = serde_json::Map::new();
                obj.insert("file".to_string(), img.filename.clone().into());
                obj.insert("name".to_string(), img.channel_name.clone().into());
                obj.insert("abs_index".to_string(), img.abs_index.into());
                obj.insert("bit_depth".to_string(), img.bit_depth.into());
                if img.wavenumber != -1.0 {
                    obj.insert("wavenumber".to_string(), img.wavenumber.into());
                }
                if img.bandwidth != -1.0 {
                    obj.insert("bandwidth".to_string(), img.bandwidth.into());
                }
                if !img.calibration_type.is_empty() {
                    obj.insert("calibration_type".to_string(), img.calibration_type.clone().into());
                }
                images_json.push(serde_json::Value::Object(obj));
            }
        }
        self.base.contents["images"] = serde_json::Value::Array(images_json);
        self.base.save(dir)
    }

    pub fn load<P: AsRef<Path>>(&mut self, file: P) -> anyhow::Result<()> {
        self.base.load(&file)?;
        if let Some(arr) = self.base.contents["images"].as_array() {
            self.images.clear();
            for item in arr {
                let holder = ImageHolder {
                    abs_index: item.get("abs_index").and_then(|v| v.as_i64()).unwrap_or(-1) as i32,
                    filename: item.get("file").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    channel_name: item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    image: crate::image::Image::new(1, 1, 1),
                    bit_depth: item.get("bit_depth").and_then(|v| v.as_i64()).unwrap_or(16) as i32,
                    wavenumber: item.get("wavenumber").and_then(|v| v.as_f64()).unwrap_or(-1.0),
                    bandwidth: item.get("bandwidth").and_then(|v| v.as_f64()).unwrap_or(-1.0),
                    calibration_type: item.get("calibration_type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    polarization: ChannelPolarization::None,
                };
                self.images.push(holder);
            }
        }
        Ok(())
    }

    pub fn get_raw_channel_val(&self, idx: usize, x: usize, y: usize) -> i32 {
        if let Some(i) = self.images.get(idx) {
            let depth_diff = i.bit_depth - i.image.maxval() as i32;
            let val = i.image.get(0, x, y).to_u32() as i32;
            if depth_diff > 0 {
                val << depth_diff
            } else {
                val >> -depth_diff
            }
        } else {
            0
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataSet {
    pub satellite_name: String,
    pub timestamp: f64,
    pub products_list: Vec<String>,
}

impl DataSet {
    pub fn new() -> Self {
        Self {
            satellite_name: String::new(),
            timestamp: 0.0,
            products_list: Vec::new(),
        }
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> anyhow::Result<()> {
        let val = serde_json::json!({
            "satellite": self.satellite_name,
            "timestamp": self.timestamp,
            "products": self.products_list,
        });
        let file = std::fs::File::create(path.as_ref().join("dataset.json"))?;
        serde_json::to_writer_pretty(file, &val)?;
        Ok(())
    }

    pub fn load<P: AsRef<Path>>(&mut self, path: P) -> anyhow::Result<()> {
        let data = std::fs::read_to_string(path.as_ref().join("dataset.json"))?;
        let val: serde_json::Value = serde_json::from_str(&data)?;
        self.satellite_name = val["satellite"].as_str().unwrap_or("").to_string();
        self.timestamp = val["timestamp"].as_f64().unwrap_or(0.0);
        if let Some(arr) = val["products"].as_array() {
            self.products_list = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct RegisteredProduct {
    pub load_from_file: fn(&str) -> Box<Product>,
}

pub type ProductLoaderRegistry = HashMap<String, RegisteredProduct>;

pub fn register_products(registry: &mut ProductLoaderRegistry) {
    registry.clear();
    registry.insert(
        "image".to_string(),
        RegisteredProduct {
            load_from_file: |path| {
                let mut p = ImageProduct::new();
                let _ = p.load(path);
                Box::new(p.base)
            },
        },
    );
}

pub fn load_product(path: &str) -> anyhow::Result<Box<Product>> {
    let mut raw = Product::new();
    raw.load(path)?;
    Ok(Box::new(raw))
}

const SPEED_OF_LIGHT_M_S: f64 = 299792458.0;

fn freq_to_wavenumber(freq: f64) -> f64 {
    freq / SPEED_OF_LIGHT_M_S
}

fn wavenumber_to_freq(wavenumber: f64) -> f64 {
    wavenumber * SPEED_OF_LIGHT_M_S
}
