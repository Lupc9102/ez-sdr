//! Unified error handling for the application.
//!
//! Replaces scattered unwrap/expect/if let Ok patterns with propagatable errors.

use std::sync::PoisonError;

#[derive(Debug, Clone)]
pub enum AppError {
    /// Mutex was poisoned (lock holder panicked)
    MutexPoisoned { context: String },

    /// I/O operation failed
    Io { context: String, message: String },

    /// Configuration invalid or missing
    Config { message: String },

    /// Source (SDR) operation failed
    Source { message: String },

    /// File not found or access denied
    FileNotFound { path: String },

    /// JSON serialization/deserialization failed
    Serialization { message: String },
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::MutexPoisoned { context } => {
                write!(f, "Mutex poisoned in {}", context)
            }
            AppError::Io { context, message } => {
                write!(f, "{}: {}", context, message)
            }
            AppError::Config { message } => write!(f, "Config error: {}", message),
            AppError::Source { message } => write!(f, "Source error: {}", message),
            AppError::FileNotFound { path } => write!(f, "File not found: {}", path),
            AppError::Serialization { message } => write!(f, "Serialization error: {}", message),
        }
    }
}

impl std::error::Error for AppError {}

/// Result type for app operations
pub type AppResult<T> = Result<T, AppError>;

/// Convert PoisonError to AppError
impl<T> From<PoisonError<T>> for AppError {
    fn from(_err: PoisonError<T>) -> Self {
        AppError::MutexPoisoned {
            context: "unknown".to_string(),
        }
    }
}
