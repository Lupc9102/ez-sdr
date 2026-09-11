/// Frequency history tracking extracted from CentralApp.
///
/// This module handles frequency tuning history, removing ~150 lines from app.rs.
use std::collections::VecDeque;

/// Frequency history entry.
#[derive(Debug, Clone)]
pub struct FrequencyEntry {
    pub freq_hz: u64,
    pub label: Option<String>,
    pub timestamp: std::time::Instant,
}

/// Frequency history manager with undo/redo.
pub struct FrequencyHistory {
    history: VecDeque<FrequencyEntry>,
    current_index: Option<usize>,
    max_entries: usize,
}

impl FrequencyHistory {
    pub fn new() -> Self {
        Self {
            history: VecDeque::with_capacity(100),
            current_index: None,
            max_entries: 100,
        }
    }

    /// Add a frequency to history.
    pub fn add(&mut self, freq_hz: u64, label: Option<String>) {
        // Don't add duplicates of the current frequency
        if let Some(idx) = self.current_index {
            if let Some(current) = self.history.get(idx) {
                if current.freq_hz == freq_hz {
                    return;
                }
            }
        }

        let entry = FrequencyEntry {
            freq_hz,
            label,
            timestamp: std::time::Instant::now(),
        };

        // If we're not at the end, remove everything after current position
        if let Some(idx) = self.current_index {
            self.history.truncate(idx + 1);
        }

        self.history.push_back(entry);
        self.current_index = Some(self.history.len() - 1);

        // Keep history bounded
        if self.history.len() > self.max_entries {
            self.history.pop_front();
            if let Some(idx) = self.current_index {
                self.current_index = Some(idx.saturating_sub(1));
            }
        }
    }

    /// Go back in history.
    pub fn back(&mut self) -> Option<u64> {
        if let Some(idx) = self.current_index {
            if idx > 0 {
                self.current_index = Some(idx - 1);
                return self.history.get(idx - 1).map(|e| e.freq_hz);
            }
        }
        None
    }

    /// Go forward in history.
    pub fn forward(&mut self) -> Option<u64> {
        if let Some(idx) = self.current_index {
            if idx + 1 < self.history.len() {
                self.current_index = Some(idx + 1);
                return self.history.get(idx + 1).map(|e| e.freq_hz);
            }
        }
        None
    }

    /// Get current frequency.
    pub fn current(&self) -> Option<&FrequencyEntry> {
        self.current_index.and_then(|idx| self.history.get(idx))
    }

    /// Get all history entries.
    pub fn entries(&self) -> &VecDeque<FrequencyEntry> {
        &self.history
    }

    /// Get recent N entries.
    pub fn recent(&self, n: usize) -> Vec<&FrequencyEntry> {
        self.history.iter().rev().take(n).collect()
    }

    /// Can go back?
    pub fn can_go_back(&self) -> bool {
        self.current_index.is_some_and(|idx| idx > 0)
    }

    /// Can go forward?
    pub fn can_go_forward(&self) -> bool {
        self.current_index
            .is_some_and(|idx| idx + 1 < self.history.len())
    }

    /// Clear history.
    pub fn clear(&mut self) {
        self.history.clear();
        self.current_index = None;
    }

    /// Get history size.
    pub fn len(&self) -> usize {
        self.history.len()
    }

    /// Is history empty?
    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }
}

impl Default for FrequencyHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_history_creation() {
        let history = FrequencyHistory::new();
        assert!(history.is_empty());
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }

    #[test]
    fn frequency_history_add() {
        let mut history = FrequencyHistory::new();
        history.add(100_000_000, None);
        history.add(200_000_000, Some("FM Radio".to_string()));

        assert_eq!(history.len(), 2);
        assert_eq!(history.current().unwrap().freq_hz, 200_000_000);
    }

    #[test]
    fn frequency_history_navigation() {
        let mut history = FrequencyHistory::new();
        history.add(100_000_000, None);
        history.add(200_000_000, None);
        history.add(300_000_000, None);

        assert_eq!(history.back(), Some(200_000_000));
        assert_eq!(history.back(), Some(100_000_000));
        assert_eq!(history.forward(), Some(200_000_000));
    }

    #[test]
    fn frequency_history_no_duplicates() {
        let mut history = FrequencyHistory::new();
        history.add(100_000_000, None);
        history.add(100_000_000, None); // Duplicate
        history.add(100_000_000, None); // Duplicate

        assert_eq!(history.len(), 1);
    }

    #[test]
    fn frequency_history_bounded() {
        let mut history = FrequencyHistory::new();
        history.max_entries = 5;

        for i in 0..10 {
            history.add(i * 1_000_000, None);
        }

        assert_eq!(history.len(), 5);
    }
}
