//! Native UAT (978 MHz) receive and ADS-B payload decoder.
//!
//! Input is complex baseband sampled at exactly 2,083,333 samples/second and
//! centered on 978 MHz. Calls to [`UatDecoder::push`] may contain any number of
//! samples; the decoder retains only enough history to complete a maximal UAT
//! frame. A caller must call [`UatDecoder::reset`] after a capture discontinuity.

mod fec;
mod message;
mod rx;

pub use message::{AdsbReport, AirGroundState, MessageKind, UatMessage};
pub use rx::{ConfigError, SampleGap, UatDecoder, SAMPLE_RATE_HZ};
