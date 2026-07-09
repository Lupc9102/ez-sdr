//! Daemon-side DSP pipelines: each takes a [`crate::bus::SampleBusHandle`] (wideband or a
//! [`crate::channelizer::Channelizer`]-produced virtual channel) and turns it into a
//! specific downstream product (spectrum frames, demodulated audio, decoded packets, ...).
//!
//! Each pipeline is a plain struct driven by `tick`/`run`, exactly like `Channelizer` — no
//! shared pipeline trait or stage-DAG abstraction. They differ enough in shape (continuous
//! DSP vs. block decode vs. event-driven parsing) that forcing a common trait would only
//! obscure each one's actual control flow.

pub mod audio;
pub mod packet;
pub mod spectrum;
