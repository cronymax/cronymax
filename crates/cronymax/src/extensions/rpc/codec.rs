//! MessagePack-RPC frame codec.
//!
//! Wire format: **bare MessagePack array frames** — no length prefix, no
//! envelope. Validated by the Phase 0 spike (`docs/extensions/msgpack-rpc-spike.md`).
//!
//! Three frame types per the MessagePack-RPC spec
//! (<https://github.com/msgpack-rpc/msgpack-rpc/blob/master/spec.md>):
//!
//! * `[0, msgid, method, params]` — Request
//! * `[1, msgid, error, result]` — Response (one of error/result is `nil`)
//! * `[2, method, params]` — Notify
//!
//! Cronymax adds **one cancellation notify** for in-flight requests
//! (spec §6.1.1 calls it out without pinning the name):
//!
//! * `[2, "$/cancel", [msgid]]` — sender asks the receiver to drop the
//!   reply for `msgid`. See [`crate::extensions::rpc::server`].

use rmpv::Value;
use serde::{Deserialize, Serialize};

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// MessagePack-RPC type tag for the request envelope.
pub const TYPE_REQUEST: i64 = 0;
pub const TYPE_RESPONSE: i64 = 1;
pub const TYPE_NOTIFY: i64 = 2;

/// Cancellation notify method name. Receiving this on the platform side
/// means "drop the response for msgid X if it hasn't been sent yet".
pub const METHOD_CANCEL: &str = "$/cancel";

/// Reserved RPC method names. The host module dispatches these by name.
pub mod method {
    pub const EXTENSION_ACTIVATE: &str = "extension/activate";
    pub const EXTENSION_DEACTIVATE: &str = "extension/deactivate";
    /// Platform → extension reverse notify sent when a register
    /// notify (agents / commands / renderers / sidebar) fails. Payload
    /// is `{ ep: string, id: string, reason: string }`. The bootstrap.js
    /// SDK shim turns it into a `console.error` so developers see the
    /// failure instead of silently dropping.
    pub const EXTENSION_REGISTER_ERROR: &str = "extension/registerError";
    pub const COMMANDS_REGISTER: &str = "commands/register";
    pub const COMMANDS_UNREGISTER: &str = "commands/unregister";
    pub const COMMANDS_EXECUTE: &str = "commands/execute";
    pub const EVENTS_PUBLISH: &str = "events/publish";
    pub const EVENTS_SUBSCRIBE: &str = "events/subscribe";
    pub const LOG_CONSOLE: &str = "log/console";
    pub const LOG_CHANNEL: &str = "log/channel";
    pub const PING: &str = "$/ping";
    pub const READY: &str = "$/ready";
    pub const AUDIT: &str = "audit";
}

/// Method names for the `cronymax.content.renderer` L2 EP (extension →
/// platform notify on register / unregister).
pub mod renderers_method {
    pub const REGISTER: &str = "renderers/register";
    pub const UNREGISTER: &str = "renderers/unregister";
}

/// Method names for the `cronymax.ui.sidebar.view` L2 EP (extension →
/// platform notify on register / unregister).
pub mod sidebar_method {
    pub const REGISTER: &str = "sidebar/register";
    pub const UNREGISTER: &str = "sidebar/unregister";
}

/// Method names for the `cronymax.agents.provider` L2 EP — both directions.
///
/// **Extension → platform** (notify, called from the extension when its
/// `cronymax.agents.registerProvider(id, impl)` runs):
/// * [`Self::REGISTER_PROVIDER`] / [`Self::UNREGISTER_PROVIDER`]
///
/// **Platform → extension** (request, opens a session and drives a turn):
/// * [`Self::SESSION_CREATE`] — returns `{ sessionId: string }`
/// * [`Self::SESSION_PROMPT`] — opens a turn; extension emits zero or
///   more [`Self::EVENT`] notifies then exactly one
///   [`Self::TURN_DONE`] notify
/// * [`Self::SESSION_RESOLVE_PERMISSION`] — answers a
///   `permissionRequest` event
/// * [`Self::SESSION_DISPOSE`] — drops the session
///
/// **Extension → platform** (notify, during a turn):
/// * [`Self::EVENT`] — one `AgentEvent` per call; the platform looks up
///   the turn by `turnId` and forwards to the chat panel / flow runtime
/// * [`Self::TURN_DONE`] — signals end of stream
pub mod agents_method {
    pub const REGISTER_PROVIDER: &str = "agents/registerProvider";
    pub const UNREGISTER_PROVIDER: &str = "agents/unregisterProvider";
    pub const SESSION_CREATE: &str = "agents/session.create";
    pub const SESSION_PROMPT: &str = "agents/session.prompt";
    pub const SESSION_RESOLVE_PERMISSION: &str = "agents/session.resolvePermission";
    pub const SESSION_DISPOSE: &str = "agents/session.dispose";
    pub const SESSION_CANCEL: &str = "agents/session.cancel";
    pub const EVENT: &str = "agents/event";
    pub const TURN_DONE: &str = "agents/turn.done";
}

/// A decoded MessagePack-RPC frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Request {
        msgid: u32,
        method: String,
        params: Value,
    },
    Response {
        msgid: u32,
        error: Value,
        result: Value,
    },
    Notify {
        method: String,
        params: Value,
    },
}

impl Frame {
    /// Encode this frame to bare MessagePack bytes (no length prefix).
    pub fn encode(&self) -> ExtensionResult<Vec<u8>> {
        match self {
            Frame::Request {
                msgid,
                method,
                params,
            } => {
                let envelope = (TYPE_REQUEST as u8, msgid, method, params);
                serialize_tuple(&envelope)
            }
            Frame::Response {
                msgid,
                error,
                result,
            } => {
                let envelope = (TYPE_RESPONSE as u8, msgid, error, result);
                serialize_tuple(&envelope)
            }
            Frame::Notify { method, params } => {
                let envelope = (TYPE_NOTIFY as u8, method, params);
                serialize_tuple(&envelope)
            }
        }
    }

    /// Try to decode the first complete frame from `buf`. Returns:
    ///
    /// * `Ok(Some((frame, n)))` — decoded one frame; caller should drop the
    ///   first `n` bytes from the buffer.
    /// * `Ok(None)` — buffer doesn't yet contain a full frame; caller
    ///   should read more bytes and retry.
    /// * `Err(_)` — malformed input.
    pub fn decode_one(buf: &[u8]) -> ExtensionResult<Option<(Self, usize)>> {
        let mut cursor = std::io::Cursor::new(buf);
        let value = match rmpv::decode::read_value(&mut cursor) {
            Ok(v) => v,
            Err(e) if is_unexpected_eof(&e) => return Ok(None),
            Err(e) => return Err(rpc_err(format!("decode value: {e}"))),
        };
        let consumed = cursor.position() as usize;
        let frame = Self::from_value(value)?;
        Ok(Some((frame, consumed)))
    }

    fn from_value(v: Value) -> ExtensionResult<Self> {
        let arr = v
            .as_array()
            .ok_or_else(|| rpc_err("frame is not an array"))?;
        if arr.is_empty() {
            return Err(rpc_err("frame array is empty"));
        }
        let ty = arr[0]
            .as_i64()
            .ok_or_else(|| rpc_err("frame type is not an integer"))?;
        match ty {
            TYPE_REQUEST => {
                if arr.len() != 4 {
                    return Err(rpc_err(format!(
                        "request frame must have 4 elements, got {}",
                        arr.len()
                    )));
                }
                Ok(Frame::Request {
                    msgid: as_u32(&arr[1], "request msgid")?,
                    method: as_string(&arr[2], "request method")?,
                    params: arr[3].clone(),
                })
            }
            TYPE_RESPONSE => {
                if arr.len() != 4 {
                    return Err(rpc_err(format!(
                        "response frame must have 4 elements, got {}",
                        arr.len()
                    )));
                }
                Ok(Frame::Response {
                    msgid: as_u32(&arr[1], "response msgid")?,
                    error: arr[2].clone(),
                    result: arr[3].clone(),
                })
            }
            TYPE_NOTIFY => {
                if arr.len() != 3 {
                    return Err(rpc_err(format!(
                        "notify frame must have 3 elements, got {}",
                        arr.len()
                    )));
                }
                Ok(Frame::Notify {
                    method: as_string(&arr[1], "notify method")?,
                    params: arr[2].clone(),
                })
            }
            other => Err(rpc_err(format!("unknown frame type {other}"))),
        }
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn serialize_tuple<T: Serialize>(t: &T) -> ExtensionResult<Vec<u8>> {
    // `rmp_serde::to_vec` (NOT `to_vec_named`) encodes Rust tuples as
    // MessagePack arrays — exactly what MessagePack-RPC needs. Using
    // `to_vec_named` would encode them as maps, breaking interop. See spike
    // §3.2 pit-1.
    rmp_serde::to_vec(t).map_err(|e| rpc_err(format!("encode: {e}")))
}

fn as_u32(v: &Value, what: &str) -> ExtensionResult<u32> {
    let n = v
        .as_u64()
        .ok_or_else(|| rpc_err(format!("{what} is not an unsigned integer")))?;
    u32::try_from(n).map_err(|_| rpc_err(format!("{what} = {n} overflows u32")))
}

fn as_string(v: &Value, what: &str) -> ExtensionResult<String> {
    let s = v
        .as_str()
        .ok_or_else(|| rpc_err(format!("{what} is not a string")))?;
    Ok(s.to_string())
}

fn is_unexpected_eof(err: &rmpv::decode::Error) -> bool {
    if let rmpv::decode::Error::InvalidMarkerRead(io) = err {
        if io.kind() == std::io::ErrorKind::UnexpectedEof {
            return true;
        }
    }
    if let rmpv::decode::Error::InvalidDataRead(io) = err {
        if io.kind() == std::io::ErrorKind::UnexpectedEof {
            return true;
        }
    }
    false
}

fn rpc_err(msg: impl Into<String>) -> ExtensionError {
    ExtensionError::Rpc(msg.into())
}

// Compatibility: a typed `Notify(method, [msgid])` for `$/cancel`. Callers
// that want to encode a cancellation can build the Frame directly; this
// helper makes the intent obvious and tests easier.
pub fn cancel_notify(msgid: u32) -> Frame {
    Frame::Notify {
        method: METHOD_CANCEL.to_string(),
        params: Value::Array(vec![Value::from(msgid)]),
    }
}

/// Decoded cancellation request, if `frame` is a `$/cancel` notify.
pub fn parse_cancel(frame: &Frame) -> Option<u32> {
    match frame {
        Frame::Notify { method, params } if method == METHOD_CANCEL => params
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.as_u64())
            .and_then(|n| u32::try_from(n).ok()),
        _ => None,
    }
}

// Manual Serialize/Deserialize impls aren't needed; we always go through
// `Frame::encode` / `Frame::decode_one`. Keep these aliases for callers that
// want lower-level access.
#[derive(Debug, Serialize, Deserialize)]
pub struct PingArgs;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_request() {
        let f = Frame::Request {
            msgid: 7,
            method: "extension/activate".into(),
            params: Value::Array(vec![Value::String("alice.x".into())]),
        };
        let bytes = f.encode().unwrap();
        let (decoded, consumed) = Frame::decode_one(&bytes).unwrap().unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, f);
    }

    #[test]
    fn roundtrip_response_ok() {
        let f = Frame::Response {
            msgid: 7,
            error: Value::Nil,
            result: Value::String("ok".into()),
        };
        let bytes = f.encode().unwrap();
        let (decoded, _) = Frame::decode_one(&bytes).unwrap().unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn roundtrip_response_error() {
        let f = Frame::Response {
            msgid: 7,
            error: Value::String("boom".into()),
            result: Value::Nil,
        };
        let bytes = f.encode().unwrap();
        let (decoded, _) = Frame::decode_one(&bytes).unwrap().unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn roundtrip_notify() {
        let f = Frame::Notify {
            method: "log/console".into(),
            params: Value::Array(vec![Value::String("hello".into())]),
        };
        let bytes = f.encode().unwrap();
        let (decoded, _) = Frame::decode_one(&bytes).unwrap().unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn cancel_notify_helper_round_trip() {
        let f = cancel_notify(42);
        let bytes = f.encode().unwrap();
        let (decoded, _) = Frame::decode_one(&bytes).unwrap().unwrap();
        assert_eq!(parse_cancel(&decoded), Some(42));
    }

    #[test]
    fn parse_cancel_ignores_other_frames() {
        let req = Frame::Request {
            msgid: 1,
            method: "$/cancel".into(),
            params: Value::Array(vec![Value::from(99u32)]),
        };
        assert!(parse_cancel(&req).is_none(), "request, not notify");
        let ping = Frame::Notify {
            method: "$/ping".into(),
            params: Value::Nil,
        };
        assert!(parse_cancel(&ping).is_none());
    }

    #[test]
    fn decode_one_returns_none_on_partial_frame() {
        let f = Frame::Request {
            msgid: 1,
            method: "x".into(),
            params: Value::Nil,
        };
        let bytes = f.encode().unwrap();
        // Truncate every prefix and ensure either a clean None or a frame
        // (never an error) — partial frames must be a clean "need more".
        for n in 0..bytes.len() {
            let prefix = &bytes[..n];
            match Frame::decode_one(prefix) {
                Ok(None) => {}
                Ok(Some(_)) => panic!("decoded full frame from {n}-byte prefix"),
                Err(e) => panic!("partial frame at len {n} returned error: {e}"),
            }
        }
        // Full buffer decodes.
        assert!(Frame::decode_one(&bytes).unwrap().is_some());
    }

    #[test]
    fn decode_one_handles_multiple_frames_in_buffer() {
        let a = Frame::Notify {
            method: "a".into(),
            params: Value::Nil,
        };
        let b = Frame::Notify {
            method: "b".into(),
            params: Value::Nil,
        };
        let mut joined = a.encode().unwrap();
        joined.extend(b.encode().unwrap());

        let (first, n1) = Frame::decode_one(&joined).unwrap().unwrap();
        assert_eq!(first, a);
        let (second, n2) = Frame::decode_one(&joined[n1..]).unwrap().unwrap();
        assert_eq!(second, b);
        assert_eq!(n1 + n2, joined.len(), "consumed both frames exactly");
    }

    #[test]
    fn decode_rejects_unknown_frame_type() {
        // [9, 1, "x", []] — type 9 is not defined
        let bytes = rmp_serde::to_vec(&(9u8, 1u32, "x", Vec::<u8>::new())).unwrap();
        let err = Frame::decode_one(&bytes).unwrap_err();
        assert!(matches!(err, ExtensionError::Rpc(_)));
    }

    #[test]
    fn decode_rejects_wrong_arity_request() {
        // [0, 1, "x"] — missing params slot
        let bytes = rmp_serde::to_vec(&(0u8, 1u32, "x")).unwrap();
        let err = Frame::decode_one(&bytes).unwrap_err();
        assert!(matches!(err, ExtensionError::Rpc(_)));
    }

    #[test]
    fn decode_rejects_non_array_envelope() {
        // Encode a map instead of an array
        let bytes = rmp_serde::to_vec_named(&std::collections::BTreeMap::from([(
            "type".to_string(),
            0u8,
        )]))
        .unwrap();
        let err = Frame::decode_one(&bytes).unwrap_err();
        assert!(matches!(err, ExtensionError::Rpc(_)));
    }

    #[test]
    fn wire_layout_matches_msgpack_rpc_spec() {
        // Sanity: a request frame produces exactly the MessagePack bytes we
        // expect. Useful as a guard against accidentally switching to
        // `to_vec_named` (which would encode as a map, not an array).
        let f = Frame::Request {
            msgid: 0,
            method: "x".into(),
            params: Value::Nil,
        };
        let bytes = f.encode().unwrap();
        // fixarray (4 elements) = 0x94
        assert_eq!(bytes[0], 0x94, "must start with fixarray-4");
        // next byte: positive fixint 0 (the type tag)
        assert_eq!(bytes[1], 0x00);
        // then positive fixint 0 (msgid)
        assert_eq!(bytes[2], 0x00);
        // then fixstr 1 = 0xa1, then 'x'
        assert_eq!(bytes[3], 0xa1);
        assert_eq!(bytes[4], b'x');
        // then nil = 0xc0
        assert_eq!(bytes[5], 0xc0);
        assert_eq!(bytes.len(), 6);
    }
}
