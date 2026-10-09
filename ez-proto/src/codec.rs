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

/// Maximum frame length for command/control messages (64 KiB).
/// Prevents unauthenticated clients from forcing multi-megabyte heap allocations.
pub const MAX_CONTROL_FRAME_LEN: usize = 64 * 1024;

/// Maximum frame length for data frames (1 MiB — ample for 64k-point spectrum frames).
pub const MAX_DATA_FRAME_LEN: usize = 1024 * 1024;

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
///
/// There is deliberately **no** `Default` impl: the frame bound is a security
/// property (64 KiB control vs 1 MiB data) and must be chosen explicitly via
/// [`MessageCodec::for_commands`] or [`MessageCodec::for_data`]. A generic
/// 1 MiB default previously widened the unauthenticated control path 16×.
pub struct MessageCodec<T> {
    inner: LengthDelimitedCodec,
    max_len: usize,
    _marker: PhantomData<fn() -> T>,
}

impl<T> MessageCodec<T> {
    #[must_use]
    pub fn with_max_frame_length(max_len: usize) -> Self {
        Self {
            // Pin the wire format explicitly rather than relying on
            // `LengthDelimitedCodec` builder defaults: 4-byte big-endian
            // length prefix, consumed from the stream.
            inner: LengthDelimitedCodec::builder()
                .length_field_length(4)
                .length_field_offset(0)
                .num_skip(4)
                .length_adjustment(0)
                .big_endian()
                .max_frame_length(max_len)
                .new_codec(),
            max_len,
            _marker: PhantomData,
        }
    }

    /// Construct a codec bounded to 64 KiB for command/control messages.
    #[must_use]
    pub fn for_commands() -> Self {
        Self::with_max_frame_length(MAX_CONTROL_FRAME_LEN)
    }

    /// Construct a codec bounded to 1 MiB for data frames.
    #[must_use]
    pub fn for_data() -> Self {
        Self::with_max_frame_length(MAX_DATA_FRAME_LEN)
    }

    /// The frame bound this codec enforces.
    #[must_use]
    pub fn max_frame_length(&self) -> usize {
        self.max_len
    }
}

impl<T> Clone for MessageCodec<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            max_len: self.max_len,
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
            // The outer frame bound caps allocation; `bincode` container
            // lengths inside it cannot preallocate past it (serde's cautious
            // prealloc + growth bounded by actual input bytes), so no
            // separate inner limit is needed and the wire format is untouched.
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<T>, Self::Error> {
        match self.decode(src)? {
            Some(msg) => Ok(Some(msg)),
            None if src.is_empty() => Ok(None),
            // A peer that disconnects mid-frame is data loss, not a clean
            // shutdown: surface it instead of swallowing the tail silently.
            None => Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated frame at end of stream",
            )
            .into()),
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

    #[test]
    fn spectrum_observation_barrier_command_round_trip() {
        let original = crate::ClientCommand::BeginSpectrumObservation {
            channel_id: 0x8000_0000,
            revision: 735,
        };
        let expected = bincode::serialize(&original).unwrap();
        let mut codec = MessageCodec::<crate::ClientCommand>::for_commands();
        let mut bytes = BytesMut::new();
        codec.encode(original, &mut bytes).unwrap();
        let decoded = codec.decode(&mut bytes).unwrap().unwrap();
        assert_eq!(bincode::serialize(&decoded).unwrap(), expected);
        assert!(bytes.is_empty());
    }

    #[test]
    fn explicit_meteor_profiles_and_product_command_round_trip() {
        for profile in crate::MeteorProfile::ALL {
            for command in [
                crate::ClientCommand::Subscribe {
                    channel: crate::ChannelSpec {
                        meteor_profile: Some(profile),
                        id: 7,
                        center_offset_hz: 0,
                        bandwidth_hz: 400_000,
                        kind: crate::PipelineKind::LrptTelemetry,
                        demod_mode: None,
                    },
                },
                crate::ClientCommand::StartTelemetryProduct { channel_id: 7 },
            ] {
                let json = serde_json::to_string(&command).unwrap();
                let decoded_json: crate::ClientCommand = serde_json::from_str(&json).unwrap();
                assert_eq!(
                    bincode::serialize(&decoded_json).unwrap(),
                    bincode::serialize(&command).unwrap()
                );
                let mut codec = MessageCodec::<crate::ClientCommand>::for_commands();
                let mut bytes = BytesMut::new();
                let original = bincode::serialize(&command).unwrap();
                codec.encode(command, &mut bytes).unwrap();
                let decoded = codec.decode(&mut bytes).unwrap().unwrap();
                assert_eq!(bincode::serialize(&decoded).unwrap(), original);
            }
        }
    }

    #[test]
    fn private_session_and_spectrum_removal_identity_round_trip() {
        use crate::{ClientChannels, ServerEvent, SpectrumFrame};
        let ids = ClientChannels {
            spectrum: 0x8000_0000,
            audio: 0x8000_0001,
            adsb: 0x8000_0002,
            lrpt: 0x8000_0003,
        };
        for original in [
            ServerEvent::Welcome {
                protocol_version: crate::PROTOCOL_VERSION,
                client_channels: ids,
                active_channels: vec![],
            },
            ServerEvent::Spectrum(SpectrumFrame {
                channel_id: ids.spectrum,
                center_hz: 137_100_000,
                sample_rate_hz: 2_400_000,
                bins: vec![-100.0, -10.0],
                timestamp_ms: 123,
                observation_revision: 735,
                measurement_bins: Some(vec![-10.0, -30.0]),
            }),
            ServerEvent::ChannelRemoved {
                channel_id: ids.audio,
            },
        ] {
            let expected = bincode::serialize(&original).unwrap();
            let mut codec = MessageCodec::<ServerEvent>::for_data();
            let mut bytes = BytesMut::new();
            codec.encode(original, &mut bytes).unwrap();
            assert_eq!(
                bincode::serialize(&codec.decode(&mut bytes).unwrap().unwrap()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn aircraft_measurements_round_trip_without_losing_validity_or_reference() {
        use crate::{AircraftTelemetry, ServerEvent};
        for altitude in [None, Some(-1000), Some(-500), Some(0), Some(35000)] {
            let original = AircraftTelemetry {
                icao: 1,
                altitude_ft: altitude,
                altitude_geometric: true,
                ground_speed_kt: Some(0.0),
                track_deg: Some(90.0),
                heading_deg: Some(120.0),
                heading_true: true,
                airspeed_kt: Some(300.0),
                airspeed_true: true,
                surface_direction_deg: None,
                on_ground: Some(false),
                ..Default::default()
            };
            let mut codec = MessageCodec::<ServerEvent>::for_data();
            let mut bytes = BytesMut::new();
            codec
                .encode(
                    ServerEvent::Aircraft(crate::AircraftSnapshot {
                        channel_id: 73,
                        aircraft: vec![original],
                        receiver: crate::ReceiverSnapshot {
                            timestamp_ms: 1234,
                            total_messages: 100,
                            messages_per_second: 20.0,
                            decode_stats: (110, 100, 10, 0, 80, 15, 5, 0.4, 0.01, 0.9, 20),
                        },
                    }),
                    &mut bytes,
                )
                .unwrap();
            let ServerEvent::Aircraft(received) = codec.decode(&mut bytes).unwrap().unwrap() else {
                panic!("wrong event");
            };
            assert_eq!(received.channel_id, 73);
            assert_eq!(received.receiver.total_messages, 100);
            assert_eq!(received.receiver.messages_per_second, 20.0);
            assert_eq!(received.receiver.timestamp_ms, 1234);
            assert_eq!(received.receiver.decode_stats.6, 5);
            let item = &received.aircraft[0];
            assert_eq!(item.altitude_ft, altitude);
            assert!(item.altitude_geometric);
            assert_eq!(item.ground_speed_kt, Some(0.0));
            assert_eq!(item.track_deg, Some(90.0));
            assert_eq!(item.heading_deg, Some(120.0));
            assert!(item.heading_true);
            assert_eq!(item.airspeed_kt, Some(300.0));
            assert!(item.airspeed_true);
            assert_eq!(item.surface_direction_deg, None);
            assert_eq!(item.on_ground, Some(false));
        }
    }

    #[tokio::test]
    async fn round_trips_a_message_through_the_codec() {
        let (client, server) = duplex(4096);
        let mut client = Framed::new(client, MessageCodec::<ClientCommand>::for_commands());
        let mut server = Framed::new(server, MessageCodec::<ClientCommand>::for_commands());

        let msg = ClientCommand::Subscribe {
            channel: crate::messages::ChannelSpec {
                meteor_profile: None,
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
        // the bound must surface as a decode error, not hang or panic.
        let (mut client, server) = duplex(4096);
        let mut server = Framed::new(server, MessageCodec::<ClientCommand>::for_commands());

        use tokio::io::AsyncWriteExt;
        client.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        drop(client);

        let result = server.next().await.expect("stream ended");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn truncated_tail_at_eof_is_an_error_not_silent_none() {
        use tokio::io::AsyncWriteExt;
        let (mut client, server) = duplex(4096);
        let mut server = Framed::new(server, MessageCodec::<ClientCommand>::for_commands());
        // Valid 10-byte prefix, then only 4 of 10 payload bytes, then EOF.
        client.write_all(&10u32.to_be_bytes()).await.unwrap();
        client.write_all(&[0xAAu8; 4]).await.unwrap();
        drop(client);
        // First poll: incomplete frame, no item yet.
        // Second poll (EOF): must error, not cleanly end the stream.
        let mut saw_err = false;
        while let Some(result) = server.next().await {
            if result.is_err() {
                saw_err = true;
                break;
            }
        }
        assert!(saw_err, "truncated tail must surface as an error");
    }

    #[tokio::test]
    async fn constructors_pin_the_documented_bounds() {
        assert_eq!(
            MessageCodec::<ClientCommand>::for_commands().max_frame_length(),
            MAX_CONTROL_FRAME_LEN
        );
        assert_eq!(
            MessageCodec::<ClientCommand>::for_data().max_frame_length(),
            MAX_DATA_FRAME_LEN
        );
    }

    #[tokio::test]
    async fn command_frame_limit_rejects_oversized_payload() {
        // Issue 21: A frame claiming > MAX_CONTROL_FRAME_LEN must be rejected by for_commands()
        let (mut client, server) = duplex(4096);
        let mut server = Framed::new(server, MessageCodec::<ClientCommand>::for_commands());

        use tokio::io::AsyncWriteExt;
        let too_large = (MAX_CONTROL_FRAME_LEN as u32) + 1;
        client.write_all(&too_large.to_be_bytes()).await.unwrap();
        drop(client);

        let result = server.next().await.expect("stream ended");
        assert!(result.is_err(), "frames larger than 64KB must be rejected");
    }
    #[test]
    fn squelch_command_reply_and_audio_applied_metadata_round_trip() {
        use crate::{AudioFrame, ServerEvent, SquelchState};
        let command = ClientCommand::SetSquelch {
            channel_id: 17,
            db: -23.5,
            request_id: 123,
        };
        let mut commands = MessageCodec::for_commands();
        let mut bytes = BytesMut::new();
        commands.encode(command.clone(), &mut bytes).unwrap();
        assert_eq!(commands.decode(&mut bytes).unwrap(), Some(command));
        let applied = SquelchState {
            db: -23.5,
            revision: 45,
        };
        let mut data = MessageCodec::for_data();
        for result in [Ok(applied), Err("not an audio channel".into())] {
            data.encode(
                ServerEvent::Squelch {
                    channel_id: 17,
                    request_id: 123,
                    result: result.clone(),
                },
                &mut bytes,
            )
            .unwrap();
            let Some(ServerEvent::Squelch {
                channel_id,
                request_id,
                result: decoded,
            }) = data.decode(&mut bytes).unwrap()
            else {
                panic!("wrong event")
            };
            assert_eq!((channel_id, request_id, decoded), (17, 123, result));
        }
        data.encode(
            ServerEvent::Audio(AudioFrame {
                clock: crate::CaptureClock::new(1, 0, 48_000, None),
                sequence: 1,
                channel_id: 17,
                sample_rate_hz: 48_000,
                samples: vec![0.0; 32],
                squelch: applied,
            }),
            &mut bytes,
        )
        .unwrap();
        let Some(ServerEvent::Audio(frame)) = data.decode(&mut bytes).unwrap() else {
            panic!("wrong event")
        };
        assert_eq!(
            (frame.channel_id, frame.sample_rate_hz, frame.squelch),
            (17, 48_000, applied)
        );
        assert_eq!(frame.samples, vec![0.0; 32]);
        assert_eq!(frame.clock, crate::CaptureClock::new(1, 0, 48_000, None));
        assert_eq!(frame.sequence, 1);
        data.encode(
            ServerEvent::StreamGap {
                channel_id: 17,
                dropped_frames: 123,
            },
            &mut bytes,
        )
        .unwrap();
        assert!(matches!(
            data.decode(&mut bytes).unwrap(),
            Some(ServerEvent::StreamGap {
                channel_id: 17,
                dropped_frames: 123
            })
        ));
    }
}
