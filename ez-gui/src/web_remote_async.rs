//! Deprecated compatibility facade with a heartbeat-only task.
//!
//! This module does not implement an HTTP remote. Use [`crate::web_remote::WebRemote`]
//! for the desktop's actual loopback control server.
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TokioMutex;
use tokio::time::sleep;

/// Heartbeat-only compatibility type; it does not serve HTTP or control SDR state.
#[deprecated(note = "heartbeat-only stub; use web_remote::WebRemote for the HTTP remote")]
pub struct WebRemoteAsync {
    last_heartbeat: Arc<TokioMutex<Instant>>,
    running: Arc<TokioMutex<bool>>,
}

#[allow(deprecated)]
impl WebRemoteAsync {
    pub fn new() -> Self {
        Self {
            last_heartbeat: Arc::new(TokioMutex::new(Instant::now())),
            running: Arc::new(TokioMutex::new(false)),
        }
    }

    /// Start the async background task (non-blocking).
    pub fn start_background_task(&self) {
        let last_heartbeat = Arc::clone(&self.last_heartbeat);
        let running = Arc::clone(&self.running);

        tokio::spawn(async move {
            *running.lock().await = true;

            while *running.lock().await {
                // Update heartbeat
                *last_heartbeat.lock().await = Instant::now();

                // Sleep without blocking UI thread
                sleep(Duration::from_millis(200)).await;

                // Do background work here (connection monitoring, etc.)
            }
        });
    }

    /// Stop the background task.
    pub async fn stop(&self) {
        *self.running.lock().await = false;
    }

    /// Check if background task is healthy.
    pub async fn is_healthy(&self) -> bool {
        let last = *self.last_heartbeat.lock().await;
        last.elapsed() < Duration::from_secs(5)
    }
}

#[allow(deprecated)]
impl Default for WebRemoteAsync {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn web_remote_async_creation() {
        let remote = WebRemoteAsync::new();
        assert!(!*remote.running.lock().await);
    }

    #[tokio::test]
    async fn web_remote_async_lifecycle() {
        let remote = WebRemoteAsync::new();
        remote.start_background_task();

        // Give it time to start
        sleep(Duration::from_millis(50)).await;

        assert!(remote.is_healthy().await);

        remote.stop().await;
    }
}
