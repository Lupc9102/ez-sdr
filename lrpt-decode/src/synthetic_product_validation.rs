//! Independent synthetic 80k RF acceptance, including actual packets/images.
use super::*;
use std::io::Read;

#[test]
#[ignore = "requires independently generated 80k image IQ; EZ_80K_IMAGE_VECTORS"]
fn independent_80k_rf_images_match_reference_across_impairments_and_partitions() {
    let directory = std::path::PathBuf::from(std::env::var("EZ_80K_IMAGE_VECTORS").unwrap());
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let reference = std::fs::read(directory.join("reference.cadu")).unwrap();
    let expected: Vec<_> = reference
        .chunks_exact(1024)
        .map(|frame| frame[4..896].to_vec())
        .collect();
    assert!(expected.len() >= 100);
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut previous_payloads = None;
        let mut previous_images = None;
        for chunk_size in [997, 16_384] {
            let (tx, _rx) = crossbeam_channel::bounded(2);
            let mut decoder =
                LrptDecoder::for_mission(320_000, MeteorProfile::Oqpsk80Interleaved, tx).unwrap();
            let mut file =
                std::fs::File::open(directory.join(case["file"].as_str().unwrap())).unwrap();
            let mut bytes = vec![0; chunk_size * 8];
            let mut input_bytes = 0_u64;
            loop {
                // Fill a complete partition; a short OS read must not change
                // the intended boundaries or split a complex sample.
                let mut filled = 0;
                while filled < bytes.len() {
                    let read = file.read(&mut bytes[filled..]).unwrap();
                    if read == 0 {
                        break;
                    }
                    filled += read;
                }
                if filled == 0 {
                    break;
                }
                assert_eq!(filled % 8, 0);
                input_bytes += filled as u64;
                decoder.push_complex(&cf32_le_bytes_to_complex(&bytes[..filled]));
            }
            assert_eq!(input_bytes, case["samples"].as_u64().unwrap() * 8);
            let mut position = 0;
            for payload in &decoder.validated_payloads {
                position += expected[position..]
                    .iter()
                    .position(|reference| reference == payload)
                    .expect("accepted frame is corrupted, reordered, duplicated or absent from the reference")
                    + 1;
            }
            if case["require_all"].as_bool().unwrap() {
                assert_eq!(
                    decoder.validated_payloads, expected,
                    "missing {name} frames"
                );
            } else {
                assert!(
                    decoder.validated_payloads.len() >= 50,
                    "no useful fade recovery"
                );
                assert!(
                    decoder.validated_payloads.len() < expected.len(),
                    "fade did not exercise loss"
                );
                assert_eq!(
                    decoder.validated_payloads.last(),
                    expected.last(),
                    "did not recover final frame after phase slip"
                );
            }
            let images = decoder.images.render_all();
            assert_eq!(images.len(), 3);
            assert!(images.iter().all(|(_, image)| image.height() > 0));
            if let Some(previous) = &previous_payloads {
                assert_eq!(
                    &decoder.validated_payloads, previous,
                    "chunk-dependent {name} frames"
                );
            }
            if let Some(previous) = &previous_images {
                assert_eq!(&images, previous, "chunk-dependent {name} pixels");
            }
            previous_payloads = Some(decoder.validated_payloads.clone());
            previous_images = Some(images);
            let output = directory.join(format!("decoded-{name}-{chunk_size}"));
            off_air_validation::export(
                &decoder,
                &reference,
                &output,
                input_bytes,
                0,
                case["samples"].as_u64().unwrap().div_ceil(320_000),
                320_000,
            )
            .unwrap();
            println!(
                "SYNTHETIC_80K_RESULT {}",
                serde_json::json!({
                    "case": name, "chunk_size": chunk_size, "frames": decoder.validated_payloads.len(),
                    "reference_frames": expected.len(), "segments": decoder.images.total_segment_count(),
                    "rs_ok": decoder.rs_ok, "input_bytes": input_bytes,
                })
            );
        }
    }
}
