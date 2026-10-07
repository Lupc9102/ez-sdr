//! Control replies remain independent of coalesced live stream delivery.
use ez_proto::{stream_queue::StreamQueue, ChannelId, ServerEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, Notify};

#[derive(Clone)]
pub(crate) struct EventSender {
    control: mpsc::Sender<ServerEvent>,
    data: Arc<Mutex<StreamQueue>>,
    notify: Arc<Notify>,
    failed: Arc<AtomicBool>,
}

pub(crate) struct EventReceiver {
    control: mpsc::Receiver<ServerEvent>,
    data: Arc<Mutex<StreamQueue>>,
    notify: Arc<Notify>,
    failed: Arc<AtomicBool>,
}

pub(crate) fn channel(capacity: usize) -> (EventSender, EventReceiver) {
    let (control_tx, control_rx) = mpsc::channel(capacity);
    let data = Arc::new(Mutex::new(StreamQueue::default()));
    let notify = Arc::new(Notify::new());
    let failed = Arc::new(AtomicBool::new(false));
    (
        EventSender {
            control: control_tx,
            data: Arc::clone(&data),
            notify: Arc::clone(&notify),
            failed: Arc::clone(&failed),
        },
        EventReceiver {
            control: control_rx,
            data,
            notify,
            failed,
        },
    )
}

impl EventSender {
    pub async fn send(
        &self,
        event: ServerEvent,
    ) -> Result<(), mpsc::error::SendError<ServerEvent>> {
        if self.is_closed() {
            return Err(mpsc::error::SendError(event));
        }
        if StreamQueue::is_stream(&event) {
            return if self.stream(event.clone()) {
                Ok(())
            } else {
                Err(mpsc::error::SendError(event))
            };
        }
        if let ServerEvent::ChannelRemoved { channel_id } = &event {
            self.clear(*channel_id);
        }
        self.control.send(event).await
    }

    pub fn stream(&self, event: ServerEvent) -> bool {
        if self.is_closed() {
            return false;
        }
        let accepted = self
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(event, Instant::now());
        if !accepted {
            self.fail();
        }
        self.notify.notify_one();
        accepted
    }

    pub fn note_gap(&self, id: ChannelId, count: u64) {
        let accepted = self
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .note_gap(id, count, Instant::now());
        if !accepted {
            self.fail();
        }
        self.notify.notify_one();
    }

    pub fn clear(&self, id: ChannelId) {
        self.data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear(id);
    }

    pub fn is_closed(&self) -> bool {
        self.control.is_closed() || self.failed.load(Ordering::Acquire)
    }
    fn fail(&self) {
        if !self.failed.swap(true, Ordering::AcqRel) {
            let _ = self.control.try_send(ServerEvent::Error {
                message: "stream channel queue limit exceeded; reconnect".into(),
            });
        }
    }
    #[cfg(test)]
    pub fn queued_frames(&self) -> usize {
        self.data.lock().unwrap().queued_frames()
    }
}

impl EventReceiver {
    pub async fn recv(&mut self) -> Option<ServerEvent> {
        loop {
            let wake = self.notify.notified();
            if let Ok(event) = self.control.try_recv() {
                return Some(event);
            }
            if self.failed.load(Ordering::Acquire) {
                return None;
            }
            if let Some((event, _)) = self.data.lock().unwrap_or_else(|p| p.into_inner()).pop() {
                return Some(event);
            }
            tokio::select! {
                biased;
                event = self.control.recv() => return event,
                _ = wake => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mixed_stream_channel_limit_reports_error_and_closes_without_silent_loss() {
        let (tx, mut rx) = channel(2);
        for id in 0..32 {
            assert!(tx.stream(ServerEvent::Aircraft(ez_proto::AircraftSnapshot {
                channel_id: id,
                ..Default::default()
            })));
        }
        assert!(
            !tx.stream(ServerEvent::Aircraft(ez_proto::AircraftSnapshot {
                channel_id: 32,
                ..Default::default()
            }))
        );
        assert!(tx.is_closed());
        assert!(
            matches!(rx.recv().await, Some(ServerEvent::Error { message }) if message.contains("queue limit"))
        );
        assert!(rx.recv().await.is_none());
        assert!(tx.send(ServerEvent::Pong { nonce: 1 }).await.is_err());
    }
}
