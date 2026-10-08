//! Join a native read worker before its device or callback context can be released.
use anyhow::Result;
use std::thread::JoinHandle;
use std::time::Duration;

pub(crate) struct StreamWorker {
    thread: Option<JoinHandle<Result<()>>>,
    cancel: Box<dyn Fn() + Send>,
}
impl StreamWorker {
    pub(crate) fn spawn(
        run: impl FnOnce() -> Result<()> + Send + 'static,
        cancel: impl Fn() + Send + 'static,
    ) -> Result<Self> {
        Ok(Self {
            thread: Some(
                std::thread::Builder::new()
                    .name("rtl-async-read".into())
                    .spawn(run)?,
            ),
            cancel: Box::new(cancel),
        })
    }
    pub(crate) fn finish_if_ready(&mut self) -> Option<Result<()>> {
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.is_finished())
        {
            Some(self.join())
        } else {
            None
        }
    }
    fn join(&mut self) -> Result<()> {
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| anyhow::anyhow!("RTL read worker panicked"))?,
            None => Ok(()),
        }
    }
    pub(crate) fn stop(&mut self) -> Result<()> {
        while self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            // librtlsdr ignores cancellation before read_async enters RUNNING.
            // Repeat it while joining, including the startup race. The native
            // event loop and cancellation own all outstanding USB transfers.
            (self.cancel)();
            std::thread::sleep(Duration::from_millis(10));
        }
        self.join()
    }
}
impl Drop for StreamWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };
    #[test]
    fn repeated_cancel_survives_startup_race_and_joins_before_device_release() {
        let active = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let run_active = Arc::clone(&active);
        let run_cancelled = Arc::clone(&cancelled);
        let run_ended = Arc::clone(&ended);
        let cancel_active = Arc::clone(&active);
        let cancel_cancelled = Arc::clone(&cancelled);
        let cancel_attempts = Arc::clone(&attempts);
        let mut worker = StreamWorker::spawn(
            move || {
                std::thread::sleep(Duration::from_millis(50));
                run_active.store(true, Ordering::Release);
                while !run_cancelled.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                // Models cancellation draining outstanding transfers before return.
                std::thread::sleep(Duration::from_millis(10));
                run_ended.store(true, Ordering::Release);
                Ok(())
            },
            move || {
                cancel_attempts.fetch_add(1, Ordering::Relaxed);
                if cancel_active.load(Ordering::Acquire) {
                    cancel_cancelled.store(true, Ordering::Release);
                }
            },
        )
        .unwrap();
        assert!(worker.finish_if_ready().is_none());
        let start = std::time::Instant::now();
        worker.stop().unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(attempts.load(Ordering::Relaxed) >= 2);
        assert!(
            ended.load(Ordering::Acquire),
            "device may only be closed after this point"
        );
    }
    #[test]
    fn native_read_failure_is_reported_after_the_worker_finishes() {
        let mut worker =
            StreamWorker::spawn(|| anyhow::bail!("injected USB failure"), || {}).unwrap();
        assert!(worker
            .stop()
            .unwrap_err()
            .to_string()
            .contains("USB failure"));
    }
}
