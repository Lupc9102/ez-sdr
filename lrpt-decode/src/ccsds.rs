//! VCDU/M-PDU header parsing and CCSDS Space Packet reassembly.
//!
//! Implements the CCSDS 732.0-B AOS Space Data Link Protocol
//! header layouts (VCDU + M-PDU) and the CCSDS Space Packet primary header
//! (a separate, widely-published generic standard), applied to the
//! RS-corrected transport frame payload.
//!
//! VCDU header (6 bytes, per CCSDS 732.0-B):
//! - byte 0: 2 bits TF version + 6 bits spacecraft ID (high bits)
//! - byte 1: 2 bits spacecraft ID (low bits) + 6 bits virtual channel ID
//! - bytes 2..5 (3 bytes, 24 bits): VCDU frame counter
//! - byte 5: VCDU frame counter cycle / replay flag byte (treated as
//!   reserved/spare here since only the counter value is used downstream)
//! - bytes 6..7: mission-specific VCDU insert zone (Meteor uses 2 bytes)
//!
//! M-PDU header (2 bytes) follows the insert zone:
//! - top bit: spare
//! - remaining 11 bits: first header pointer (byte offset within the
//!   M-PDU payload where a new Space Packet primary header begins; a
//!   reserved value of 0x7FF means "no new packet starts in this VCDU").

use std::collections::BTreeMap;

/// VCDU header length in bytes.
pub const VCDU_HEADER_LEN: usize = 6;
/// Meteor AOS VCDUs carry a two-byte insert zone before the M-PDU header.
pub const VCDU_INSERT_ZONE_LEN: usize = 2;
/// M-PDU header length in bytes.
pub const MPDU_HEADER_LEN: usize = 2;
/// Offset of the M-PDU header within an RS-corrected VCDU payload.
pub const MPDU_HEADER_OFFSET: usize = VCDU_HEADER_LEN + VCDU_INSERT_ZONE_LEN;
/// Sentinel value in the M-PDU first-header-pointer field meaning "no new
/// packet header starts in this VCDU payload".
pub const NO_PACKET_START: u16 = 0x7FF;
/// CCSDS Space Packet primary header length in bytes.
pub const SPACE_PACKET_HEADER_LEN: usize = 6;

/// Parsed VCDU header fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VcduHeader {
    pub version: u8,
    pub spacecraft_id: u16,
    pub virtual_channel_id: u8,
    pub frame_counter: u32,
}

/// Parse a 6-byte VCDU header.
#[must_use]
pub fn parse_vcdu_header(bytes: &[u8]) -> Option<VcduHeader> {
    if bytes.len() < VCDU_HEADER_LEN {
        return None;
    }
    let version = (bytes[0] >> 6) & 0b11;
    let spacecraft_id = (u16::from(bytes[0] & 0x3F) << 2) | u16::from(bytes[1] >> 6);
    let virtual_channel_id = bytes[1] & 0x3F;
    let frame_counter =
        (u32::from(bytes[2]) << 16) | (u32::from(bytes[3]) << 8) | u32::from(bytes[4]);
    Some(VcduHeader {
        version,
        spacecraft_id,
        virtual_channel_id,
        frame_counter,
    })
}

/// Parsed M-PDU header: first-header-pointer, in bytes, relative to the
/// start of the M-PDU payload (the data immediately after this header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpduHeader {
    pub first_header_pointer: u16,
}

/// Parse a 2-byte M-PDU header.
#[must_use]
pub fn parse_mpdu_header(bytes: &[u8]) -> Option<MpduHeader> {
    if bytes.len() < MPDU_HEADER_LEN {
        return None;
    }
    let raw = (u16::from(bytes[0]) << 8) | u16::from(bytes[1]);
    let first_header_pointer = raw & 0x07FF;
    Some(MpduHeader {
        first_header_pointer,
    })
}

/// Parsed CCSDS Space Packet primary header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpacePacketHeader {
    pub version: u8,
    pub packet_type: u8,
    pub secondary_header_flag: bool,
    pub apid: u16,
    pub sequence_flags: u8,
    pub sequence_count: u16,
    /// Data length field as transmitted (packet data length = this + 1
    /// bytes of packet data field, per CCSDS convention).
    pub packet_data_length: u16,
}

impl SpacePacketHeader {
    /// Total packet length in bytes: 6-byte primary header + packet data
    /// field (`packet_data_length + 1` bytes).
    #[must_use]
    pub fn total_len(&self) -> usize {
        SPACE_PACKET_HEADER_LEN + usize::from(self.packet_data_length) + 1
    }
}

/// Parse a 6-byte CCSDS Space Packet primary header.
#[must_use]
pub fn parse_space_packet_header(bytes: &[u8]) -> Option<SpacePacketHeader> {
    if bytes.len() < SPACE_PACKET_HEADER_LEN {
        return None;
    }
    let word0 = (u16::from(bytes[0]) << 8) | u16::from(bytes[1]);
    let version = ((word0 >> 13) & 0b111) as u8;
    let packet_type = ((word0 >> 12) & 0b1) as u8;
    let secondary_header_flag = ((word0 >> 11) & 0b1) != 0;
    let apid = word0 & 0x07FF;

    let word1 = (u16::from(bytes[2]) << 8) | u16::from(bytes[3]);
    let sequence_flags = ((word1 >> 14) & 0b11) as u8;
    let sequence_count = word1 & 0x3FFF;

    let packet_data_length = (u16::from(bytes[4]) << 8) | u16::from(bytes[5]);

    Some(SpacePacketHeader {
        version,
        packet_type,
        secondary_header_flag,
        apid,
        sequence_flags,
        sequence_count,
        packet_data_length,
    })
}

// Sequence flags per CCSDS 732.0-B Space Packet primary header. Current
// reassembly (`PacketReassembler`) segments purely on the M-PDU
// `first_header_pointer`, so these aren't consulted internally yet, but
// they're kept as documented, spec-accurate constants for callers that
// want to inspect `SpacePacketHeader::sequence_flags` directly (e.g. to
// detect and report anomalous segmentation).
#[allow(
    dead_code,
    reason = "documented CCSDS constant for header inspection, not yet consulted by reassembly logic"
)]
/// Sequence flags: packet is a standalone (unsegmented) packet.
pub const SEQ_FLAG_UNSEGMENTED: u8 = 0b11;
#[allow(
    dead_code,
    reason = "documented CCSDS constant for header inspection, not yet consulted by reassembly logic"
)]
/// Sequence flags: first segment of a packet.
pub const SEQ_FLAG_FIRST: u8 = 0b01;
#[allow(
    dead_code,
    reason = "documented CCSDS constant for header inspection, not yet consulted by reassembly logic"
)]
/// Sequence flags: continuation segment.
pub const SEQ_FLAG_CONTINUATION: u8 = 0b00;
#[allow(
    dead_code,
    reason = "documented CCSDS constant for header inspection, not yet consulted by reassembly logic"
)]
/// Sequence flags: last segment.
pub const SEQ_FLAG_LAST: u8 = 0b10;

/// A fully-reassembled CCSDS Space Packet.
#[derive(Debug, Clone)]
pub struct SpacePacket {
    pub apid: u16,
    /// Kept for future consumers that need version/type/sequence-count
    /// detail beyond `apid`; not read internally today.
    #[allow(dead_code)]
    pub header: SpacePacketHeader,
    /// Packet data field (everything after the 6-byte primary header),
    /// length `packet_data_length + 1` bytes when complete.
    pub data: Vec<u8>,
}

/// One in-progress packet per virtual channel, including a partial primary
/// header whose APID and length are not yet available.
#[derive(Debug, Default)]
struct PacketBuffer {
    bytes: Vec<u8>,
    header: Option<SpacePacketHeader>,
}

impl PacketBuffer {
    /// Consume only this packet's remaining header/body, never subsequent data.
    fn append(&mut self, bytes: &[u8]) -> Option<(usize, Option<SpacePacket>)> {
        let mut consumed = 0;
        if self.header.is_none() {
            consumed = bytes.len().min(SPACE_PACKET_HEADER_LEN - self.bytes.len());
            self.bytes.extend_from_slice(&bytes[..consumed]);
            if self.bytes.len() < SPACE_PACKET_HEADER_LEN {
                return Some((consumed, None));
            }
            let header = parse_space_packet_header(&self.bytes)?;
            if header.version != 0 {
                return None;
            }
            self.header = Some(header);
        }
        let header = self.header.unwrap();
        let take = (header.total_len() - self.bytes.len()).min(bytes.len() - consumed);
        self.bytes
            .extend_from_slice(&bytes[consumed..consumed + take]);
        consumed += take;
        let packet = (self.bytes.len() == header.total_len()).then(|| SpacePacket {
            apid: header.apid,
            header,
            data: self.bytes[SPACE_PACKET_HEADER_LEN..].to_vec(),
        });
        Some((consumed, packet))
    }
}

#[derive(Debug, Default)]
struct VcidContext {
    expected_frame_counter: Option<u32>,
    pending: Option<PacketBuffer>,
}

/// Reassembles M-PDU packets with independent state for each virtual channel.
/// APID identifies the delivered packet, never the owner of partial bytes.
#[derive(Debug, Default)]
pub struct PacketReassembler {
    vcid_contexts: BTreeMap<u8, VcidContext>,
    completed: Vec<SpacePacket>,
}

/// AOS VCIDs are six bits. Each can retain at most one 65,542-byte packet.
pub const MAX_VIRTUAL_CHANNELS: usize = 64;
pub const MAX_COMPLETED_PACKETS: usize = 4096;
/// An idle-only M-PDU does not interrupt an in-progress packet (AOS §4.1.4.2).
pub const IDLE_ONLY: u16 = 0x7FE;

impl PacketReassembler {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a packet zone. FHP marks its first new header; bytes before it
    /// continue the previous packet. Header and body fragmentation both persist
    /// across frames. Gaps invalidate only the affected virtual channel.
    pub fn push_mpdu_payload(
        &mut self,
        payload: &[u8],
        first_header_pointer: u16,
        vcid: u8,
        frame_counter: u32,
    ) {
        if usize::from(vcid) >= MAX_VIRTUAL_CHANNELS {
            return;
        }
        let context = self.vcid_contexts.entry(vcid).or_default();
        if context
            .expected_frame_counter
            .is_some_and(|expected| expected != frame_counter)
        {
            context.pending = None;
        }
        context.expected_frame_counter = Some(frame_counter.wrapping_add(1) & 0xFFFFFF);
        if first_header_pointer == IDLE_ONLY {
            return;
        }
        let pointer = usize::from(first_header_pointer);
        if first_header_pointer != NO_PACKET_START && pointer > payload.len() {
            context.pending = None;
            return;
        }
        let continuation_end = if first_header_pointer == NO_PACKET_START {
            payload.len()
        } else {
            pointer
        };
        if let Some(pending) = &mut context.pending {
            match pending.append(&payload[..continuation_end]) {
                Some((_, Some(packet))) => {
                    context.pending = None;
                    Self::complete(&mut self.completed, packet);
                }
                None => context.pending = None,
                _ => {}
            }
        }
        if first_header_pointer == NO_PACKET_START {
            return;
        }
        // A known new-header boundary supersedes any unfinished packet.
        context.pending = None;
        let mut cursor = pointer;
        while cursor < payload.len() {
            let mut pending = PacketBuffer::default();
            match pending.append(&payload[cursor..]) {
                Some((consumed, Some(packet))) => {
                    cursor += consumed;
                    Self::complete(&mut self.completed, packet);
                }
                Some((_, None)) => {
                    context.pending = Some(pending);
                    break;
                }
                None => break,
            }
        }
    }

    fn complete(completed: &mut Vec<SpacePacket>, packet: SpacePacket) {
        if packet.apid == 0x7FF {
            return; // CCSDS idle packet
        }
        if completed.len() >= MAX_COMPLETED_PACKETS {
            completed.remove(0);
        }
        completed.push(packet);
    }

    /// Drain all packets completed so far.
    pub fn drain_completed(&mut self) -> Vec<SpacePacket> {
        std::mem::take(&mut self.completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vcdu_header_fields() {
        // version=0, spacecraft_id = 0b00_1010_10 (170 in 8 bits split
        // 6+2), vcid=0b10_1010 (42), frame_counter = 0x00_01_02.
        let bytes = [
            0b00_101010, // version(2)=00, sc_id_hi(6)=101010
            0b10_101010, // sc_id_lo(2)=10, vcid(6)=101010
            0x00,
            0x01,
            0x02,
            0x00,
        ];
        let hdr = parse_vcdu_header(&bytes).unwrap();
        assert_eq!(hdr.version, 0);
        assert_eq!(hdr.frame_counter, 0x000102);
    }

    #[test]
    fn parses_mpdu_header_pointer() {
        let bytes = [0x00, 0x05]; // pointer=5
        let hdr = parse_mpdu_header(&bytes).unwrap();
        assert_eq!(hdr.first_header_pointer, 5);
    }

    #[test]
    fn mpdu_no_packet_start_sentinel() {
        let bytes = [0x07, 0xFF];
        let hdr = parse_mpdu_header(&bytes).unwrap();
        assert_eq!(hdr.first_header_pointer, NO_PACKET_START);
    }

    fn build_space_packet(apid: u16, seq: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let word0 = apid & 0x07FF;
        bytes.push((word0 >> 8) as u8);
        bytes.push((word0 & 0xFF) as u8);
        let word1 = (SEQ_FLAG_UNSEGMENTED as u16) << 14 | (seq & 0x3FFF);
        bytes.push((word1 >> 8) as u8);
        bytes.push((word1 & 0xFF) as u8);
        let len_field = (data.len() - 1) as u16;
        bytes.push((len_field >> 8) as u8);
        bytes.push((len_field & 0xFF) as u8);
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn parses_space_packet_header_roundtrip() {
        let data = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let packet = build_space_packet(100, 42, &data);
        let hdr = parse_space_packet_header(&packet).unwrap();
        assert_eq!(hdr.apid, 100);
        assert_eq!(hdr.sequence_count, 42);
        assert_eq!(hdr.sequence_flags, SEQ_FLAG_UNSEGMENTED);
        assert_eq!(hdr.total_len(), packet.len());
    }

    #[test]
    fn meteor_vcdu_layout_places_mpdu_after_two_byte_insert_zone() {
        let packet_data = b"realistic Meteor packet layout";
        let packet = build_space_packet(64, 7, packet_data);
        let mut payload = Vec::new();
        payload.extend_from_slice(&[
            0x2A, 0x83, 0x00, 0x01, 0x02, 0x00, // VCDU header, bytes 0..=5
        ]);
        payload.extend_from_slice(&[0x55, 0xAA]); // insert zone, bytes 6..=7
        payload.extend_from_slice(&[0x00, 0x00]); // M-PDU header, bytes 8..=9
        payload.extend_from_slice(&packet); // first Space Packet starts at byte 10

        assert_eq!(VCDU_HEADER_LEN, 6);
        assert_eq!(VCDU_INSERT_ZONE_LEN, 2);
        assert_eq!(MPDU_HEADER_OFFSET, 8);
        let vcdu = parse_vcdu_header(&payload[..VCDU_HEADER_LEN]).unwrap();
        let mpdu =
            parse_mpdu_header(&payload[MPDU_HEADER_OFFSET..MPDU_HEADER_OFFSET + MPDU_HEADER_LEN])
                .unwrap();
        assert_eq!(mpdu.first_header_pointer, 0);
        assert_eq!(parse_space_packet_header(&payload[10..]).unwrap().apid, 64);

        let mut reassembler = PacketReassembler::new();
        reassembler.push_mpdu_payload(
            &payload[MPDU_HEADER_OFFSET + MPDU_HEADER_LEN..],
            mpdu.first_header_pointer,
            vcdu.virtual_channel_id,
            vcdu.frame_counter,
        );
        let completed = reassembler.drain_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, packet_data);
    }

    #[test]
    fn reassembles_single_packet_within_one_vcdu() {
        let data = vec![0xAAu8; 20];
        let packet = build_space_packet(200, 1, &data);
        let mut reassembler = PacketReassembler::new();
        // first_header_pointer = 0: packet header starts at byte 0.
        reassembler.push_mpdu_payload(&packet, 0, 0, 0);
        let completed = reassembler.drain_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].apid, 200);
        assert_eq!(completed[0].data, data);
    }

    #[test]
    fn reassembles_packet_spanning_two_vcdus() {
        let data = vec![0x55u8; 40];
        let packet = build_space_packet(300, 2, &data);
        let mid = packet.len() / 2;
        let (first_half, second_half) = packet.split_at(mid);

        let mut reassembler = PacketReassembler::new();
        reassembler.push_mpdu_payload(first_half, 0, 0, 0);
        assert!(reassembler.drain_completed().is_empty());

        // Second VCDU: no new packet starts, it's all continuation.
        reassembler.push_mpdu_payload(second_half, NO_PACKET_START, 0, 1);
        let completed = reassembler.drain_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].apid, 300);
        assert_eq!(completed[0].data, data);
    }

    #[test]
    fn every_header_and_body_split_survives_continuations_and_idle_frames() {
        let packet = build_space_packet(64, 16383, b"split header and body");
        for split in 1..packet.len() {
            let mut assembler = PacketReassembler::new();
            assembler.push_mpdu_payload(&packet[..split], 0, 1, 0xFFFFFE);
            assert!(assembler.drain_completed().is_empty());
            assembler.push_mpdu_payload(&[0xFF; 20], IDLE_ONLY, 1, 0xFFFFFF);
            for (index, byte) in packet[split..].iter().enumerate() {
                assembler.push_mpdu_payload(&[*byte], NO_PACKET_START, 1, index as u32);
            }
            let result = assembler.drain_completed();
            assert_eq!(result.len(), 1, "split {split}");
            assert_eq!(result[0].header.sequence_count, 16383);
            assert_eq!(result[0].data, b"split header and body");
        }
    }

    #[test]
    fn split_header_continuation_ends_at_new_header_pointer() {
        let first = build_space_packet(64, 1, b"AAAA");
        let second = build_space_packet(65, 2, b"BBBB");
        for split in 1..first.len() {
            let mut assembler = PacketReassembler::new();
            assembler.push_mpdu_payload(&first[..split], 0, 1, 0);
            let tail = [&first[split..], &second].concat();
            assembler.push_mpdu_payload(&tail, (first.len() - split) as u16, 1, 1);
            let result = assembler.drain_completed();
            assert_eq!(result.len(), 2, "split {split}");
            assert_eq!(result[0].data, b"AAAA");
            assert_eq!(result[1].data, b"BBBB");
        }
    }

    #[test]
    fn same_apid_on_two_channels_keeps_bytes_and_frame_gaps_isolated() {
        let first = build_space_packet(64, 1, b"AAAA");
        let second = build_space_packet(64, 2, b"BBBB");
        for split in 1..first.len() {
            for gap in [false, true] {
                let mut assembler = PacketReassembler::new();
                assembler.push_mpdu_payload(&first[..split], 0, 1, 100);
                assembler.push_mpdu_payload(&second[..split], 0, 2, 200);
                assembler.push_mpdu_payload(
                    &first[split..],
                    NO_PACKET_START,
                    1,
                    if gap { 102 } else { 101 },
                );
                assembler.push_mpdu_payload(&second[split..], NO_PACKET_START, 2, 201);
                let result = assembler.drain_completed();
                if gap {
                    assert_eq!(result.len(), 1, "split {split}");
                    assert_eq!(result[0].data, b"BBBB");
                } else {
                    assert_eq!(result.len(), 2, "split {split}");
                    assert_eq!(result[0].data, b"AAAA");
                    assert_eq!(result[1].data, b"BBBB");
                }
            }
        }
    }

    #[test]
    fn gap_discards_partial_header_until_a_known_new_header() {
        let first = build_space_packet(64, 1, b"AAAA");
        let second = build_space_packet(65, 2, b"BBBB");
        let mut assembler = PacketReassembler::new();
        assembler.push_mpdu_payload(&first[..3], 0, 1, 0);
        assembler.push_mpdu_payload(&first[3..], NO_PACKET_START, 1, 2);
        assert!(assembler.drain_completed().is_empty());
        assembler.push_mpdu_payload(&second, 0, 1, 3);
        assert_eq!(assembler.drain_completed()[0].data, b"BBBB");
    }

    #[test]
    fn reassembles_multiple_small_packets_in_one_vcdu() {
        let data_a = vec![1u8; 10];
        let data_b = vec![2u8; 10];
        let packet_a = build_space_packet(10, 1, &data_a);
        let packet_b = build_space_packet(20, 1, &data_b);
        let mut combined = packet_a.clone();
        combined.extend_from_slice(&packet_b);

        let mut reassembler = PacketReassembler::new();
        reassembler.push_mpdu_payload(&combined, 0, 0, 0);
        let completed = reassembler.drain_completed();
        assert_eq!(completed.len(), 2);
        assert_eq!(completed[0].apid, 10);
        assert_eq!(completed[0].data, data_a);
        assert_eq!(completed[1].apid, 20);
        assert_eq!(completed[1].data, data_b);
    }

    #[test]
    fn corrupt_fhp_past_end_drops_payload_without_hallucinating() {
        // A corrupt first-header pointer (not 0x7FF, but past the end, e.g.
        // bit-error 0x7FE) must not parse continuation bytes as fresh packet
        // headers and fabricate garbage scanlines.
        let mut reassembler = PacketReassembler::new();
        let garbage = vec![0xFFu8; 64];
        reassembler.push_mpdu_payload(&garbage, 0x7FE, 0, 0);
        assert!(reassembler.drain_completed().is_empty());
        assert!(reassembler
            .vcid_contexts
            .get(&0)
            .is_none_or(|c| c.pending.is_none()));
    }

    #[test]
    fn apid_tracking_is_bounded() {
        // Thousands of distinct APIDs (junk input) must not open thousands
        // of 64 KB buffers.
        let mut reassembler = PacketReassembler::new();
        for apid in 0..200u16 {
            let packet = build_space_packet(apid, 1, &[0xAAu8; 4]);
            reassembler.push_mpdu_payload(&packet, 0, 0, apid as u32);
        }
        assert!(reassembler.vcid_contexts.len() <= MAX_VIRTUAL_CHANNELS);
        assert!(reassembler
            .vcid_contexts
            .values()
            .all(|context| context.pending.is_none()));
        let _ = reassembler.drain_completed();
    }

    #[test]
    fn apid_continuation_is_deterministic_and_routes_to_active_apid() {
        // Issue 25: Continuation packets must deterministically route to the active APID.
        let mut reassembler = PacketReassembler::new();
        let data1 = vec![0x42u8; 30];
        let p1 = build_space_packet(64, 1, &data1);
        let (p1_a, p1_b) = p1.split_at(15);

        // First VCDU opens APID 64
        reassembler.push_mpdu_payload(p1_a, 0, 0, 0);
        assert_eq!(
            reassembler
                .vcid_contexts
                .get(&0)
                .unwrap()
                .pending
                .as_ref()
                .and_then(|packet| packet.header.map(|header| header.apid)),
            Some(64)
        );
        assert!(reassembler.drain_completed().is_empty());

        // Second VCDU continues APID 64
        reassembler.push_mpdu_payload(p1_b, NO_PACKET_START, 0, 1);
        let completed = reassembler.drain_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].apid, 64);
        assert_eq!(completed[0].data, data1);
        assert_eq!(
            reassembler
                .vcid_contexts
                .get(&0)
                .unwrap()
                .pending
                .as_ref()
                .and_then(|packet| packet.header.map(|header| header.apid)),
            None
        );
    }
}
