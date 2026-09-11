//! `ez-daemon`: the headless background service that owns all SDR hardware, wideband
//! channelization, and multi-tenant DSP pipeline routing for `ez-sdr`.
//!
//! Architecture (concrete modules and channels throughout — no generic workflow/stage-DAG
//! abstraction):
//!
//! ```text
//! IqSource --(ingest thread)--> SampleBus --(fan-out)--> VirtualChannel (DDC) --> pipeline
//! ```
//!
//! The daemon runs identically whether zero or many GUI clients are attached; clients are
//! on-demand network observers/controllers (see `ez-proto`), never part of the hot path.

pub mod app;
pub mod broadcast;
pub mod bus;
pub mod channelizer;
pub mod hardware;
pub mod ingest;
pub mod pipelines;
pub mod recording;
pub mod server;
pub mod state;
pub mod web;

pub use app::{run, DaemonConfig, SourceConfig};
