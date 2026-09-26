use crate::tle_engine::PassInfo;
use chrono::{DateTime, Datelike, Local, TimeZone};
use std::sync::{Arc, Mutex};

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

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
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
    #[must_use]
    pub fn active_job(&self, now_unix: f64) -> Option<&ScheduledJob> {
        if !self.auto_tune_enabled {
            return None;
        }
        self.jobs
            .iter()
            .find(|j| now_unix >= j.aos_dt && now_unix <= j.los_dt)
    }

    /// Desktop Meteor work is offline; it must not retune a live radio. Search
    /// eligible jobs first so a concurrent Meteor pass cannot hide another job.
    pub fn active_radio_job(&self, now_unix: f64) -> Option<&ScheduledJob> {
        if !self.auto_tune_enabled {
            return None;
        }
        self.jobs.iter().find(|j| {
            !j.satellite.to_ascii_lowercase().contains("meteor")
                && now_unix >= j.aos_dt
                && now_unix <= j.los_dt
        })
    }

    /// Check if any custom task should fire now. Returns frequency if fired.
    #[must_use]
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

pub struct SchedulerPanel {
    pub new_task_label: String,
    pub new_task_freq_mhz: String,
    pub new_task_time: String,
    pub new_task_error: String,
}

impl Default for SchedulerPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl SchedulerPanel {
    pub fn new() -> Self {
        Self {
            new_task_label: String::new(),
            new_task_freq_mhz: String::new(),
            new_task_time: String::new(),
            new_task_error: String::new(),
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        shared: &Arc<Mutex<crate::app::SharedState>>,
        snapshot: &Option<crate::app::SharedSnapshot>,
    ) {
        let (jobs, custom_tasks, auto_tune) = match snapshot {
            Some(s) => (s.jobs.clone(), s.custom_tasks.clone(), s.auto_tune_enabled),
            None => return,
        };
        ui.heading("Scheduler");

        // Next event countdown summary
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let next_pass = jobs.first();
        let next_task = custom_tasks
            .iter()
            .filter(|t| !t.fired && t.at_unix > now_unix)
            .min_by(|a, b| {
                a.at_unix
                    .partial_cmp(&b.at_unix)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some(job) = next_pass {
            ui.colored_label(
                egui::Color32::from_rgb(100, 180, 255),
                format!("Next pass: {} at {} ({})", job.satellite, job.aos, job.los),
            )
            .on_hover_text(
                "Next satellite pass scheduled. Enable Auto-tune below to tune automatically.",
            );
        } else if next_task.is_none() {
            ui.colored_label(egui::Color32::GRAY, "No upcoming events. Add a custom task below or update TLE data in the Satellite tab.");
        }
        if let Some(task) = next_task {
            let secs = (task.at_unix - now_unix).max(0.0) as u64;
            let countdown = if secs < 60 {
                format!("{secs}s")
            } else {
                format!("{}m {}s", secs / 60, secs % 60)
            };
            ui.colored_label(
                egui::Color32::from_rgb(241, 196, 15),
                format!(
                    "Next task: '{}' at {:.3} MHz — fires in {}",
                    task.label,
                    task.frequency_hz as f64 / 1e6,
                    countdown
                ),
            );
        }
        ui.add_space(4.0);

        // Auto-tune toggle
        let mut auto = auto_tune;
        if ui.checkbox(&mut auto, "Auto-tune to satellite passes")
            .on_hover_text("When enabled, the SDR automatically tunes to the frequency of any satellite currently overhead.")
            .changed()
        {
            if let Ok(mut state) = shared.try_lock() {
                state.scheduler.auto_tune_enabled = auto;
            }
        }

        ui.separator();
        ui.label(egui::RichText::new("Upcoming Satellite Passes").strong());

        // Visual timeline: 24-hour bar with pass blocks
        {
            let tl_h = 28.0;
            let (tl_rect, tl_resp) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), tl_h), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(tl_rect, 2.0, egui::Color32::from_rgb(8, 8, 18));

            let today_start = (now_unix as u64).saturating_sub((now_unix as u64) % 86400) as f64;
            let day_span = 86400.0f64;

            // Hour marks
            for h in 0..=24 {
                let frac = h as f32 / 24.0;
                let x = tl_rect.left() + frac * tl_rect.width();
                painter.line_segment(
                    [
                        egui::pos2(x, tl_rect.top()),
                        egui::pos2(x, tl_rect.bottom()),
                    ],
                    egui::Stroke::new(0.4, egui::Color32::from_rgba_premultiplied(60, 60, 80, 100)),
                );
                if h % 4 == 0 {
                    painter.text(
                        egui::pos2(x + 2.0, tl_rect.top() + 1.0),
                        egui::Align2::LEFT_TOP,
                        format!("{h:02}"),
                        egui::FontId::proportional(7.0),
                        egui::Color32::from_gray(90),
                    );
                }
            }

            // Pass blocks (color-coded by satellite index)
            let pass_colors = [
                egui::Color32::from_rgb(52, 152, 219),
                egui::Color32::from_rgb(46, 204, 113),
                egui::Color32::from_rgb(241, 196, 15),
                egui::Color32::from_rgb(155, 89, 182),
                egui::Color32::from_rgb(231, 76, 60),
            ];
            let hover_x = tl_resp.hover_pos().map(|p| p.x);
            let mut hovered_job: Option<&ScheduledJob> = None;
            for (i, job) in jobs.iter().enumerate() {
                let aos_frac = ((job.aos_dt - today_start) / day_span).clamp(0.0, 1.0) as f32;
                let los_frac = ((job.los_dt - today_start) / day_span).clamp(0.0, 1.0) as f32;
                if los_frac <= aos_frac {
                    continue;
                }
                let x1 = tl_rect.left() + aos_frac * tl_rect.width();
                let x2 = tl_rect.left() + los_frac * tl_rect.width();
                let col = pass_colors[i % pass_colors.len()];
                let block = egui::Rect::from_x_y_ranges(
                    x1..=x2,
                    (tl_rect.top() + 4.0)..=(tl_rect.bottom() - 4.0),
                );
                painter.rect_filled(block, 1.0, col.linear_multiply(0.7));
                painter.rect_filled(
                    egui::Rect::from_x_y_ranges(
                        x1..=(x1 + 1.0),
                        (tl_rect.top() + 4.0)..=(tl_rect.bottom() - 4.0),
                    ),
                    0.0,
                    col,
                );
                painter.rect_filled(
                    egui::Rect::from_x_y_ranges(
                        (x2 - 1.0)..=x2,
                        (tl_rect.top() + 4.0)..=(tl_rect.bottom() - 4.0),
                    ),
                    0.0,
                    col,
                );
                if x2 - x1 > 16.0 {
                    painter.text(
                        egui::pos2(f32::midpoint(x1, x2), tl_rect.center().y),
                        egui::Align2::CENTER_CENTER,
                        &job.satellite,
                        egui::FontId::proportional(7.0),
                        egui::Color32::WHITE,
                    );
                }
                if let Some(hx) = hover_x {
                    if hx >= x1 && hx <= x2 {
                        hovered_job = Some(job);
                    }
                }
            }

            // Current time marker
            let now_frac = ((now_unix - today_start) / day_span).clamp(0.0, 1.0) as f32;
            let now_x = tl_rect.left() + now_frac * tl_rect.width();
            painter.line_segment(
                [
                    egui::pos2(now_x, tl_rect.top()),
                    egui::pos2(now_x, tl_rect.bottom()),
                ],
                egui::Stroke::new(1.2, egui::Color32::from_rgb(255, 80, 80)),
            );

            // Tooltip for hovered pass
            if let Some(job) = hovered_job {
                let tip = format!(
                    "{}\nAOS: {}  LOS: {}\n{:.3} MHz",
                    job.satellite,
                    job.aos,
                    job.los,
                    job.frequency_hz as f64 / 1e6
                );
                tl_resp.on_hover_text(tip);
            }
        }

        if jobs.is_empty() {
            ui.label(
                egui::RichText::new("No upcoming passes (update TLE data in Satellite tab).")
                    .color(egui::Color32::GRAY),
            );
        } else {
            egui::ScrollArea::vertical().max_height(180.0).id_salt("sched_sat_scroll").show(ui, |ui| {
                egui::Grid::new("sched_grid").num_columns(5).striped(true).show(ui, |ui| {
                    ui.label(egui::RichText::new("Satellite").strong()).on_hover_text("Satellite name from TLE catalogue.");
                    ui.label(egui::RichText::new("AOS").strong()).on_hover_text("Acquisition of Signal — time the satellite rises above the horizon.");
                    ui.label(egui::RichText::new("LOS").strong()).on_hover_text("Loss of Signal — time the satellite drops below the horizon.");
                    ui.label(egui::RichText::new("Freq").strong()).on_hover_text("Downlink frequency to tune to.");
                    ui.label(egui::RichText::new("Tune").strong());
                    ui.end_row();
                    for job in &jobs {
                        ui.label(&job.satellite);
                        ui.label(&job.aos);
                        ui.label(&job.los);
                        ui.monospace(format!("{:.3} MHz", job.frequency_hz as f64 / 1e6));
                        if ui.small_button("📡 Tune").on_hover_text("Tune SDR to this satellite's frequency now.").clicked() {
                            if let Ok(mut state) = shared.try_lock() {
                                state.source.frequency_hz = job.frequency_hz;
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        }

        ui.separator();
        ui.label(egui::RichText::new("Custom Timed Tasks").strong())
            .on_hover_text(
                "Schedule a one-shot frequency tune at a specific time (HH:MM:SS today).",
            );

        // Add task form
        ui.group(|ui| {
            egui::Grid::new("task_form_grid")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Label:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_task_label)
                            .desired_width(150.0)
                            .hint_text("e.g. NOAA pass"),
                    );
                    ui.end_row();
                    ui.label("Freq (MHz):");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_task_freq_mhz)
                            .desired_width(100.0)
                            .hint_text("137.620"),
                    );
                    ui.end_row();
                    ui.label("Time (HH:MM):");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_task_time)
                            .desired_width(80.0)
                            .hint_text("14:30"),
                    );
                    ui.end_row();
                });
            if !self.new_task_error.is_empty() {
                ui.colored_label(egui::Color32::RED, self.new_task_error.as_str());
            }
            if ui.button("+ Add Task").clicked() {
                let label = self.new_task_label.trim().to_string();
                let freq_res = self.new_task_freq_mhz.trim().parse::<f64>();
                let time_res = parse_hhmm_today(self.new_task_time.trim());
                match (freq_res, time_res) {
                    (Ok(mhz), Some(at_unix)) if mhz > 0.0 => {
                        if let Ok(mut state) = shared.try_lock() {
                            state.scheduler.custom_tasks.push(CustomTask {
                                label: if label.is_empty() {
                                    format!("{mhz:.3} MHz")
                                } else {
                                    label
                                },
                                frequency_hz: (mhz * 1e6) as u64,
                                at_unix,
                                fired: false,
                            });
                        }
                        self.new_task_label.clear();
                        self.new_task_freq_mhz.clear();
                        self.new_task_time.clear();
                        self.new_task_error.clear();
                    }
                    (Err(_), _) => self.new_task_error = "Invalid frequency.".to_string(),
                    (_, None) => {
                        self.new_task_error = "Invalid time — use HH:MM format.".to_string();
                    }
                    _ => self.new_task_error = "Frequency must be > 0.".to_string(),
                }
            }
        });

        if !custom_tasks.is_empty() {
            egui::Grid::new("custom_tasks_grid")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Label").strong());
                    ui.label(egui::RichText::new("Freq").strong());
                    ui.label(egui::RichText::new("In").strong());
                    ui.label(egui::RichText::new("Del").strong());
                    ui.end_row();
                    let mut remove_idx = None;
                    for (i, task) in custom_tasks.iter().enumerate() {
                        let remaining = task.at_unix - now_unix;
                        let color = if task.fired {
                            egui::Color32::GRAY
                        } else if remaining < 0.0 {
                            egui::Color32::RED
                        } else {
                            egui::Color32::WHITE
                        };
                        ui.colored_label(color, &task.label);
                        ui.monospace(format!("{:.3} MHz", task.frequency_hz as f64 / 1e6));
                        let in_str = if task.fired {
                            "fired".to_string()
                        } else if remaining < 0.0 {
                            "overdue".to_string()
                        } else if remaining < 60.0 {
                            format!("{remaining:.0}s")
                        } else {
                            format!("{:.0}m", remaining / 60.0)
                        };
                        ui.label(in_str);
                        if ui.small_button("✕").clicked() {
                            remove_idx = Some(i);
                        }
                        ui.end_row();
                    }
                    if let Some(idx) = remove_idx {
                        if let Ok(mut state) = shared.try_lock() {
                            if idx < state.scheduler.custom_tasks.len() {
                                state.scheduler.custom_tasks.remove(idx);
                            }
                        }
                    }
                });
        }
    }
}

/// Parse "HH:MM" or "HH:MM:SS" as a unix timestamp for today in local time.
pub fn parse_hhmm_today(s: &str) -> Option<f64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let mut target = parse_hhmm_today_at(s, now)?;
    // If the requested wall-clock time already passed today, roll over to
    // tomorrow so a task scheduled for tonight doesn't fire immediately.
    // `<=` so a task set for exactly now fires tomorrow, not instantly.
    if target <= now as f64 {
        target += 86400.0;
    }
    Some(target)
}

/// Parse "HH:MM" or "HH:MM:SS" as a unix timestamp for today in local time,
/// given an arbitrary `now_unix` (seconds since epoch) as the reference "now".
pub fn parse_hhmm_today_at(s: &str, now_unix: u64) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let sec: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    if h > 23 || m > 59 || sec > 59 {
        return None;
    }

    // Convert UTC timestamp to local time to find today's local midnight
    let utc_dt = DateTime::from_timestamp(now_unix as i64, 0)?;
    let local_dt = utc_dt.with_timezone(&Local);
    let midnight = Local
        .with_ymd_and_hms(local_dt.year(), local_dt.month(), local_dt.day(), 0, 0, 0)
        .single()?;

    let target = midnight
        + chrono::Duration::hours(h as i64)
        + chrono::Duration::minutes(m as i64)
        + chrono::Duration::seconds(sec as i64);

    Some(target.timestamp() as f64)
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
        assert_eq!(
            job.expect("active_job should find scheduled job in window")
                .frequency_hz,
            100_000_000
        );
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
    fn desktop_radio_jobs_skip_meteor_without_hiding_concurrent_passes() {
        let mut scheduler = make_scheduler();
        scheduler.jobs[0].satellite = "Meteor-M2-3".into();
        assert!(scheduler.active_radio_job(150.0).is_none());
        scheduler.jobs.push(ScheduledJob {
            satellite: "ISS".into(),
            frequency_hz: 145_800_000,
            aos: String::new(),
            los: String::new(),
            aos_dt: 100.0,
            los_dt: 200.0,
        });
        assert_eq!(scheduler.active_radio_job(150.0).unwrap().satellite, "ISS");
        scheduler.auto_tune_enabled = false;
        assert!(scheduler.active_radio_job(150.0).is_none());
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
        assert_eq!(
            job.expect("active_job should return first matching job")
                .satellite,
            "SatA"
        ); // first in list wins
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
        let _ = s.poll_custom_tasks(1_500.0);
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

    #[test]
    fn update_from_passes_empty_list_clears_jobs() {
        let mut s = make_scheduler();
        assert_eq!(s.jobs.len(), 1);
        s.update_from_passes(&[]);
        assert!(s.jobs.is_empty());
    }

    #[test]
    fn poll_custom_tasks_multiple_due_at_same_time() {
        let mut s = Scheduler::new();
        s.custom_tasks.push(CustomTask {
            label: "First".into(),
            frequency_hz: 100_000,
            at_unix: 1000.0,
            fired: false,
        });
        s.custom_tasks.push(CustomTask {
            label: "Second".into(),
            frequency_hz: 200_000,
            at_unix: 1000.0,
            fired: false,
        });
        let r1 = s.poll_custom_tasks(1500.0);
        assert_eq!(r1, Some(("First".into(), 100_000)));
        let r2 = s.poll_custom_tasks(1500.0);
        assert_eq!(r2, Some(("Second".into(), 200_000)));
        assert!(s.poll_custom_tasks(1500.0).is_none());
    }

    #[test]
    fn poll_custom_tasks_zero_delay() {
        let mut s = Scheduler::new();
        s.custom_tasks.push(CustomTask {
            label: "Immediate".into(),
            frequency_hz: 433_000_000,
            at_unix: 0.0,
            fired: false,
        });
        let result = s.poll_custom_tasks(1.0);
        assert_eq!(result, Some(("Immediate".into(), 433_000_000)));
    }

    #[test]
    fn poll_custom_tasks_very_large_delay() {
        let mut s = Scheduler::new();
        s.custom_tasks.push(CustomTask {
            label: "FarFuture".into(),
            frequency_hz: 433_000_000,
            at_unix: f64::MAX,
            fired: false,
        });
        assert!(s.poll_custom_tasks(1_000_000_000.0).is_none());
    }

    #[test]
    fn scheduled_job_edge_times_midnight() {
        let mut s = Scheduler::new();
        s.jobs.push(ScheduledJob {
            satellite: "MidnightSat".into(),
            aos: String::new(),
            los: String::new(),
            frequency_hz: 100,
            aos_dt: 0.0,
            los_dt: 0.0,
        });
        assert!(s.active_job(0.0).is_some());
        assert!(s.active_job(-0.001).is_none());
    }

    #[test]
    fn scheduled_job_edge_times_noon() {
        let mut s = Scheduler::new();
        s.jobs.push(ScheduledJob {
            satellite: "NoonSat".into(),
            aos: String::new(),
            los: String::new(),
            frequency_hz: 200,
            aos_dt: 43200.0,
            los_dt: 44400.0,
        });
        assert!(s.active_job(43200.0).is_some());
        assert!(s.active_job(43800.0).is_some());
        assert!(s.active_job(44400.0).is_some());
        assert!(s.active_job(43199.0).is_none());
        assert!(s.active_job(44401.0).is_none());
    }

    #[test]
    fn scheduled_job_edge_times_2359() {
        let mut s = Scheduler::new();
        s.jobs.push(ScheduledJob {
            satellite: "LateSat".into(),
            aos: String::new(),
            los: String::new(),
            frequency_hz: 300,
            aos_dt: 86340.0,
            los_dt: 86400.0,
        });
        assert!(s.active_job(86340.0).is_some());
        assert!(s.active_job(86400.0).is_some());
        assert!(s.active_job(86339.0).is_none());
    }

    #[test]
    fn test_midnight() {
        let ts = 1705320000u64;
        let result =
            parse_hhmm_today_at("00:00", ts).expect("00:00 should parse as local midnight");
        let local_midnight = Local
            .with_ymd_and_hms(2024, 1, 15, 0, 0, 0)
            .single()
            .expect("2024-01-15 00:00:00 should be a valid local time");
        assert!((result - local_midnight.timestamp() as f64).abs() < 1.0);
    }

    #[test]
    fn test_noon() {
        let ts = 1705320000u64;
        let midnight = parse_hhmm_today_at("00:00", ts)
            .expect("00:00 should parse as local midnight for noon test");
        let noon = parse_hhmm_today_at("12:00", ts).expect("12:00 should parse as local noon");
        assert!((noon - midnight - 12.0 * 3600.0).abs() < 1.0);
    }

    #[test]
    fn test_dst_safe() {
        let ts = 1711843200u64;
        let result = parse_hhmm_today_at("00:00", ts);
        assert!(result.is_some());
    }

    #[test]
    fn test_invalid_formats() {
        assert!(parse_hhmm_today_at("", 0).is_none());
        assert!(parse_hhmm_today_at("12", 0).is_none());
        assert!(parse_hhmm_today_at("abc", 0).is_none());
        assert!(parse_hhmm_today_at("25:00", 0).is_none());
        assert!(parse_hhmm_today_at("12:60", 0).is_none());
        assert!(parse_hhmm_today_at("12:00:60", 0).is_none());
    }

    #[test]
    fn test_different_reference_days() {
        let ts1 = 1704067200u64;
        let ts2 = 1704153600u64;
        let r1 = parse_hhmm_today_at("09:00", ts1).expect("09:00 should parse for ts1");
        let r2 = parse_hhmm_today_at("09:00", ts2).expect("09:00 should parse for ts2");
        assert!((r2 - r1 - 86400.0).abs() < 2.0);
    }

    #[test]
    fn test_seconds_field() {
        let ts = 1705320000u64;
        let r1 = parse_hhmm_today_at("01:02:03", ts).expect("01:02:03 should parse with seconds");
        let r2 =
            parse_hhmm_today_at("01:02:00", ts).expect("01:02:00 should parse without seconds");
        assert_eq!((r1 - r2).round() as i64, 3);
    }

    #[test]
    fn parse_hhmm_today_always_future() {
        // Issue 46: a task scheduled for a wall-clock time already past today
        // must roll to tomorrow, never fire immediately.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_secs_f64();
        for s in ["00:00", "00:00:00", "12:00", "23:59"] {
            let t = parse_hhmm_today(s).expect("valid HH:MM should parse");
            assert!(
                t > now,
                "{s} should resolve in the future (got {t}, now {now})"
            );
            assert!(
                t < now + 86400.0 + 1.0,
                "{s} should be within 24h (got {t}, now {now})"
            );
        }
        assert!(parse_hhmm_today("25:00").is_none());
        assert!(parse_hhmm_today("").is_none());
    }
}
