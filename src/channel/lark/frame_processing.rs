//! Frame payload processing helpers for the Lark WebSocket channel.

use std::collections::HashMap;

use super::protocol::Frame;

/// Attempt to reassemble a potentially fragmented event payload.
///
/// Returns `Some(payload_bytes)` when all fragments have arrived (or for
/// single-frame events), `None` when still waiting for more fragments.
pub(super) fn reassemble_payload(
    frame: &Frame,
    fragments: &mut HashMap<String, (u32, HashMap<u32, Vec<u8>>)>,
) -> Option<Vec<u8>> {
    let message_id = frame.headers.get("message_id").cloned().unwrap_or_default();
    let sum: u32 = frame
        .headers
        .get("sum")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let seq: u32 = frame
        .headers
        .get("seq")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if sum > 1 {
        // Multi-frame event — collect fragments.
        let entry = fragments
            .entry(message_id.clone())
            .or_insert_with(|| (sum, HashMap::new()));
        entry.1.insert(seq, frame.payload.clone());

        if entry.1.len() == sum as usize {
            // All fragments received — reassemble.
            let mut assembled = Vec::new();
            for i in 0..sum {
                if let Some(chunk) = entry.1.get(&i) {
                    assembled.extend_from_slice(chunk);
                }
            }
            fragments.remove(&message_id);
            Some(assembled)
        } else {
            None // waiting for more fragments
        }
    } else {
        // Single-frame event.
        Some(frame.payload.clone())
    }
}

/// Decompress payload if the frame's `payload_encoding` indicates gzip.
pub(super) fn maybe_decompress(frame: &Frame, payload_bytes: Vec<u8>) -> Vec<u8> {
    let encoding = String::from_utf8_lossy(&frame.payload_encoding);
    if encoding == "gzip" {
        use std::io::Read;
        let mut decoder = flate2::read::GzDecoder::new(&payload_bytes[..]);
        let mut decompressed = Vec::new();
        match decoder.read_to_end(&mut decompressed) {
            Ok(_) => {
                log::info!(
                    "Lark WS: decompressed gzip payload {} -> {} bytes",
                    payload_bytes.len(),
                    decompressed.len()
                );
                decompressed
            }
            Err(e) => {
                log::error!("Lark WS: gzip decompress failed: {}", e);
                payload_bytes
            }
        }
    } else {
        payload_bytes
    }
}
