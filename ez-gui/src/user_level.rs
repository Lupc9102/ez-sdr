use crate::app::Tab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserLevel {
    Beginner,
    Intermediate,
    Advanced,
    ClerkMaxwell,
}

impl UserLevel {
    pub fn from_str(s: &str) -> Self {
        match s {
            "beginner" => Self::Beginner,
            "intermediate" => Self::Intermediate,
            "advanced" => Self::Advanced,
            "clerk_maxwell" => Self::ClerkMaxwell,
            _ => Self::Beginner,
        }
    }

    pub fn to_str(self) -> &'static str {
        match self {
            Self::Beginner => "beginner",
            Self::Intermediate => "intermediate",
            Self::Advanced => "advanced",
            Self::ClerkMaxwell => "clerk_maxwell",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Beginner => "Beginner",
            Self::Intermediate => "Intermediate",
            Self::Advanced => "Advanced",
            Self::ClerkMaxwell => "Clerk_Maxwell",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Beginner => "Guided experience with AI assistance",
            Self::Intermediate => "Balanced UI, moderate hand-holding",
            Self::Advanced => "Full controls, minimal hints",
            Self::ClerkMaxwell => "Everything plus experimental features",
        }
    }

    pub fn show_advanced_controls(self) -> bool {
        matches!(self, Self::Advanced | Self::ClerkMaxwell)
    }

    pub fn simplify_layout(self) -> bool {
        matches!(self, Self::Beginner)
    }

    pub fn has_inline_expand(self) -> bool {
        matches!(self, Self::Beginner | Self::Intermediate)
    }

    pub fn levels() -> &'static [UserLevel; 4] {
        &[Self::Beginner, Self::Intermediate, Self::Advanced, Self::ClerkMaxwell]
    }
}

pub struct TutorialState {
    pub active: bool,
    pub level: UserLevel,
    pub step: usize,
    pub skip_confirm_phase: u8,
    pub highlight_target: Option<String>,
    pub tab_to_open: Option<Tab>,
    pub level_chosen: bool,
    pub asked_resume: bool,
    pub resume_response: Option<bool>,
}

impl TutorialState {
    pub fn new() -> Self {
        Self {
            active: true,
            level: UserLevel::Beginner,
            step: 0,
            skip_confirm_phase: 0,
            highlight_target: None,
            tab_to_open: None,
            level_chosen: false,
            asked_resume: false,
            resume_response: None,
        }
    }

    pub fn dismiss(&mut self) {
        self.active = false;
        self.highlight_target = None;
        self.tab_to_open = None;
        self.skip_confirm_phase = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_level_from_str_valid() {
        assert_eq!(UserLevel::from_str("beginner"), UserLevel::Beginner);
        assert_eq!(UserLevel::from_str("intermediate"), UserLevel::Intermediate);
        assert_eq!(UserLevel::from_str("advanced"), UserLevel::Advanced);
        assert_eq!(UserLevel::from_str("clerk_maxwell"), UserLevel::ClerkMaxwell);
    }

    #[test]
    fn user_level_from_str_invalid_defaults_to_beginner() {
        assert_eq!(UserLevel::from_str("unknown"), UserLevel::Beginner);
        assert_eq!(UserLevel::from_str(""), UserLevel::Beginner);
    }

    #[test]
    fn user_level_to_str_roundtrip() {
        for level in &[UserLevel::Beginner, UserLevel::Intermediate, UserLevel::Advanced, UserLevel::ClerkMaxwell] {
            assert_eq!(UserLevel::from_str(level.to_str()), *level);
        }
    }

    #[test]
    fn user_level_labels() {
        assert_eq!(UserLevel::Beginner.label(), "Beginner");
        assert_eq!(UserLevel::ClerkMaxwell.label(), "Clerk_Maxwell");
    }

    #[test]
    fn user_level_descriptions() {
        assert!(UserLevel::Beginner.description().contains("AI"));
        assert!(UserLevel::Advanced.description().contains("Full controls"));
    }

    #[test]
    fn show_advanced_controls() {
        assert!(!UserLevel::Beginner.show_advanced_controls());
        assert!(!UserLevel::Intermediate.show_advanced_controls());
        assert!(UserLevel::Advanced.show_advanced_controls());
        assert!(UserLevel::ClerkMaxwell.show_advanced_controls());
    }

    #[test]
    fn simplify_layout() {
        assert!(UserLevel::Beginner.simplify_layout());
        assert!(!UserLevel::Intermediate.simplify_layout());
        assert!(!UserLevel::Advanced.simplify_layout());
        assert!(!UserLevel::ClerkMaxwell.simplify_layout());
    }

    #[test]
    fn has_inline_expand() {
        assert!(UserLevel::Beginner.has_inline_expand());
        assert!(UserLevel::Intermediate.has_inline_expand());
        assert!(!UserLevel::Advanced.has_inline_expand());
        assert!(!UserLevel::ClerkMaxwell.has_inline_expand());
    }

    #[test]
    fn levels_returns_all_four() {
        let lvls = UserLevel::levels();
        assert_eq!(lvls.len(), 4);
        assert!(lvls.contains(&UserLevel::Beginner));
        assert!(lvls.contains(&UserLevel::Intermediate));
        assert!(lvls.contains(&UserLevel::Advanced));
        assert!(lvls.contains(&UserLevel::ClerkMaxwell));
    }

    #[test]
    fn tutorial_state_new() {
        let state = TutorialState::new();
        assert!(state.active);
        assert_eq!(state.level, UserLevel::Beginner);
        assert_eq!(state.step, 0);
        assert!(!state.level_chosen);
    }

    #[test]
    fn tutorial_state_dismiss() {
        let mut state = TutorialState::new();
        state.step = 5;
        state.level_chosen = true;
        state.dismiss();
        assert!(!state.active);
        assert!(state.highlight_target.is_none());
        assert_eq!(state.skip_confirm_phase, 0);
    }
}
