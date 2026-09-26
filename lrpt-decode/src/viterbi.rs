//! Hard-decision CCSDS rate-1/2, constraint-length-7 Viterbi decoder.
//!
//! Meteor LRPT's attached sync marker is convolutionally encoded on the air;
//! the raw dibit stream therefore cannot be searched for `0x1ACFFC1D` until
//! this stage has recovered the information bits. The decoder is deliberately
//! streaming and bounded: it emits one bit after a 64-bit traceback depth and
//! carries its path metrics across IQ chunks.

use std::collections::VecDeque;

const STATES: usize = 64;
const TRACEBACK: usize = 64;
const G1: u8 = 0o171;
const G2: u8 = 0o133;
const INF: i32 = 1_000_000;

#[inline]
fn parity(v: u8) -> u8 {
    (v.count_ones() as u8) & 1
}

#[inline]
fn branch_cost(state: usize, input: u8, received: [u8; 2]) -> i32 {
    let reg = ((input << 6) | state as u8) & 0x7f;
    let expected = [parity(reg & G1) ^ 1, parity(reg & G2) ^ 1];
    i32::from(expected[0] != received[0]) + i32::from(expected[1] != received[1])
}

/// A bounded streaming hard-decision Viterbi decoder.
#[derive(Debug, Clone)]
pub struct ViterbiDecoder {
    metrics: [i32; STATES],
    decisions: VecDeque<[u8; STATES]>,
}

impl Default for ViterbiDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ViterbiDecoder {
    #[must_use]
    pub fn new() -> Self {
        let mut metrics = [INF; STATES];
        metrics[0] = 0;
        Self {
            metrics,
            decisions: VecDeque::with_capacity(TRACEBACK + 1),
        }
    }

    /// Feed convolutionally encoded bits (two bits per trellis step).
    /// Returned bits are delayed by the traceback depth.
    pub fn push_bits(&mut self, bits: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bits.len() / 2);
        for pair in bits.chunks_exact(2) {
            let received = [pair[0] & 1, pair[1] & 1];
            let mut next = [INF; STATES];
            let mut predecessor = [0u8; STATES];
            for state in 0..STATES {
                if self.metrics[state] >= INF {
                    continue;
                }
                for input in 0..=1u8 {
                    let next_state = ((usize::from(input) << 5) | (state >> 1)) & (STATES - 1);
                    let cost = self.metrics[state] + branch_cost(state, input, received);
                    if cost < next[next_state] {
                        next[next_state] = cost;
                        predecessor[next_state] = state as u8;
                    }
                }
            }
            self.metrics = next;
            self.decisions.push_back(predecessor);

            if self.decisions.len() >= TRACEBACK {
                let mut state = self
                    .metrics
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, metric)| **metric)
                    .map_or(0, |(state, _)| state);
                let mut traceback = vec![0u8; self.decisions.len()];
                for (idx, decision) in self.decisions.iter().enumerate().rev() {
                    traceback[idx] = ((state >> 5) & 1) as u8;
                    state = usize::from(decision[state]);
                }
                out.push(traceback[0]);
                self.decisions.pop_front();
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(bits: &[u8]) -> Vec<u8> {
        let mut state = 0u8;
        let mut out = Vec::with_capacity(bits.len() * 2);
        for &bit in bits {
            let reg = (((bit & 1) << 6) | state) & 0x7f;
            out.push(parity(reg & G1) ^ 1);
            out.push(parity(reg & G2) ^ 1);
            state = (reg >> 1) & 0x3f;
        }
        out
    }

    #[test]
    fn decodes_clean_rate_half_stream() {
        let source: Vec<u8> = (0..160).map(|i| ((i * 17 + 3) & 1) as u8).collect();
        let coded = encode(&source);
        let mut decoder = ViterbiDecoder::new();
        let decoded = decoder.push_bits(&coded);
        assert!(decoded.len() >= source.len() - TRACEBACK);
        assert_eq!(
            &decoded[..source.len() - TRACEBACK],
            &source[..source.len() - TRACEBACK]
        );
    }

    #[test]
    fn coded_asm_matches_meteor_reference_word() {
        let source: Vec<u8> = 0x1acf_fc1du32
            .to_be_bytes()
            .iter()
            .flat_map(|byte| (0..8).rev().map(move |bit| (byte >> bit) & 1))
            .collect();
        let coded = encode(&source);
        let mut value = 0u64;
        for bit in coded {
            value = (value << 1) | u64::from(bit);
        }
        assert_eq!(value, 0xfca2_b63d_b00d_9794);
    }
}
