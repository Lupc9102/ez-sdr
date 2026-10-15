//! Soapy stream result seam, shared by the FFI backend and injected-code tests.
use super::ReadPending;
use anyhow::Result;
use num_complex::Complex32;

pub(crate) fn read_once(
    output: &mut [Complex32],
    bytes: &mut Vec<u8>,
    read: impl FnOnce(&mut [u8], usize) -> i32,
) -> Result<usize> {
    let count = output.len();
    bytes.resize(count * 4, 0);
    let result = read(bytes, count);
    match result {
        -1 | 0 => Err(ReadPending::Timeout.into()),
        -4 => Err(ReadPending::Gap.into()),
        n if n < 0 => anyhow::bail!("soapy readStream failed with code {n}"),
        n => {
            anyhow::ensure!(
                n as usize <= count,
                "soapy readStream exceeded buffer: {n} > {count}"
            );
            // CS16 is a native-endian ABI stream, rather than a disk format.
            for (sample, pair) in output
                .iter_mut()
                .zip(bytes[..n as usize * 4].chunks_exact(4))
            {
                *sample = Complex32::new(
                    f32::from(i16::from_ne_bytes([pair[0], pair[1]])) / 32768.0,
                    f32::from(i16::from_ne_bytes([pair[2], pair[3]])) / 32768.0,
                );
            }
            Ok(n as usize)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn documented_timeout_overflow_and_fatal_codes_use_distinct_contracts() {
        let mut output = [Complex32::new(99.0, 99.0); 2];
        let mut bytes = Vec::new();
        for code in [-1, 0, -4, -2, -3, -5] {
            let error = read_once(&mut output, &mut bytes, |_, _| code).unwrap_err();
            match code {
                -1 | 0 => assert!(matches!(
                    error.downcast_ref::<ReadPending>(),
                    Some(ReadPending::Timeout)
                )),
                -4 => assert!(matches!(
                    error.downcast_ref::<ReadPending>(),
                    Some(ReadPending::Gap)
                )),
                _ => assert!(error.downcast_ref::<ReadPending>().is_none()),
            }
            assert_eq!(output, [Complex32::new(99.0, 99.0); 2]);
        }
        let count = read_once(&mut output, &mut bytes, |bytes, count| {
            assert_eq!(count, 2);
            for (sample, pair) in [i16::MIN, i16::MAX, 0, -8192]
                .iter()
                .zip(bytes.chunks_exact_mut(2))
            {
                pair.copy_from_slice(&sample.to_ne_bytes());
            }
            2
        })
        .unwrap();
        assert_eq!(count, 2);
        assert_eq!(output[0], Complex32::new(-1.0, 32767.0 / 32768.0));
        assert_eq!(output[1], Complex32::new(0.0, -0.25));
        assert!(read_once(&mut output, &mut bytes, |_, _| 3).is_err());
    }
}
