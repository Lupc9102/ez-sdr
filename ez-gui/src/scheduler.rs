use crate::tle_engine::PassInfo;

pub struct Scheduler {
    pub jobs: Vec<ScheduledJob>,
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

impl Scheduler {
    pub fn new() -> Self {
        Self { jobs: vec![] }
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
        self.jobs.iter().find(|j| now_unix >= j.aos_dt && now_unix <= j.los_dt)
    }
}
