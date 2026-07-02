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
        self.jobs = passes
            .iter()
            .map(|p| ScheduledJob {
                satellite: p.satellite.clone(),
                aos: p.aos.clone(),
                los: p.los.clone(),
                frequency_hz: p.frequency_hz,
                aos_dt: p.aos_dt,
                los_dt: p.los_dt,
            })
            .collect();
    }

    /// Returns the first job whose AOS-LOS window contains `now_unix`, if any.
    pub fn active_job(&self, now_unix: f64) -> Option<&ScheduledJob> {
        if !self.auto_tune_enabled {
            return None;
        }
        self.jobs
            .iter()
            .find(|j| now_unix >= j.aos_dt && now_unix <= j.los_dt)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_scheduler_starts_empty() {
        let s = Scheduler::new();
        assert!(s.jobs.is_empty());
        assert!(s.auto_tune_enabled);
        assert!(s.custom_tasks.is_empty());
    }

    #[test]
    fn active_job_returns_none_when_disabled() {
        let mut s = Scheduler::new();
        s.auto_tune_enabled = false;
        s.jobs.push(ScheduledJob {
            satellite: "TestSat".into(),
            aos: String::new(),
            los: String::new(),
            frequency_hz: 100_000_000,
            aos_dt: 100.0,
            los_dt: 200.0,
        });
        assert!(s.active_job(150.0).is_none());
    }

    fn make_scheduler() -> Scheduler {
        let mut s = Scheduler::new();
        s.jobs.push(ScheduledJob {
            satellite: "TestSat".into(),
            aos: String::new(),
            los: String::new(),
            frequency_hz: 100_000_000,
            aos_dt: 100.0,
            los_dt: 200.0,
        });
        s
    }

    #[test]
    fn active_job_finds_job_in_window() {
        let s = make_scheduler();
        let job = s.active_job(150.0);
        assert!(job.is_some());
        assert_eq!(job.unwrap().frequency_hz, 100_000_000);
    }

    #[test]
    fn active_job_returns_none_outside_window() {
        let s = make_scheduler();
        assert!(s.active_job(50.0).is_none());
        assert!(s.active_job(250.0).is_none());
        assert!(s.active_job(100.0).is_some());
        assert!(s.active_job(200.0).is_some());
    }

    #[test]
    fn active_job_prefers_first_match() {
        let mut s = Scheduler::new();
        s.jobs.push(ScheduledJob {
            satellite: "SatA".into(),
            aos: String::new(),
            los: String::new(),
            frequency_hz: 100,
            aos_dt: 0.0,
            los_dt: 100.0,
        });
        s.jobs.push(ScheduledJob {
            satellite: "SatB".into(),
            aos: String::new(),
            los: String::new(),
            frequency_hz: 200,
            aos_dt: 50.0,
            los_dt: 150.0,
        });
        let job = s.active_job(75.0);
        assert!(job.is_some());
        assert_eq!(job.unwrap().satellite, "SatA"); // first in list wins
    }

    #[test]
    fn poll_custom_tasks_returns_pending() {
        let mut s = Scheduler::new();
        s.custom_tasks.push(CustomTask {
            label: "Tune".into(),
            frequency_hz: 433_000_000,
            at_unix: 1_000.0,
            fired: false,
        });
        let result = s.poll_custom_tasks(1_500.0);
        assert_eq!(result, Some(("Tune".into(), 433_000_000)));
    }

    #[test]
    fn poll_custom_tasks_skips_fired() {
        let mut s = Scheduler::new();
        s.custom_tasks.push(CustomTask {
            label: "Tune".into(),
            frequency_hz: 433_000_000,
            at_unix: 1_000.0,
            fired: true, // already fired
        });
        assert!(s.poll_custom_tasks(1_500.0).is_none());
    }

    #[test]
    fn poll_custom_tasks_not_yet_due() {
        let mut s = Scheduler::new();
        s.custom_tasks.push(CustomTask {
            label: "Tune".into(),
            frequency_hz: 433_000_000,
            at_unix: 2_000.0,
            fired: false,
        });
        assert!(s.poll_custom_tasks(1_000.0).is_none());
    }

    #[test]
    fn poll_custom_tasks_marks_as_fired() {
        let mut s = Scheduler::new();
        s.custom_tasks.push(CustomTask {
            label: "Tune".into(),
            frequency_hz: 433_000_000,
            at_unix: 1_000.0,
            fired: false,
        });
        s.poll_custom_tasks(1_500.0);
        // Should not fire again
        assert!(s.poll_custom_tasks(2_000.0).is_none());
    }

    #[test]
    fn update_from_passes_populates_jobs() {
        let mut s = Scheduler::new();
        let passes = vec![PassInfo {
            satellite: "Sat1".into(),
            aos: "10:00".into(),
            los: "10:30".into(),
            frequency_hz: 100,
            aos_dt: 100.0,
            los_dt: 200.0,
            max_elevation: 45.0,
        }];
        s.update_from_passes(&passes);
        assert_eq!(s.jobs.len(), 1);
        assert_eq!(s.jobs[0].satellite, "Sat1");
    }
}
