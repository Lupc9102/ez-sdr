//! Map projections - translated from src-core/projection/

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjType {
    Invalid,
    Standard,
    Raytracer,
    ThinPlateSpline,
}

#[derive(Debug, Clone)]
pub struct Projection {
    pub config: Value,
    pub width: i32,
    pub height: i32,
    fwd_type: ProjType,
    inv_type: ProjType,
    proj_timestamp: f64,
}

impl Default for Projection {
    fn default() -> Self {
        Self {
            config: Value::Object(Default::default()),
            width: -1,
            height: -1,
            fwd_type: ProjType::Invalid,
            inv_type: ProjType::Invalid,
            proj_timestamp: -1.0,
        }
    }
}

impl Projection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_fwd_type(&self) -> ProjType {
        self.fwd_type
    }

    pub fn get_inv_type(&self) -> ProjType {
        self.inv_type
    }

    pub fn init(&mut self, fwd: bool, inv: bool) -> bool {
        if let Some(v) = self.config.get("width") {
            self.width = v.as_i64().unwrap_or(-1) as i32;
        } else {
            return false;
        }
        if let Some(v) = self.config.get("height") {
            self.height = v.as_i64().unwrap_or(-1) as i32;
        } else {
            return false;
        }
        if self.config.get("proj_timestamp").is_some() {
            self.proj_timestamp = self.config["proj_timestamp"].as_f64().unwrap_or(-1.0);
        }
        if fwd {
            self.fwd_type = ProjType::Standard;
        }
        if inv {
            self.inv_type = ProjType::Standard;
        }
        true
    }

    pub fn forward(&self, lon: f64, lat: f64, except: bool) -> Result<(f64, f64), &'static str> {
        if self.fwd_type == ProjType::Invalid {
            if except {
                return Err("invalid forward projection");
            }
            return Ok((0.0, 0.0));
        }
        Ok((lon, lat))
    }

    pub fn inverse(&self, x: f64, y: f64, except: bool) -> Result<(f64, f64, f64), &'static str> {
        if self.inv_type == ProjType::Invalid {
            if except {
                return Err("invalid inverse projection");
            }
            return Ok((0.0, 0.0, -1.0));
        }
        Ok((x, y, self.proj_timestamp))
    }

    pub fn to_json(&self) -> Value {
        let mut j = self.config.clone();
        if self.height != -1 {
            j["height"] = self.height.into();
        }
        if self.width != -1 {
            j["width"] = self.width.into();
        }
        j
    }

    pub fn from_json(&mut self, j: &Value) {
        self.config = j.clone();
        self.height = j.get("height").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
        self.width = j.get("width").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
        self.fwd_type = ProjType::Invalid;
        self.inv_type = ProjType::Invalid;
        self.init(true, true);
    }
}
