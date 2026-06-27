//! CCSDS - translated from src-core/common/ccsds/

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CCSDSHeader {
    pub raw: [u8; 6],
    pub version: u8,
    pub packet_type: bool,
    pub secondary_header_flag: bool,
    pub apid: u16,
    pub sequence_flag: u8,
    pub packet_sequence_count: u16,
    pub packet_length: u16,
}

impl CCSDSHeader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_bytes(raw: &[u8]) -> Self {
        let mut r = [0u8; 6];
        r.copy_from_slice(&raw[..6]);
        let version = raw[0] >> 5;
        let packet_type = ((raw[0] >> 4) & 0x01) != 0;
        let secondary_header_flag = ((raw[0] >> 3) & 0x01) != 0;
        let apid = (((raw[0] & 0x07) as u16) << 8) | (raw[1] as u16);
        let sequence_flag = raw[2] >> 6;
        let packet_sequence_count = (((raw[2] & 0x3F) as u16) << 8) | (raw[3] as u16);
        let packet_length = ((raw[4] as u16) << 8) | (raw[5] as u16);
        Self {
            raw: r,
            version,
            packet_type,
            secondary_header_flag,
            apid,
            sequence_flag,
            packet_sequence_count,
            packet_length,
        }
    }

    pub fn encode_hdr(&mut self) {
        self.raw[0] = ((self.version & 0x07) << 5)
            | ((self.packet_type as u8) << 4)
            | ((self.secondary_header_flag as u8) << 3)
            | (((self.apid >> 8) & 0x07) as u8);
        self.raw[1] = (self.apid & 0xFF) as u8;
        self.raw[2] = ((self.sequence_flag & 0x03) << 6)
            | (((self.packet_sequence_count >> 8) & 0x3F) as u8);
        self.raw[3] = (self.packet_sequence_count & 0xFF) as u8;
        self.raw[4] = (self.packet_length >> 8) as u8;
        self.raw[5] = (self.packet_length & 0xFF) as u8;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CCSDSPacket {
    pub header: CCSDSHeader,
    pub payload: Vec<u8>,
}

impl CCSDSPacket {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn encode_hdr(&mut self) {
        self.header.packet_length = self.payload.len().saturating_sub(1) as u16;
        self.header.encode_hdr();
    }
}

pub fn parse_ccsds_header(header: &[u8]) -> CCSDSHeader {
    CCSDSHeader::from_bytes(header)
}

pub fn crc_check_ccitt(pkt: &CCSDSPacket) -> bool {
    let payload = &pkt.payload;
    if payload.len() < 2 {
        return false;
    }
    let crc2 = ((payload[payload.len() - 2] as u16) << 8) | (payload[payload.len() - 1] as u16);
    let mut crc: u16 = 0xFFFF;
    const CCITT_CRC_GEN: u16 = 0x1021;
    for j in 0..(payload.len() + 6 - 2) {
        let val = if j < 6 {
            pkt.header.raw[j]
        } else {
            payload[j - 6]
        };
        let mut data_byte = (val as u16) << 8;
        for _ in 0..8 {
            if (data_byte ^ crc) & 0x8000 != 0 {
                crc = (crc << 1) ^ CCITT_CRC_GEN;
            } else {
                crc <<= 1;
            }
            data_byte <<= 1;
        }
    }
    crc == crc2
}

pub fn crc_check_hdlc32(pkt: &CCSDSPacket) -> bool {
    let payload = &pkt.payload;
    if payload.len() < 4 {
        return false;
    }
    let crc2 = ((payload[payload.len() - 4] as u32) << 24)
        | ((payload[payload.len() - 3] as u32) << 16)
        | ((payload[payload.len() - 2] as u32) << 8)
        | (payload[payload.len() - 1] as u32);
    let mut crc: u32 = 0xFFFFFFFF;
    const HDLC_CRC_GEN: u32 = 0xEDB88320;
    for j in 0..(payload.len() + 6 - 4) {
        let val = if j < 6 {
            pkt.header.raw[j]
        } else {
            payload[j - 6]
        };
        let mut data_byte = (val as u32) << 8;
        for _ in 0..8 {
            if (data_byte ^ crc) & 0x80000000 != 0 {
                crc = (crc << 1) ^ HDLC_CRC_GEN;
            } else {
                crc <<= 1;
            }
            data_byte <<= 1;
        }
    }
    (crc ^ 0xFFFFFFFF) == crc2
}

pub fn crc_check_vertical_parity(pkt: &CCSDSPacket) -> bool {
    let payload = &pkt.payload;
    if payload.len() < 2 {
        return false;
    }
    let crc2 = ((payload[payload.len() - 2] as u16) << 8) | (payload[payload.len() - 1] as u16);
    let mut checksum: u16 = 0;
    for j in 0..((payload.len() + 6 - 2) / 2) {
        let j1 = j * 2;
        let j2 = j1 + 1;
        let val1 = if j1 < 6 {
            pkt.header.raw[j1]
        } else {
            payload[j1 - 6]
        };
        let val2 = if j2 < 6 {
            pkt.header.raw[j2]
        } else {
            payload[j2 - 6]
        };
        checksum ^= ((val1 as u16) << 8) | (val2 as u16);
    }
    crc2 == checksum
}
