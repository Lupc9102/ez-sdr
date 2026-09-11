/// Status bar management extracted from CentralApp.
///
/// This module handles all status messages, flashes, and progress indicators,
/// removing ~200 lines from the app.rs god object.
use std::time::{Duration, Instant};

/// Status message with severity and expiry.
#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub severity: StatusSeverity,
    pub created_at: Instant,
    pub duration: Duration,
}

/// Severity level for status messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusSeverity {
    Info,
    Success,
    Warning,
    Error,
}

impl StatusSeverity {
    /// Get emoji icon for severity.
    pub fn icon(&self) -> &'static str {
        match self {
            StatusSeverity::Info => "ℹ️",
            StatusSeverity::Success => "✅",
            StatusSeverity::Warning => "⚠️",
            StatusSeverity::Error => "❌",
        }
    }

    /// Get egui color for severity.
    pub fn color(&self) -> egui::Color32 {
        match self {
            StatusSeverity::Info => egui::Color32::from_rgb(100, 149, 237),
            StatusSeverity::Success => egui::Color32::from_rgb(50, 205, 50),
            StatusSeverity::Warning => egui::Color32::from_rgb(255, 165, 0),
            StatusSeverity::Error => egui::Color32::from_rgb(220, 20, 60),
        }
    }
}

/// Status bar manager.
pub struct StatusBar {
    current_message: Option<StatusMessage>,
    message_history: Vec<StatusMessage>,
    max_history: usize,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            current_message: None,
            message_history: Vec::with_capacity(50),
            max_history: 50,
        }
    }

    /// Show a status message.
    pub fn show(&mut self, text: String, severity: StatusSeverity, duration: Duration) {
        let message = StatusMessage {
            text,
            severity,
            created_at: Instant::now(),
            duration,
        };

        // Add to history
        self.message_history.push(message.clone());
        if self.message_history.len() > self.max_history {
            self.message_history.remove(0);
        }

        self.current_message = Some(message);
    }

    /// Show info message (default 3 seconds).
    pub fn info(&mut self, text: String) {
        self.show(text, StatusSeverity::Info, Duration::from_secs(3));
    }

    /// Show success message (default 2 seconds).
    pub fn success(&mut self, text: String) {
        self.show(text, StatusSeverity::Success, Duration::from_secs(2));
    }

    /// Show warning message (default 5 seconds).
    pub fn warning(&mut self, text: String) {
        self.show(text, StatusSeverity::Warning, Duration::from_secs(5));
    }

    /// Show error message (default 10 seconds).
    pub fn error(&mut self, text: String) {
        self.show(text, StatusSeverity::Error, Duration::from_secs(10));
    }

    /// Update status bar (check expiry).
    pub fn update(&mut self) {
        if let Some(msg) = &self.current_message {
            if msg.created_at.elapsed() > msg.duration {
                self.current_message = None;
            }
        }
    }

    /// Get current message if any.
    pub fn current(&self) -> Option<&StatusMessage> {
        self.current_message.as_ref()
    }

    /// Get message history.
    pub fn history(&self) -> &[StatusMessage] {
        &self.message_history
    }

    /// Clear current message.
    pub fn clear(&mut self) {
        self.current_message = None;
    }

    /// Render status bar UI.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.update();

        if let Some(msg) = self.current() {
            ui.horizontal(|ui| {
                ui.label(msg.severity.icon());
                ui.colored_label(msg.severity.color(), &msg.text);

                // Progress bar for remaining time
                let elapsed = msg.created_at.elapsed().as_secs_f32();
                let total = msg.duration.as_secs_f32();
                let remaining = (1.0 - (elapsed / total)).max(0.0);

                ui.add(
                    egui::ProgressBar::new(remaining)
                        .desired_width(50.0)
                        .show_percentage(),
                );
            });
        }
    }

    /// Render status strip at the bottom of the window
    pub fn render_strip(
        &self,
        ui: &mut egui::Ui,
        shared: &std::sync::Arc<std::sync::Mutex<crate::app::SharedState>>,
    ) {
        use crate::sdr_panel::DemodMode;
        let Some(state) = shared.try_lock().ok() else {
            return;
        };
        let freq_mhz = state.source.frequency_hz as f64 / 1e6;
        let mode = match state.demod_mode {
            DemodMode::Auto => format!(
                "Auto → {}",
                DemodMode::for_frequency(state.source.frequency_hz).label()
            ),
            m => m.label().to_string(),
        };
        let level = state.spectrum.signal_level();
        let (word, color) = if level > -30.0 {
            ("Strong ✓", egui::Color32::from_rgb(60, 220, 100))
        } else if level > -60.0 {
            ("Weak", egui::Color32::from_rgb(230, 200, 60))
        } else if level > -80.0 {
            ("Quiet", egui::Color32::from_rgb(180, 180, 190))
        } else {
            ("Silent", egui::Color32::from_rgb(120, 120, 130))
        };
        ui.horizontal(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(52, 152, 219),
                format!("● {freq_mhz:.3} MHz"),
            );
            ui.label(mode);
            ui.colored_label(color, word);
            if state.recording {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), "● REC");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(msg) = self.current() {
                    if msg.created_at.elapsed() < msg.duration {
                        ui.label(
                            egui::RichText::new(&msg.text)
                                .small()
                                .color(msg.severity.color()),
                        );
                    }
                }
            });
        });
    }
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_bar_creation() {
        let bar = StatusBar::new();
        assert!(bar.current().is_none());
        assert_eq!(bar.history().len(), 0);
    }

    #[test]
    fn status_bar_show_message() {
        let mut bar = StatusBar::new();
        bar.info("Test message".to_string());
        assert!(bar.current().is_some());
        assert_eq!(bar.history().len(), 1);
    }

    #[test]
    fn status_bar_expiry() {
        let mut bar = StatusBar::new();
        bar.show(
            "Test".to_string(),
            StatusSeverity::Info,
            Duration::from_millis(1),
        );
        std::thread::sleep(Duration::from_millis(10));
        bar.update();
        assert!(bar.current().is_none());
    }

    #[test]
    fn status_bar_history_bounded() {
        let mut bar = StatusBar::new();
        bar.max_history = 5;

        for i in 0..10 {
            bar.info(format!("Message {}", i));
        }

        assert_eq!(bar.history().len(), 5);
    }
}
