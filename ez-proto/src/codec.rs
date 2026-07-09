//! Length-delimited, `bincode`-serialized framing over any `AsyncRead`/`AsyncWrite`.
//!
//! Wire format per message: a 4-byte big-endian length prefix (via
//! [`tokio_util::codec::LengthDelimitedCodec`]) followed by that many bytes of `bincode`
//! payload. This keeps framing cheap (no delimiter scanning, no text parsing) which
//! matters for high-rate spectrum/audio streaming.

use std::marker::PhantomData;

use bytes::{Bytes, BytesMut};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec};

/// 16 MiB — comfortably above a full-resolution spectrum frame or a telemetry image
/// tile, while still catching a corrupt/malicious length prefix quickly.
const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bincode error: {0}")]
    Bincode(#[from] bincode::Error),
}

/// A `tokio_util` [`Encoder`]/[`Decoder`] for a single message type `T`, layered on top
/// of length-delimited framing. Construct one per direction (a connection typically
/// needs `MessageCodec<ClientCommand>` for reads and `MessageCodec<ServerEvent>` for
/// writes, or vice versa on the client side).
pub struct MessageCodec<T> {
    inner: LengthDelimitedCodec,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Default for MessageCodec<T> {
    fn default() -> Self {
        Self {
            inner: LengthDelimitedCodec::builder()
                .max_frame_length(MAX_FRAME_LEN)
                .new_codec(),
            _marker: PhantomData,
        }
    }
}

impl<T> Clone for MessageCodec<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: Serialize> Encoder<T> for MessageCodec<T> {
    type Error = CodecError;

    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload = bincode::serialize(&item)?;
        self.inner
            .encode(Bytes::from(payload), dst)
            .map_err(CodecError::Io)
    }
}

impl<T: DeserializeOwned> Decoder for MessageCodec<T> {
    type Item = T;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<T>, Self::Error> {
        match self.inner.decode(src)? {
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{ClientCommand, PipelineKind};
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::duplex;
    use tokio_util::codec::Framed;

    #[tokio::test]
    async fn round_trips_a_message_through_the_codec() {
        let (client, server) = duplex(4096);
        let mut client = Framed::new(client, MessageCodec::<ClientCommand>::default());
        let mut server = Framed::new(server, MessageCodec::<ClientCommand>::default());

        let msg = ClientCommand::Subscribe {
            channel: crate::messages::ChannelSpec {
                id: 7,
                center_offset_hz: -25_000,
                bandwidth_hz: 12_500,
                kind: PipelineKind::Audio,
                demod_mode: Some(crate::messages::DemodMode::Fm),
            },
        };

        client.send(msg.clone()).await.expect("send");
        let received = server
            .next()
            .await
            .expect("stream ended")
            .expect("decode error");
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn oversized_frame_is_rejected_not_panicking() {
        // A garbage 4-byte length prefix claiming a frame far larger than
        // MAX_FRAME_LEN must surface as a decode error, not hang or panic.
        let (mut client, server) = duplex(4096);
        let mut server = Framed::new(server, MessageCodec::<ClientCommand>::default());

        use tokio::io::AsyncWriteExt;
        client.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        drop(client);

        let result = server.next().await.expect("stream ended");
        assert!(result.is_err());
    }
}
