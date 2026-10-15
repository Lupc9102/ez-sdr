//! UAT shortened Reed-Solomon codes (GF(256), polynomial 0x187, fcr=120).
//!
//! The code parameters and correction procedure follow FlightAware dump978's
//! `fec.cc`/`libs/fec` implementation. The algorithm is a Rust translation of
//! the Phil Karn decoder distributed with dump978; see THIRD-PARTY-NOTICES.

const FIELD: usize = 255;
const A0: usize = FIELD;

#[derive(Clone)]
struct Gf {
    alpha: [u8; 256],
    index: [u16; 256],
}

impl Gf {
    fn new() -> Self {
        let mut alpha = [0; 256];
        let mut index = [A0 as u16; 256];
        let mut value = 1u16;
        for (i, alpha_i) in alpha.iter_mut().enumerate().take(FIELD) {
            index[value as usize] = i as u16;
            *alpha_i = value as u8;
            value <<= 1;
            if value & 0x100 != 0 {
                value ^= 0x187;
            }
            value &= 0xff;
        }
        alpha[FIELD] = 0;
        Self { alpha, index }
    }

    #[inline]
    fn pow(&self, n: isize) -> u8 {
        self.alpha[n.rem_euclid(FIELD as isize) as usize]
    }
}

fn decode_shortened(raw: &[u8], roots: usize, pad: usize) -> Option<(Vec<u8>, usize)> {
    if raw.len() + pad != FIELD || roots == 0 || roots >= FIELD {
        return None;
    }
    let gf = Gf::new();
    let mut syndromes = vec![0u8; roots];
    for s in &mut syndromes {
        *s = raw[0];
    }
    for &byte in &raw[1..] {
        for (i, s) in syndromes.iter_mut().enumerate() {
            *s = if *s == 0 {
                byte
            } else {
                byte ^ gf.pow(gf.index[*s as usize] as isize + (120 + i) as isize)
            };
        }
    }
    if syndromes.iter().all(|s| *s == 0) {
        return Some((raw[..raw.len() - roots].to_vec(), 0));
    }
    let si = syndromes
        .iter()
        .map(|s| gf.index[*s as usize])
        .collect::<Vec<_>>();
    let mut lambda = vec![0u8; roots + 1];
    let mut b = vec![A0 as u16; roots + 1];
    lambda[0] = 1;
    for i in 0..=roots {
        b[i] = gf.index[lambda[i] as usize];
    }
    let mut el = 0usize;
    for r in 1..=roots {
        let mut discrepancy = 0u8;
        for i in 0..r {
            if lambda[i] != 0 && si[r - i - 1] as usize != A0 {
                discrepancy ^=
                    gf.pow(gf.index[lambda[i] as usize] as isize + si[r - i - 1] as isize);
            }
        }
        let dlog = gf.index[discrepancy as usize] as usize;
        if dlog == A0 {
            b.copy_within(0..roots, 1);
            b[0] = A0 as u16;
            continue;
        }
        let mut t = vec![0u8; roots + 1];
        t[0] = lambda[0];
        for i in 0..roots {
            t[i + 1] = if b[i] as usize != A0 {
                lambda[i + 1] ^ gf.pow(dlog as isize + b[i] as isize)
            } else {
                lambda[i + 1]
            };
        }
        if 2 * el < r {
            el = r - el;
            for i in 0..=roots {
                b[i] = if lambda[i] == 0 {
                    A0 as u16
                } else {
                    ((gf.index[lambda[i] as usize] as usize + FIELD - dlog) % FIELD) as u16
                };
            }
        } else {
            b.copy_within(0..roots, 1);
            b[0] = A0 as u16;
        }
        lambda = t;
    }

    let mut lambda_log = vec![A0 as u16; roots + 1];
    let mut degree = 0;
    for i in 0..=roots {
        lambda_log[i] = gf.index[lambda[i] as usize];
        if lambda_log[i] as usize != A0 {
            degree = i;
        }
    }
    if degree == 0 {
        return None;
    }

    let mut reg = lambda_log.clone();
    let mut roots_at = Vec::with_capacity(degree);
    let mut loc = Vec::with_capacity(degree);
    for i in 1..=FIELD {
        let k = i - 1; // prim=1 => iprim=1
        let mut q = 1u8;
        for j in (1..=degree).rev() {
            if reg[j] as usize != A0 {
                reg[j] = ((reg[j] as usize + j) % FIELD) as u16;
                q ^= gf.alpha[reg[j] as usize];
            }
        }
        if q == 0 {
            if k < pad {
                return None;
            }
            roots_at.push(i);
            loc.push(k);
            if roots_at.len() == degree {
                break;
            }
        }
    }
    if roots_at.len() != degree {
        return None;
    }

    let omega_degree = degree - 1;
    let mut omega = vec![A0 as u16; roots];
    for i in 0..=omega_degree {
        let mut value = 0;
        for j in 0..=i {
            if si[i - j] as usize != A0 && lambda_log[j] as usize != A0 {
                value ^= gf.pow(si[i - j] as isize + lambda_log[j] as isize);
            }
        }
        omega[i] = gf.index[value as usize];
    }

    let mut corrections = vec![0u8; degree];
    let mut corrected_count = 0;
    for j in 0..degree {
        let mut numerator = 0u8;
        for i in (0..=omega_degree).rev() {
            if omega[i] as usize != A0 {
                numerator ^= gf.pow(omega[i] as isize + (i * roots_at[j]) as isize);
            }
        }
        if numerator == 0 {
            continue;
        }
        let numerator2 = gf.pow((roots_at[j] * 119 + FIELD) as isize);
        let mut denominator = 0u8;
        for i in (0..=degree.min(roots - 1)).step_by(2) {
            if lambda_log[i + 1] as usize != A0 {
                denominator ^= gf.pow(lambda_log[i + 1] as isize + (i * roots_at[j]) as isize);
            }
        }
        if denominator == 0 {
            return None;
        }
        let value = (gf.index[numerator as usize] as usize
            + gf.index[numerator2 as usize] as usize
            + FIELD
            - gf.index[denominator as usize] as usize)
            % FIELD;
        corrections[j] = gf.alpha[value];
        corrected_count += 1;
    }

    for (i, expected) in syndromes.iter().enumerate() {
        let mut actual = 0u8;
        for j in 0..degree {
            if corrections[j] != 0 {
                let exponent = ((120 + i) * (FIELD - loc[j] - 1)) % FIELD;
                actual ^= gf.pow(gf.index[corrections[j] as usize] as isize + exponent as isize);
            }
        }
        if actual != *expected {
            return None;
        }
    }

    let mut corrected = raw.to_vec();
    for i in 0..degree {
        if corrections[i] != 0 {
            let offset = loc[i] - pad;
            corrected[offset] ^= corrections[i];
        }
    }
    Some((
        corrected[..corrected.len() - roots].to_vec(),
        corrected_count,
    ))
}

pub(crate) fn correct_downlink(raw: &[u8]) -> Option<(Vec<u8>, usize)> {
    if raw.len() == 30 {
        return decode_shortened(raw, 12, 225).filter(|(payload, _)| payload[0] >> 3 == 0);
    }
    if raw.len() != 48 {
        return None;
    }
    if let Some(result) = decode_shortened(raw, 14, 207) {
        if result.0.first().is_some_and(|header| header >> 3 != 0) {
            return Some(result);
        }
    }
    decode_shortened(&raw[..30], 12, 225).filter(|(payload, _)| payload[0] >> 3 == 0)
}

pub(crate) fn correct_uplink(raw: &[u8]) -> Option<(Vec<u8>, usize)> {
    if raw.len() != 552 {
        return None;
    }
    let mut payload = Vec::with_capacity(432);
    let mut errors = 0;
    for block in 0..6 {
        let interleaved = (0..92).map(|i| raw[i * 6 + block]).collect::<Vec<_>>();
        let (data, count) = decode_shortened(&interleaved, 20, 163)?;
        payload.extend(data);
        errors += count;
    }
    Some((payload, errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_lengths_and_non_codewords() {
        assert!(correct_downlink(&[0; 47]).is_none());
        assert!(correct_uplink(&[0; 551]).is_none());
        let mut bad = [0u8; 48];
        for (i, byte) in bad.iter_mut().take(8).enumerate() {
            *byte = 0xa5 ^ i as u8;
        }
        assert!(correct_downlink(&bad).is_none());
    }

    #[test]
    fn corrects_published_dump978_long_downlink_codeword() {
        // First long packet from FlightAware dump978's sample-data.txt.gz.
        // Its parity was regenerated with the pinned upstream libfec encoder;
        // expected corrected payload is the published sample line.
        let raw = hex("08a66ef1353e2d525fd4050911882aa038101d06b85d440be2a4c2a0000590000000d0e3c7ccb1fed50a5afd9d6aa963");
        let (payload, corrections) = correct_downlink(&raw).unwrap();
        assert_eq!(corrections, 0);
        assert_eq!(
            payload,
            hex("08a66ef1353e2d525fd4050911882aa038101d06b85d440be2a4c2a0000590000000")
        );
    }

    #[test]
    fn corrects_multiple_symbol_errors_in_downlink() {
        let codeword = hex("08a66ef1353e2d525fd4050911882aa038101d06b85d440be2a4c2a0000590000000d0e3c7ccb1fed50a5afd9d6aa963");
        let expected = hex("08a66ef1353e2d525fd4050911882aa038101d06b85d440be2a4c2a0000590000000");
        let positions = [0usize, 1, 10, 19, 27, 33, 44];
        for count in 1..=7 {
            let mut raw = codeword.clone();
            for (n, position) in positions.iter().take(count).enumerate() {
                raw[*position] ^= 0x31 + n as u8;
            }
            let (payload, corrections) = correct_downlink(&raw).unwrap();
            assert_eq!(corrections, count);
            assert_eq!(payload, expected);
        }
    }

    #[test]
    fn decodes_published_basic_downlink_codeword() {
        // First short packet from dump978 sample-data.txt.gz, with parity
        // generated by the upstream (12-root) UAT short code.
        let raw = hex("00a66ef135445d525a280519119421204800a72cc2c53adcb4ca5e2cd175");
        let (payload, corrections) = correct_downlink(&raw).unwrap();
        assert_eq!(corrections, 0);
        assert_eq!(payload, hex("00a66ef135445d525a280519119421204800"));
    }

    #[test]
    fn decodes_upstream_encoded_six_block_uplink_fixture() {
        let raw = include_bytes!("../tests/fixtures/uplink_encoded.bin");
        let (payload, corrections) = correct_uplink(raw).unwrap();
        assert_eq!(corrections, 0);
        assert_eq!(payload, (0..432).map(|byte| byte as u8).collect::<Vec<_>>());

        let mut damaged = raw.to_vec();
        damaged[6 * 10 + 4] ^= 0x42;
        let (corrected, corrections) = correct_uplink(&damaged).unwrap();
        assert_eq!(corrections, 1);
        assert_eq!(corrected, payload);
    }

    fn hex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
}
