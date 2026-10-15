//! Decode independently corrected CADUs to isolate packet/image acceptance
//! from RF transport. Input: ASM + 892 corrected VCDU bytes + 128 RS bytes.
//! Run with a public reference CADU file and an output directory.
#![allow(dead_code)]

#[path = "../src/ccsds.rs"]
mod ccsds;
#[path = "../src/image_builder.rs"]
mod image_builder;

use std::collections::BTreeMap;
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: audit_image_reference CORRECTED.cadu OUTPUT_DIR".into());
    }
    let mut input = std::fs::File::open(&args[1])?;
    std::fs::create_dir_all(&args[2])?;
    let mut reassembler = ccsds::PacketReassembler::new();
    let mut builder = image_builder::ImageBuilder::new();
    let mut frames = 0u64;
    let mut packets = BTreeMap::<u16, u64>::new();
    loop {
        let mut cadu = [0; 1024];
        let mut filled = 0;
        while filled < cadu.len() {
            let count = input.read(&mut cadu[filled..])?;
            if count == 0 {
                if filled != 0 {
                    return Err("truncated corrected CADU".into());
                }
                break;
            }
            filled += count;
        }
        if filled == 0 {
            break;
        }
        if cadu[..4] != [0x1a, 0xcf, 0xfc, 0x1d] {
            return Err("incorrect CADU marker".into());
        }
        frames += 1;
        let payload = &cadu[4..896];
        let header = ccsds::parse_vcdu_header(payload).ok_or("missing VCDU header")?;
        let mpdu = ccsds::parse_mpdu_header(&payload[8..10]).ok_or("missing M-PDU header")?;
        reassembler.push_mpdu_payload(
            &payload[10..],
            mpdu.first_header_pointer,
            header.virtual_channel_id,
            header.frame_counter,
        );
        for packet in reassembler.drain_completed() {
            *packets.entry(packet.apid).or_default() += 1;
            builder.push_packet(packet.apid, packet.header.sequence_count, &packet.data);
        }
    }
    let mut images = Vec::new();
    for (apid, image) in builder.render_all() {
        let path = std::path::Path::new(&args[2]).join(format!("apid-{apid}.png"));
        image.save(path)?;
        images.push(
            serde_json::json!({"apid": apid, "width": image.width(), "height": image.height()}),
        );
    }
    let result = serde_json::json!({
        "corrected_frames": frames, "packets": packets,
        "complete_lines": builder.total_line_count(), "decoded_segments": builder.total_segment_count(),
        "dropped_segments": builder.dropped_scanlines(), "images": images,
    });
    let json = serde_json::to_string_pretty(&result)?;
    println!("{json}");
    std::fs::write(std::path::Path::new(&args[2]).join("result.json"), json)?;
    Ok(())
}
