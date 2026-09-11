//! VCDU/M-PDU header parsing and CCSDS Space Packet reassembly.
//!
//! Implements the generic CCSDS 132.0-B "TM Space Data Link Protocol"
//! header layouts (VCDU + M-PDU) and the CCSDS Space Packet primary header
//! (a separate, widely-published generic standard), applied to the
//! RS-corrected transport frame payload.
//!
//! VCDU header (6 bytes, per CCSDS 132.0-B):
//! - byte 0: 2 bits TF version + 6 bits spacecraft ID (high bits)
//! - byte 1: 2 bits spacecraft ID (low bits) + 6 bits virtual channel ID
//! - bytes 2..5 (3 bytes, 24 bits): VCDU frame counter
//! - byte 5: VCDU frame counter cycle / replay flag byte (treated as
//!   reserved/spare here since only the counter value is used downstream)
//!
//! M-PDU header (2 bytes) immediately follows the VCDU header:
//! - top bit: spare
//! - remaining 11 bits: first header pointer (byte offset within the
//!   M-PDU payload where a new Space Packet primary header begins; a
//!   reserved value of 0x7FF means "no new packet starts in this VCDU").

use std::collections::BTreeMap;

/// VCDU header length in bytes.
pub const VCDU_HEADER_LEN: usize = 6;
/// M-PDU header length in bytes.
pub const MPDU_HEADER_LEN: usize = 2;
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

// Sequence flags per CCSDS 132.0-B Space Packet primary header. Current
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

/// Per-APID reassembly state: accumulates M-PDU payload bytes into
/// complete Space Packets, since a packet's data can span multiple VCDUs.
#[derive(Debug, Default)]
struct ApidBuffer {
    /// Bytes accumulated so far for the packet currently being assembled.
    buf: Vec<u8>,
    /// Expected total length of the packet currently being assembled, once
    /// known from its primary header.
    expected_len: Option<usize>,
}

/// Reassembles CCSDS Space Packets from a stream of M-PDU payloads (the
/// VCDU payload minus VCDU+M-PDU headers), tracking partial packets
/// per-APID across many VCDUs.
#[derive(Debug, Default)]
pub struct PacketReassembler {
    apid_buffers: BTreeMap<u16, ApidBuffer>,
    active_apid: Option<u16>,
    /// Fully reassembled packets ready to be drained by the caller.
    completed: Vec<SpacePacket>,
}

/// Upper bounds against hostile/junk input. A real LRPT stream uses a
/// handful of APIDs; thousands of distinct APIDs (or an undrained
/// completion queue) means garbage, not data.
pub const MAX_TRACKED_APIDS: usize = 64;
pub const MAX_COMPLETED_PACKETS: usize = 4096;

impl PacketReassembler {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one VCDU's M-PDU payload (after VCDU+M-PDU headers are
    /// stripped) plus the M-PDU header's `first_header_pointer`, which
    /// tells us where (if anywhere) a new Space Packet header starts
    /// within `payload`.
    ///
    /// Reassembly strategy: if `first_header_pointer` is
    /// `NO_PACKET_START`, the entire payload is a continuation of
    /// whichever APID packet is already in progress -- but since M-PDU
    /// payloads interleave multiple virtual channels' worth of packet data
    /// without per-byte APID tagging until a new header appears, we track
    /// a single "current APID" continuation context per call site. This
    /// matches the standard M-PDU model: at most one packet is "in
    /// progress" per virtual channel at any time, so continuation bytes
    /// append to whichever packet was last opened on this same virtual
    /// channel.
    pub fn push_mpdu_payload(&mut self, payload: &[u8], first_header_pointer: u16) {
        let mut cursor = 0usize;
        let current_apid: Option<u16> = self.last_open_apid();

        if first_header_pointer == NO_PACKET_START {
            // Entire payload continues whatever packet is open.
            if let Some(apid) = current_apid {
                self.append_and_try_complete(apid, payload);
            }
            return;
        }

        let pointer = usize::from(first_header_pointer);
        if pointer > payload.len() {
            // Corrupt first-header pointer (not the 0x7FF idle marker, yet
            // past the end — e.g. bit-error 0x7FE): nothing here can be
            // trusted as a packet start. The old code fell through with
            // cursor = 0 and parsed continuation bytes as fresh headers,
            // fabricating garbage scanlines. Drop the payload instead.
            return;
        }
        if pointer > 0 {
            // Bytes before the pointer are continuation of the previously
            // open packet.
            if let Some(apid) = current_apid {
                self.append_and_try_complete(apid, &payload[..pointer]);
            }
            cursor = pointer;
        }

        // From `cursor` onward: one or more new packet headers may start
        // back-to-back if multiple small packets fit in this VCDU.
        while cursor + SPACE_PACKET_HEADER_LEN <= payload.len() {
            let Some(header) = parse_space_packet_header(&payload[cursor..]) else {
                break;
            };
            let apid = header.apid;
            let remaining = &payload[cursor..];
            let take = remaining.len().min(header.total_len());
            if self.buffer_for(apid).is_some() {
                self.active_apid = Some(apid);
                let entry = self.apid_buffers.get_mut(&apid).unwrap();
                entry.buf.clear();
                entry.expected_len = Some(header.total_len());
                entry.buf.extend_from_slice(&remaining[..take]);
                self.try_complete(apid);
            }
            cursor += take;
        }
    }

    /// Mutable reassembly buffer for `apid`, refusing to track new APIDs
    /// past [`MAX_TRACKED_APIDS`] (junk input could otherwise open thousands
    /// of 64 KB buffers — see the `total_len` field sizing).
    fn buffer_for(&mut self, apid: u16) -> Option<&mut ApidBuffer> {
        if !self.apid_buffers.contains_key(&apid) && self.apid_buffers.len() >= MAX_TRACKED_APIDS {
            return None;
        }
        Some(self.apid_buffers.entry(apid).or_default())
    }

    fn last_open_apid(&self) -> Option<u16> {
        // Return active APID if it still has an open/incomplete packet buffer
        if let Some(apid) = self.active_apid {
            if let Some(b) = self.apid_buffers.get(&apid) {
                if matches!(b.expected_len, Some(len) if b.buf.len() < len) {
                    return Some(apid);
                }
            }
        }
        // Fallback: search deterministic BTreeMap order for an open packet
        self.apid_buffers
            .iter()
            .find(|(_, b)| matches!(b.expected_len, Some(len) if b.buf.len() < len))
            .map(|(&apid, _)| apid)
    }

    fn append_and_try_complete(&mut self, apid: u16, bytes: &[u8]) {
        let Some(entry) = self.buffer_for(apid) else {
            return;
        };
        entry.buf.extend_from_slice(bytes);
        self.try_complete(apid);
    }

    fn try_complete(&mut self, apid: u16) {
        let is_complete = self
            .apid_buffers
            .get(&apid)
            .is_some_and(|entry| matches!(entry.expected_len, Some(len) if entry.buf.len() >= len));
        if !is_complete {
            return;
        }
        if let Some(entry) = self.apid_buffers.get_mut(&apid) {
            let len = entry.expected_len.unwrap();
            let packet_bytes: Vec<u8> = entry.buf.drain(0..len).collect();
            entry.expected_len = None;
            if self.active_apid == Some(apid) {
                self.active_apid = None;
            }
            if let Some(header) = parse_space_packet_header(&packet_bytes) {
                let data = packet_bytes[SPACE_PACKET_HEADER_LEN..].to_vec();
                // Bound the queue for callers that never drain: in-pipeline
                // use drains every push, so this only bites direct API users
                // feeding junk without draining.
                if self.completed.len() >= MAX_COMPLETED_PACKETS {
                    self.completed.remove(0);
                }
                self.completed.push(SpacePacket { apid, header, data });
            }
        }
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
    fn reassembles_single_packet_within_one_vcdu() {
        let data = vec![0xAAu8; 20];
        let packet = build_space_packet(200, 1, &data);
        let mut reassembler = PacketReassembler::new();
        // first_header_pointer = 0: packet header starts at byte 0.
        reassembler.push_mpdu_payload(&packet, 0);
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
        reassembler.push_mpdu_payload(first_half, 0);
        assert!(reassembler.drain_completed().is_empty());

        // Second VCDU: no new packet starts, it's all continuation.
        reassembler.push_mpdu_payload(second_half, NO_PACKET_START);
        let completed = reassembler.drain_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].apid, 300);
        assert_eq!(completed[0].data, data);
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
        reassembler.push_mpdu_payload(&combined, 0);
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
        reassembler.push_mpdu_payload(&garbage, 0x7FE);
        assert!(reassembler.drain_completed().is_empty());
        assert!(reassembler.active_apid.is_none());
    }

    #[test]
    fn apid_tracking_is_bounded() {
        // Thousands of distinct APIDs (junk input) must not open thousands
        // of 64 KB buffers.
        let mut reassembler = PacketReassembler::new();
        for apid in 0..200u16 {
            let packet = build_space_packet(apid, 1, &[0xAAu8; 4]);
            reassembler.push_mpdu_payload(&packet, 0);
        }
        assert!(reassembler.apid_buffers.len() <= MAX_TRACKED_APIDS);
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
        reassembler.push_mpdu_payload(p1_a, 0);
        assert_eq!(reassembler.active_apid, Some(64));
        assert!(reassembler.drain_completed().is_empty());

        // Second VCDU continues APID 64
        reassembler.push_mpdu_payload(p1_b, NO_PACKET_START);
        let completed = reassembler.drain_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].apid, 64);
        assert_eq!(completed[0].data, data1);
        assert_eq!(reassembler.active_apid, None);
    }
}
