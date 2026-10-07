//! Reproducible audit probes, deliberately separate from production behavior.
//! Run: cargo run -p lrpt-decode --example audit_logic --release
#![allow(dead_code)]
#[path = "../src/ccsds.rs"]
mod ccsds;
#[path = "../src/frame_sync.rs"]
mod frame_sync;
#[path = "../src/image_builder.rs"]
mod image_builder;
#[path = "../src/qpsk.rs"]
mod qpsk;
#[path = "../src/randomizer.rs"]
mod randomizer;
#[path = "../src/reed_solomon.rs"]
mod reed_solomon;
#[path = "../src/viterbi.rs"]
mod viterbi;

use num_complex::Complex32;

fn packet(apid: u16, data: &[u8]) -> Vec<u8> {
    let mut bytes = (0x0800 | apid).to_be_bytes().to_vec();
    bytes.extend(0xc000u16.to_be_bytes());
    bytes.extend(((data.len() - 1) as u16).to_be_bytes());
    bytes.extend(data);
    bytes
}

fn coding(frame_count: usize) -> Vec<[f32; 2]> {
    let mut bits = vec![0; 256];
    for frame in 0..frame_count {
        let mut payload = vec![0u8; 892];
        payload[1] = 5;
        payload[4] = frame as u8;
        payload[8] = 7;
        payload[9] = 255;
        for (index, byte) in payload[10..].iter_mut().enumerate() {
            *byte = ((index * 37 + 19) % 256) as u8;
        }
        let mut cadu = frame_sync::SYNC_MARKER.to_be_bytes().to_vec();
        cadu.extend(randomizer::derandomize(&reed_solomon::encode_interleaved(
            &payload,
        )));
        bits.extend(
            cadu.iter()
                .flat_map(|byte| (0..8).rev().map(move |shift| (byte >> shift) & 1)),
        );
    }
    bits.extend(vec![0; 256]);
    let mut state = 0;
    bits.into_iter()
        .map(|bit| {
            let register: u8 = (bit << 6) | state;
            state = register >> 1;
            [0o171, 0o133].map(|generator| {
                if (register & generator).count_ones() % 2 == 0 {
                    -1.0
                } else {
                    1.0
                }
            })
        })
        .collect()
}

fn signal(symbols: &[[f32; 2]], phase: f32, offset: f32, rate: u32) -> Vec<Complex32> {
    let symbol_rate = 80_000.0;
    let count = ((symbols.len() as f64 + 0.5) * f64::from(rate) / symbol_rate) as usize;
    (0..count)
        .map(|index| {
            let time = index as f64 / f64::from(rate);
            let i = (time * symbol_rate).floor() as usize;
            let q = ((time * symbol_rate - 0.5).max(0.0)).floor() as usize;
            Complex32::new(
                symbols[i.min(symbols.len() - 1)][0],
                symbols[q.min(symbols.len() - 1)][1],
            ) * Complex32::from_polar(0.4, phase + std::f32::consts::TAU * offset * time as f32)
        })
        .collect()
}

fn main() {
    if std::env::var_os("EZ_AUDIT_LONG_VITERBI").is_some() {
        let mut decoder = viterbi::ViterbiDecoder::new();
        let mut seed = 0x12345678u32;
        for _ in 0..2_000 {
            let bits: Vec<u8> = (0..8_192)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed & 1) as u8
                })
                .collect();
            decoder.push_bits(&bits);
        }
        let input: Vec<u8> = (0..2_048).map(|i| ((i * 37 + i / 7) & 1) as u8).collect();
        let mut state = 0u8;
        let coded: Vec<_> = input
            .iter()
            .flat_map(|bit| {
                let reg = (bit << 6) | state;
                state = reg >> 1;
                [0o171, 0o133].map(|g| ((reg & g).count_ones() as u8 & 1) ^ 1)
            })
            .collect();
        let output = decoder.push_bits(&coded);
        let fresh = viterbi::ViterbiDecoder::new().push_bits(&coded);
        let fresh_errors = fresh
            .iter()
            .skip(128)
            .zip(input.iter().skip(65))
            .filter(|(a, b)| a != b)
            .count();
        println!(
            "viterbi fresh_control_errors={fresh_errors} fresh_output_ones={}",
            fresh.iter().filter(|b| **b == 1).count()
        );
        let errors = output
            .iter()
            .skip(128)
            .zip(input.iter().skip(65))
            .filter(|(a, b)| a != b)
            .count();
        println!("viterbi after_noise_trellis_steps=8192000 clean_comparison_bits={} errors={} output_ones={}", output.len()-128, errors, output.iter().filter(|b| **b == 1).count());
        return;
    }
    let complete = packet(64, b"header crossing VCDU");
    for split in [3, 6, 9] {
        let mut parser = ccsds::PacketReassembler::new();
        parser.push_mpdu_payload(&complete[..split], 0, 5, 0);
        parser.push_mpdu_payload(&complete[split..], ccsds::NO_PACKET_START, 5, 1);
        println!(
            "split_header bytes={split} packets={}",
            parser.drain_completed().len()
        );
    }
    let mut parser = ccsds::PacketReassembler::new();
    let a = packet(64, b"AAAA");
    let b = packet(64, b"BBBB");
    parser.push_mpdu_payload(&a[..8], 0, 1, 0);
    parser.push_mpdu_payload(&b[..8], 0, 2, 0);
    parser.push_mpdu_payload(&a[8..], ccsds::NO_PACKET_START, 1, 1);
    let mixed = parser.drain_completed();
    println!(
        "cross_vcid_same_apid data={:?}",
        mixed
            .iter()
            .map(|p| String::from_utf8_lossy(&p.data).into_owned())
            .collect::<Vec<_>>()
    );
    let symbols = coding(3);
    for rate in [640_000, 2_048_000] {
        for degrees in [0, 15, 30, 45, 60, 90, 180] {
            let iq = signal(&symbols, (degrees as f32).to_radians(), 0.0, rate);
            let (tx, _rx) = crossbeam_channel::bounded(2);
            let mut decoder = lrpt_decode::LrptDecoder::new(rate, 80_000, tx);
            for chunk in iq.chunks(16_384) {
                decoder.push_complex(chunk);
            }
            let result = decoder.finish();
            println!(
                "oqpsk rate={rate} phase={degrees} rs_ok={}",
                result.map_or(0, |r| r.rs_ok)
            );
        }
    }
    let mut demod = qpsk::QpskDemod::new(qpsk::QpskConfig {
        sample_rate: 640_000.0,
        symbol_rate: 80_000.0,
        oqpsk: true,
        ..Default::default()
    });
    demod.process(&vec![Complex32::new(0.0, 0.0); 10_000]);
    println!("zero_input_costas_locked={}", demod.is_locked());
    let mut bits = Vec::new();
    for frame in 0..2 {
        let marker = frame_sync::SYNC_MARKER ^ if frame == 0 { 3 } else { 0 };
        bits.extend((0..32).rev().map(|shift| ((marker >> shift) & 1) as u8));
        bits.extend(vec![0; 8_160]);
    }
    let count = frame_sync::FrameSync::new().push_bits(&bits).len();
    println!("frame_sync early_marker_2_errors later_marker_perfect extracted={count} expected=2");
    let six_frame_signal = signal(&coding(6), 0.0, 0.0, 640_000);
    let (tx, _rx) = crossbeam_channel::bounded(2);
    let mut control = lrpt_decode::LrptDecoder::new(640_000, 80_000, tx);
    for chunk in six_frame_signal.chunks(16_384) {
        control.push_complex(chunk);
    }
    let control_rs_ok = control.finish().map_or(0, |result| result.rs_ok);
    let (tx, _rx) = crossbeam_channel::bounded(2);
    let mut decoder = lrpt_decode::LrptDecoder::new(640_000, 80_000, tx);
    let mut slipped = six_frame_signal;
    let boundary = (256 + 3 * 8_192) * 8;
    for sample in &mut slipped[boundary..] {
        *sample = -*sample;
    }
    for chunk in slipped.chunks(16_384) {
        decoder.push_complex(chunk);
    }
    println!(
        "phase_slip_180 continuous_6frame_stream rs_ok={} clean_control_rs_ok={control_rs_ok}",
        decoder.finish().map_or(0, |result| result.rs_ok)
    );
    let mut fixture = vec![0u8; 14];
    fixture[11] = 0xff;
    fixture[12] = 0xf0;
    fixture[13] = 50;
    let bits: Vec<_> = (0..14).flat_map(|_| [0, 0, 1, 0, 1, 0]).collect();
    for chunk in bits.chunks(8) {
        fixture.push(
            chunk
                .iter()
                .enumerate()
                .fold(0, |byte, (i, bit)| byte | (bit << (7 - i))),
        );
    }
    let mut builder = image_builder::ImageBuilder::new();
    builder.push_packet(64, 16_300, &fixture);
    let first = builder.render(64);
    println!(
        "first_segment_sequence_16300 height={:?}",
        first.map(|i| i.height())
    );
    let mut builder = image_builder::ImageBuilder::new();
    builder.push_packet(
        64,
        0,
        include_bytes!("../../audit/2026-10-07/fixtures/nonflat-q50.segment"),
    );
    let decoded = builder.render(64).unwrap();
    let expected = include_bytes!("../../audit/2026-10-07/fixtures/nonflat-q50.pixels");
    let differences: Vec<_> = (0..8)
        .flat_map(|y| (0..112).map(move |x| (x, y)))
        .enumerate()
        .map(|(i, (x, y))| decoded.get_pixel(x, y).0[0].abs_diff(expected[i]))
        .collect();
    println!(
        "independent_nonflat_jpeg pixels={} max_error={} mean_error={:.6}",
        differences.len(),
        differences.iter().max().unwrap(),
        differences.iter().map(|x| *x as usize).sum::<usize>() as f64 / differences.len() as f64
    );
    let mut dc_segment = vec![0; 14];
    dc_segment[11..14].copy_from_slice(&[0xff, 0xf0, 50]);
    let mut entropy = vec![1, 1, 0, 1, 0, 0, 0, 0, 1, 0, 1, 0]; // standard DC category 5, +16, then EOB
    for _ in 1..14 {
        entropy.extend([0, 0, 1, 0, 1, 0]);
    }
    for chunk in entropy.chunks(8) {
        dc_segment.push(
            chunk
                .iter()
                .enumerate()
                .fold(0, |v, (i, b)| v | (b << (7 - i))),
        );
    }
    let mut dc_builder = image_builder::ImageBuilder::new();
    dc_builder.push_packet(64, 0, &dc_segment);
    let dc_image = dc_builder.render(64).unwrap();
    println!(
        "JPEG q50_DC16 expected_uniform_pixel=160 actual_first_pixel={}",
        dc_image.get_pixel(0, 0).0[0]
    );
}
