use crate::tle_engine::PassInfo;

pub struct Scheduler {
    pub jobs: Vec<ScheduledJob>,
    pub auto_tune_enabled: bool,
    pub custom_tasks: Vec<CustomTask>,
}

#[derive(Debug, Clone)]
pub struct ScheduledJob {
    pub satellite: String,
    pub aos: String,
    pub los: String,
    pub frequency_hz: u64,
    pub aos_dt: f64,
    pub los_dt: f64,
}

/// A one-shot "tune to frequency at time" task
#[derive(Debug, Clone)]
pub struct CustomTask {
    pub label: String,
    pub frequency_hz: u64,
    pub at_unix: f64,
    pub fired: bool,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            jobs: vec![],
            auto_tune_enabled: true,
            custom_tasks: vec![],
        }
    }

    pub fn update_from_passes(&mut self, passes: &[PassInfo]) {
        self.jobs = passes.iter().map(|p| ScheduledJob {
            satellite: p.satellite.clone(),
            aos: p.aos.clone(),
            los: p.los.clone(),
            frequency_hz: p.frequency_hz,
            aos_dt: p.aos_dt,
            los_dt: p.los_dt,
        }).collect();
    }

    /// Returns the first job whose AOS-LOS window contains `now_unix`, if any.
    pub fn active_job(&self, now_unix: f64) -> Option<&ScheduledJob> {
        if !self.auto_tune_enabled { return None; }
        self.jobs.iter().find(|j| now_unix >= j.aos_dt && now_unix <= j.los_dt)
    }

    /// Check if any custom task should fire now. Returns frequency if fired.
    pub fn poll_custom_tasks(&mut self, now_unix: f64) -> Option<(String, u64)> {
        for task in &mut self.custom_tasks {
            if !task.fired && now_unix >= task.at_unix {
                task.fired = true;
                return Some((task.label.clone(), task.frequency_hz));
            }
        }
        None
    }
}
