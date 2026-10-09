//! Meteor 80k transport: an eight-bit 0x27 marker every 80 coded bits,
//! followed by a 36-branch convolutional deinterleaver. Each branch visit
//! delays (35 - branch) * 2048 samples. Markers are outside the interleaver.
//!
//! The receiver owns buffered marker acquisition, commutator state and
//! missing-history flags across arbitrary input partitions. It never treats
//! zero-filled startup history as received data.

use std::collections::VecDeque;

pub(crate) const BRANCHES: usize = 36;
pub(crate) const DELAY_STEP: usize = 2048;
const MARKER: u8 = 0x27;
const WIRE_BITS: usize = 80;
pub(crate) const DATA_BITS: usize = 72;
const ACQUIRE_BITS: usize = 2 * WIRE_BITS + 8;
const MAX_BAD_MARKERS: usize = 3;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ChannelBit {
    pub value: u8,
    pub valid: bool,
}

impl ChannelBit {
    const ERASURE: Self = Self {
        value: 0,
        valid: false,
    };
}

pub(crate) struct TransportBlock {
    pub generation: u64,
    /// Position of the first data bit at this marker acquisition, in the
    /// original input coded-bit clock, before markers are removed.
    pub first_wire_bit: u64,
    pub bits: [ChannelBit; DATA_BITS],
}

pub(crate) struct MeteorDeinterleaver {
    pending: VecDeque<ChannelBit>,
    front_wire_bit: u64,
    generation: u64,
    locked: bool,
    bad_markers: usize,
    first_wire_bit: u64,
    branches: [VecDeque<ChannelBit>; BRANCHES],
    branch: usize,
}

impl MeteorDeinterleaver {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            front_wire_bit: 0,
            generation: 0,
            locked: false,
            bad_markers: 0,
            first_wire_bit: 0,
            branches: std::array::from_fn(|_| VecDeque::new()),
            branch: 0,
        }
    }

    fn marker_errors(&self, offset: usize) -> usize {
        (0..8)
            .filter(|&bit| {
                let received = self.pending[offset + bit];
                !received.valid || received.value != ((MARKER >> (7 - bit)) & 1)
            })
            .count()
    }

    fn discard(&mut self, count: usize) {
        self.pending.drain(..count);
        self.front_wire_bit += count as u64;
    }

    fn acquire(&mut self) {
        self.generation += 1;
        self.locked = true;
        self.bad_markers = 0;
        self.first_wire_bit = self.front_wire_bit + 8;
        self.branch = 0;
        for (branch, history) in self.branches.iter_mut().enumerate() {
            history.clear();
            history.resize((BRANCHES - 1 - branch) * DELAY_STEP, ChannelBit::ERASURE);
        }
    }

    pub fn push_bits(&mut self, bits: &[ChannelBit]) -> Vec<TransportBlock> {
        self.pending.extend(bits.iter().copied());
        let mut output = Vec::new();
        loop {
            if !self.locked {
                if self.pending.len() < ACQUIRE_BITS {
                    break;
                }
                // Three exact, energized repetitions establish the marker
                // phase. A single coincidental byte in data is insufficient.
                if [0, WIRE_BITS, 2 * WIRE_BITS]
                    .into_iter()
                    .any(|offset| self.marker_errors(offset) != 0)
                {
                    self.discard(1);
                    continue;
                }
                self.acquire();
            }
            if self.pending.len() < WIRE_BITS {
                break;
            }
            if self.marker_errors(0) > 2 {
                self.bad_markers += 1;
            } else {
                self.bad_markers = 0;
            }
            if self.bad_markers >= MAX_BAD_MARKERS {
                // Do not stitch a shifted stream into delayed old data.
                // Search from the current window, preserving possible new
                // markers inside it. Acquisition invalidates all history.
                self.locked = false;
                self.discard(1);
                continue;
            }
            let mut block = TransportBlock {
                generation: self.generation,
                first_wire_bit: self.first_wire_bit,
                bits: [ChannelBit::ERASURE; DATA_BITS],
            };
            for (index, result) in block.bits.iter_mut().enumerate() {
                let incoming = self.pending[index + 8];
                let history = &mut self.branches[self.branch];
                *result = if history.is_empty() {
                    incoming
                } else {
                    let delayed = history.pop_front().unwrap();
                    history.push_back(incoming);
                    delayed
                };
                self.branch = (self.branch + 1) % BRANCHES;
            }
            self.discard(WIRE_BITS);
            output.push(block);
        }
        output
    }

    /// Map a decoded information-bit end (including traceback latency) to
    /// its observation in the original recovered-symbol clock. This permits
    /// chronological merging across hypotheses with different acquisitions.
    pub fn observed_symbol_end(first_wire_bit: u64, information_end: u64) -> u64 {
        let last_data = information_end.saturating_mul(2).saturating_sub(1);
        let last_wire = first_wire_bit
            + (last_data / DATA_BITS as u64) * WIRE_BITS as u64
            + last_data % DATA_BITS as u64;
        (last_wire + 1).div_ceil(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unpack(bytes: &[u8]) -> Vec<ChannelBit> {
        bytes
            .iter()
            .flat_map(|&byte| {
                (0..8).map(move |bit| ChannelBit {
                    value: (byte >> (7 - bit)) & 1,
                    valid: true,
                })
            })
            .collect()
    }

    #[test]
    fn every_reference_bit_matches_the_independent_c_receiver_across_partitions() {
        let wire = unpack(include_bytes!(
            "../../audit/2026-10-07/fixtures/meteor-80k-wire.bits"
        ));
        let expected = unpack(include_bytes!(
            "../../audit/2026-10-07/fixtures/meteor-80k-reference.bits"
        ));
        for chunk_size in [61, 4096, wire.len()] {
            let mut stage = MeteorDeinterleaver::new();
            let mut count = 0;
            for chunk in wire.chunks(chunk_size) {
                for block in stage.push_bits(chunk) {
                    assert_eq!(block.generation, 1);
                    assert_eq!(block.first_wire_bit, 43); // 35-bit prefix + marker
                    for bit in block.bits {
                        assert_eq!(bit.value, expected[count].value, "coded bit {count}");
                        if count >= (BRANCHES - 1) * DELAY_STEP * BRANCHES {
                            assert!(bit.valid, "startup erasure did not clear at {count}");
                        }
                        count += 1;
                    }
                }
            }
            assert_eq!(count, expected.len());
        }
    }

    #[test]
    fn missing_input_follows_its_branch_delay_and_startup_is_unobserved() {
        let mut wire = unpack(include_bytes!(
            "../../audit/2026-10-07/fixtures/meteor-80k-wire.bits"
        ));
        let branch = 17;
        wire[35 + 8 + branch].valid = false;
        let erased_at = branch + (BRANCHES - 1 - branch) * DELAY_STEP * BRANCHES;
        let mut stage = MeteorDeinterleaver::new();
        let mut position = 0;
        for chunk in wire.chunks(4096) {
            for block in stage.push_bits(chunk) {
                for bit in block.bits {
                    if position == 0 || position == erased_at {
                        assert!(!bit.valid, "erasure became observed at {position}");
                    }
                    if position == erased_at + BRANCHES {
                        assert!(bit.valid, "one missing input poisoned later samples");
                    }
                    position += 1;
                }
            }
        }
    }

    fn marked_groups(count: usize) -> Vec<ChannelBit> {
        (0..count)
            .flat_map(|_| {
                let mut bits = unpack(&[MARKER]);
                bits.extend(
                    [ChannelBit {
                        value: 1,
                        valid: true,
                    }; DATA_BITS],
                );
                bits
            })
            .collect()
    }

    #[test]
    fn isolated_marker_errors_preserve_data_but_shifted_alignment_reacquires_empty_history() {
        let mut wire = marked_groups(12);
        wire[3 * WIRE_BITS].value ^= 1;
        let mut stage = MeteorDeinterleaver::new();
        let blocks = stage.push_bits(&wire);
        assert_eq!(blocks.len(), 12);
        assert!(blocks.iter().all(|block| block.generation == 1));

        let mut wire = marked_groups(12);
        for group in 4..7 {
            for bit in &mut wire[group * WIRE_BITS..group * WIRE_BITS + 8] {
                bit.value = 0;
            }
        }
        let mut stage = MeteorDeinterleaver::new();
        let blocks = stage.push_bits(&wire);
        let first_new = blocks.iter().find(|block| block.generation == 2).unwrap();
        assert_eq!(first_new.first_wire_bit, 7 * WIRE_BITS as u64 + 8);
        assert_eq!(first_new.bits.iter().filter(|bit| bit.valid).count(), 2);
        assert!(first_new.bits[35].valid && first_new.bits[71].valid);
    }

    #[test]
    fn noise_and_unobserved_markers_cannot_allocate_delay_history() {
        for valid in [false, true] {
            let mut stage = MeteorDeinterleaver::new();
            let input = vec![ChannelBit { value: 0, valid }; 20_000];
            assert!(stage.push_bits(&input).is_empty());
            assert!(stage.branches.iter().all(VecDeque::is_empty));
            assert!(stage.pending.len() < ACQUIRE_BITS);
        }
    }
}
