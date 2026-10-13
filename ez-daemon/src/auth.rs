//! Shared bearer-token authentication for the daemon transports.

use std::sync::Arc;
use subtle::ConstantTimeEq;

/// An optional daemon-wide secret. The stored bytes are never formatted or logged.
#[derive(Clone, Default)]
pub struct Authenticator(Option<Arc<[u8]>>);

impl Authenticator {
    pub fn new(token: Option<String>) -> anyhow::Result<Self> {
        match token {
            Some(token) if token.is_empty() => {
                anyhow::bail!("authentication token must not be empty")
            }
            Some(token) => Ok(Self(Some(Arc::from(token.into_bytes())))),
            None => Ok(Self::default()),
        }
    }

    #[must_use]
    pub fn is_required(&self) -> bool {
        self.0.is_some()
    }

    /// Constant-time comparison for equal-length tokens. Tokens should be random,
    /// fixed-length secrets (at least 32 bytes).
    #[must_use]
    pub fn accepts(&self, candidate: &str) -> bool {
        let Some(expected) = &self.0 else {
            return true;
        };
        let candidate = candidate.as_bytes();
        expected.as_ref().ct_eq(candidate).into()
    }
}

#[cfg(test)]
mod tests {
    use super::Authenticator;

    #[test]
    fn optional_auth_accepts_any_token_but_configured_auth_requires_exact_secret() {
        assert!(Authenticator::default().accepts("anything"));
        let auth = Authenticator::new(Some("a long random test token".into())).unwrap();
        assert!(auth.is_required());
        assert!(auth.accepts("a long random test token"));
        assert!(!auth.accepts("a long random test tokeN"));
        assert!(!auth.accepts("short"));
        assert!(Authenticator::new(Some(String::new())).is_err());
    }
}
