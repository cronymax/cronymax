//! Protobuf Frame Codec for Lark WebSocket protocol.
//!
//! Minimal manual protobuf encode/decode for the Lark `pbbp2.Frame` message.
//! Only two message types are used (Frame, Header), so a full prost dependency
//! is avoided.
//!
//! Frame proto (field numbers from Lark pbbp2 proto definition):
//!   SeqID:            uint64          (field 1, varint)
//!   LogID:            uint64          (field 2, varint)
//!   service:          int32           (field 3, varint)
//!   method:           int32           (field 4, varint)  — 0=Control, 1=Data
//!   headers:          repeated Header (field 5, length-delimited embedded)
//!   payload_encoding: string          (field 6, length-delimited)
//!   payload_type:     string          (field 7, length-delimited)
//!   payload:          bytes           (field 8, length-delimited)
//!   LogIDNew:         string          (field 9, length-delimited)
//!
//! Header proto:
//!   key: string   (field 1, length-delimited)
//!   value: string (field 2, length-delimited)

use std::collections::HashMap;

/// A single protobuf frame on the Lark WebSocket wire.
#[derive(Debug, Clone, Default)]
pub(super) struct Frame {
    /// Sequence ID (auto-increment per connection).
    pub(super) seq_id: u64,
    /// Log/trace ID (legacy uint64 form).
    pub(super) log_id: u64,
    /// Service ID from the endpoint URL.
    pub(super) service: u32,
    /// 0 = control (ping/pong), 1 = data (event).
    pub(super) method: u32,
    pub(super) headers: HashMap<String, String>,
    /// Optional: "gzip" if payload is gzip-compressed.
    pub(super) payload_encoding: Vec<u8>,
    /// Optional: payload MIME type hint.
    pub(super) payload_type: Vec<u8>,
    /// Event payload (JSON, possibly gzip-compressed).
    pub(super) payload: Vec<u8>,
    /// Log/trace ID (new string form).
    pub(super) log_id_new: String,
}

impl Frame {
    /// Decode a Frame from raw protobuf bytes.
    pub(super) fn decode(data: &[u8]) -> anyhow::Result<Self> {
        let mut frame = Frame::default();
        let mut pos = 0;
        while pos < data.len() {
            let (tag, new_pos) = read_varint(data, pos)?;
            pos = new_pos;
            let field_number = (tag >> 3) as u32;
            let wire_type = (tag & 7) as u32;

            match (field_number, wire_type) {
                // SeqID: varint (field 1)
                (1, 0) => {
                    let (v, p) = read_varint(data, pos)?;
                    frame.seq_id = v;
                    pos = p;
                }
                // LogID: varint (field 2)
                (2, 0) => {
                    let (v, p) = read_varint(data, pos)?;
                    frame.log_id = v;
                    pos = p;
                }
                // service: varint (field 3)
                (3, 0) => {
                    let (v, p) = read_varint(data, pos)?;
                    frame.service = v as u32;
                    pos = p;
                }
                // method: varint (field 4)
                (4, 0) => {
                    let (v, p) = read_varint(data, pos)?;
                    frame.method = v as u32;
                    pos = p;
                }
                // headers: repeated embedded message (field 5)
                (5, 2) => {
                    let (bytes, p) = read_bytes(data, pos)?;
                    pos = p;
                    // Decode embedded Header message.
                    let (key, value) = decode_header(&bytes)?;
                    frame.headers.insert(key, value);
                }
                // payload_encoding: string (field 6)
                (6, 2) => {
                    let (bytes, p) = read_bytes(data, pos)?;
                    frame.payload_encoding = bytes;
                    pos = p;
                }
                // payload_type: string (field 7)
                (7, 2) => {
                    let (bytes, p) = read_bytes(data, pos)?;
                    frame.payload_type = bytes;
                    pos = p;
                }
                // payload: bytes (field 8)
                (8, 2) => {
                    let (bytes, p) = read_bytes(data, pos)?;
                    frame.payload = bytes;
                    pos = p;
                }
                // LogIDNew: string (field 9)
                (9, 2) => {
                    let (bytes, p) = read_bytes(data, pos)?;
                    frame.log_id_new = String::from_utf8_lossy(&bytes).to_string();
                    pos = p;
                }
                // Unknown field — skip.
                (_, 0) => {
                    let (_v, p) = read_varint(data, pos)?;
                    pos = p;
                }
                (_, 2) => {
                    let (_bytes, p) = read_bytes(data, pos)?;
                    pos = p;
                }
                (_, 5) => pos += 4, // 32-bit
                (_, 1) => pos += 8, // 64-bit
                _ => anyhow::bail!("Unknown wire type {} at pos {}", wire_type, pos),
            }
        }
        Ok(frame)
    }

    /// Encode this Frame into protobuf bytes.
    ///
    /// All four required proto2 fields (SeqID, LogID, service, method) are
    /// always encoded, even when zero — the Lark server validates their presence.
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        // SeqID (field 1, varint) — REQUIRED: always encode
        write_tag(&mut buf, 1, 0);
        write_varint(&mut buf, self.seq_id);
        // LogID (field 2, varint) — REQUIRED: always encode
        write_tag(&mut buf, 2, 0);
        write_varint(&mut buf, self.log_id);
        // service (field 3, varint) — REQUIRED: always encode
        write_tag(&mut buf, 3, 0);
        write_varint(&mut buf, self.service as u64);
        // method (field 4, varint) — REQUIRED: always encode
        write_tag(&mut buf, 4, 0);
        write_varint(&mut buf, self.method as u64);
        // headers (field 5, repeated embedded)
        for (k, v) in &self.headers {
            let header_bytes = encode_header(k, v);
            write_tag(&mut buf, 5, 2);
            write_varint(&mut buf, header_bytes.len() as u64);
            buf.extend_from_slice(&header_bytes);
        }
        // payload_encoding (field 6, string)
        if !self.payload_encoding.is_empty() {
            write_tag(&mut buf, 6, 2);
            write_varint(&mut buf, self.payload_encoding.len() as u64);
            buf.extend_from_slice(&self.payload_encoding);
        }
        // payload_type (field 7, string)
        if !self.payload_type.is_empty() {
            write_tag(&mut buf, 7, 2);
            write_varint(&mut buf, self.payload_type.len() as u64);
            buf.extend_from_slice(&self.payload_type);
        }
        // payload (field 8, bytes)
        if !self.payload.is_empty() {
            write_tag(&mut buf, 8, 2);
            write_varint(&mut buf, self.payload.len() as u64);
            buf.extend_from_slice(&self.payload);
        }
        // LogIDNew (field 9, string)
        if !self.log_id_new.is_empty() {
            write_tag(&mut buf, 9, 2);
            write_varint(&mut buf, self.log_id_new.len() as u64);
            buf.extend_from_slice(self.log_id_new.as_bytes());
        }
        buf
    }
}

/// Decode a Header (key, value) from embedded protobuf bytes.
fn decode_header(data: &[u8]) -> anyhow::Result<(String, String)> {
    let mut key = String::new();
    let mut value = String::new();
    let mut pos = 0;
    while pos < data.len() {
        let (tag, p) = read_varint(data, pos)?;
        pos = p;
        let field = (tag >> 3) as u32;
        match field {
            1 => {
                let (bytes, p) = read_bytes(data, pos)?;
                key = String::from_utf8_lossy(&bytes).to_string();
                pos = p;
            }
            2 => {
                let (bytes, p) = read_bytes(data, pos)?;
                value = String::from_utf8_lossy(&bytes).to_string();
                pos = p;
            }
            _ => {
                let (_bytes, p) = read_bytes(data, pos)?;
                pos = p;
            }
        }
    }
    Ok((key, value))
}

/// Encode a Header (key, value) into protobuf bytes.
fn encode_header(key: &str, value: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    // key (field 1, string)
    write_tag(&mut buf, 1, 2);
    write_varint(&mut buf, key.len() as u64);
    buf.extend_from_slice(key.as_bytes());
    // value (field 2, string)
    write_tag(&mut buf, 2, 2);
    write_varint(&mut buf, value.len() as u64);
    buf.extend_from_slice(value.as_bytes());
    buf
}

// ─── Varint helpers ──────────────────────────────────────────────────────────

fn read_varint(data: &[u8], start: usize) -> anyhow::Result<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    let mut pos = start;
    loop {
        if pos >= data.len() {
            anyhow::bail!("Unexpected end of data reading varint at pos {}", start);
        }
        let byte = data[pos];
        pos += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok((result, pos));
        }
        shift += 7;
        if shift >= 64 {
            anyhow::bail!("Varint overflow at pos {}", start);
        }
    }
}

fn read_bytes(data: &[u8], start: usize) -> anyhow::Result<(Vec<u8>, usize)> {
    let (len, pos) = read_varint(data, start)?;
    let len = len as usize;
    if pos + len > data.len() {
        anyhow::bail!(
            "Not enough data for length-delimited field: need {} bytes at pos {}, have {}",
            len,
            pos,
            data.len() - pos
        );
    }
    Ok((data[pos..pos + len].to_vec(), pos + len))
}

fn write_tag(buf: &mut Vec<u8>, field_number: u32, wire_type: u32) {
    write_varint(buf, ((field_number << 3) | wire_type) as u64);
}

fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}
