//! Test-only exports of the actual RF decoder's accepted data and products.
//! The selected reference contains original independent CADUs, in their
//! original order. It is not regenerated with the application's encoder.

use super::{ccsds, LrptDecoder};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

pub(crate) fn export(
    decoder: &LrptDecoder,
    reference: &[u8],
    directory: &Path,
    input_bytes: u64,
    start_seconds: u64,
    requested_seconds: u64,
    sample_rate_hz: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(directory)?;
    let observed_keys: Vec<_> = decoder
        .validated_payloads
        .iter()
        .map(|payload| {
            let header = ccsds::parse_vcdu_header(payload).unwrap();
            (header.virtual_channel_id, header.frame_counter)
        })
        .collect();
    let observed: BTreeSet<_> = observed_keys.iter().copied().collect();
    let mut all_reference_keys = BTreeSet::new();
    let mut selected_keys = Vec::new();
    let mut selected_reference = std::fs::File::create(directory.join("reference.cadu"))?;
    for cadu in reference.chunks_exact(1024) {
        let header = ccsds::parse_vcdu_header(&cadu[4..896]).unwrap();
        let key = (header.virtual_channel_id, header.frame_counter);
        all_reference_keys.insert(key);
        if observed.contains(&key) {
            selected_reference.write_all(cadu)?;
            selected_keys.push(key);
        }
    }
    let missing_reference_keys: Vec<_> =
        all_reference_keys.difference(&observed).copied().collect();
    let mut payloads = std::fs::File::create(directory.join("vcdu.bin"))?;
    for payload in &decoder.validated_payloads {
        payloads.write_all(payload)?;
    }
    // Retain the packets actually drained by the live decoder, not packets
    // reconstructed by running its reassembler a second time for this export.
    let mut packets = std::fs::File::create(directory.join("packets.bin"))?;
    let mut packet_counts = BTreeMap::<u16, usize>::new();
    for packet in &decoder.validated_packets {
        let header = packet.header;
        let word = (u16::from(header.version) << 13)
            | (u16::from(header.packet_type) << 12)
            | (u16::from(header.secondary_header_flag) << 11)
            | header.apid;
        let sequence = (u16::from(header.sequence_flags) << 14) | header.sequence_count;
        packets.write_all(&word.to_be_bytes())?;
        packets.write_all(&sequence.to_be_bytes())?;
        packets.write_all(&header.packet_data_length.to_be_bytes())?;
        packets.write_all(&packet.data)?;
        *packet_counts.entry(packet.apid).or_default() += 1;
    }
    let mut images = Vec::new();
    for (apid, image) in decoder.images.render_all() {
        image.save(directory.join(format!("apid-{apid}.png")))?;
        images.push(serde_json::json!({
            "apid": apid, "width": image.width(), "height": image.height(),
        }));
    }
    let result = serde_json::json!({
        "source": "actual LrptDecoder::for_mission RF path; test-only observation",
        "mission_profile": decoder.profile,
        "sample_rate_hz": sample_rate_hz,
        "input_bytes": input_bytes,
        "start_seconds": start_seconds,
        "requested_seconds": requested_seconds,
        "corrected_frames": decoder.validated_payloads.len(),
        "packets": packet_counts,
        "complete_lines": decoder.images.total_line_count(),
        "decoded_segments": decoder.images.total_segment_count(),
        "dropped_segments": decoder.images.dropped_scanlines(),
        "images": images,
        "observed_frame_keys": observed_keys,
        "selected_reference_order_matches": selected_keys == observed_keys,
        "reference_unique_frames": all_reference_keys.len(),
        "reference_missing_frame_keys": missing_reference_keys,
        "all_reference_frames_recovered": missing_reference_keys.is_empty(),
        "coverage_boundary": "Byte/image comparisons apply to accepted frames. Missing reference frames are reported explicitly and are not counted as recovered.",
    });
    std::fs::write(
        directory.join("result.json"),
        serde_json::to_string_pretty(&result)? + "\n",
    )?;
    Ok(())
}
