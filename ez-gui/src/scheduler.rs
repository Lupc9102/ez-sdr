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
        }).collect();
    }
}
