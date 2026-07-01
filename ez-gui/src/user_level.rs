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

    pub fn show_experimental_features(self) -> bool {
        matches!(self, Self::ClerkMaxwell)
    }

    pub fn hint_verbosity(self) -> HintLevel {
        match self {
            Self::Beginner => HintLevel::Verbose,
            Self::Intermediate => HintLevel::Normal,
            Self::Advanced => HintLevel::Minimal,
            Self::ClerkMaxwell => HintLevel::Off,
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintLevel {
    Verbose,
    Normal,
    Minimal,
    Off,
}

pub fn user_level_hint(level: UserLevel, verbose: &'static str, normal: &'static str, minimal: &'static str) -> &'static str {
    match level.hint_verbosity() {
        HintLevel::Verbose => verbose,
        HintLevel::Normal => normal,
        HintLevel::Minimal => minimal,
        HintLevel::Off => "",
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

    pub fn is_in_tutorial(&self) -> bool {
        self.active && !self.asked_resume
    }

    pub fn dismiss(&mut self) {
        self.active = false;
        self.highlight_target = None;
        self.tab_to_open = None;
        self.skip_confirm_phase = 0;
    }
}
