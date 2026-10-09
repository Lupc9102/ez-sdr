//! Shared Meteor mission/transport identity. A symbol rate alone cannot
//! identify marker interleaving or the differential channel-code convention.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum MeteorProfile {
    /// Meteor-M2-x: OQPSK, 72 ksym/s, NRZ-M after Viterbi.
    #[default]
    Oqpsk72,
    /// Meteor-M2-x: OQPSK, 80 ksym/s, markers and convolutional interleaver
    /// before Viterbi; NRZ-M after Viterbi.
    Oqpsk80Interleaved,
}

impl MeteorProfile {
    pub const ALL: [Self; 2] = [Self::Oqpsk72, Self::Oqpsk80Interleaved];

    pub const fn symbol_rate_hz(self) -> u32 {
        match self {
            Self::Oqpsk72 => 72_000,
            Self::Oqpsk80Interleaved => 80_000,
        }
    }

    pub const fn oqpsk(self) -> bool {
        matches!(self, Self::Oqpsk72 | Self::Oqpsk80Interleaved)
    }

    pub const fn nrzm(self) -> bool {
        matches!(self, Self::Oqpsk72 | Self::Oqpsk80Interleaved)
    }

    pub const fn interleaved(self) -> bool {
        matches!(self, Self::Oqpsk80Interleaved)
    }

    /// Full occupied bandwidth for the mission RRC roll-off of 0.5.
    pub const fn occupied_bandwidth_hz(self) -> u32 {
        self.symbol_rate_hz() * 3 / 2
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Oqpsk72 => "OQPSK 72k · NRZ-M",
            Self::Oqpsk80Interleaved => "OQPSK 80k · interleaved NRZ-M",
        }
    }
}
