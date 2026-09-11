//! Wire protocol shared between `ez-daemon` (the headless background service) and any
//! network-attached client (the GUI, a CLI monitor, an integration test, ...).
//!
//! The daemon is the sole owner of hardware and DSP state. Clients are on-demand
//! observers/controllers: they attach, subscribe to whichever pipelines they care about,
//! issue control commands, and may detach at any time without affecting the daemon or
//! other attached clients.

pub mod codec;
pub mod messages;

pub use codec::{CodecError, MessageCodec, MAX_CONTROL_FRAME_LEN, MAX_DATA_FRAME_LEN};
pub use messages::*;
