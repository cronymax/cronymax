//! End-to-end test of the `crony_client_*` C ABI.
//!
//! Boots a `GipsTransport`-backed dispatch loop on a unique service
//! name, then drives the FFI surface (`crony_client_new`,
//! `crony_client_send`, `crony_client_recv`, `crony_client_close`)
//! the same way the C++ `RuntimeProxy` will, and asserts a real
//! Hello/Welcome + Ping/Pong round-trip succeeds.
//!
//! This is the design's "exercise the ABI from a Rust unit test"
//! mitigation for ABI drift between the Rust gips client and the
//! C surface the host links against.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crony::boundary::GipsTransport;
use crony::ffi::{
    crony_abi_version, crony_bytes_free, crony_client_close, crony_client_new,
    crony_client_recv, crony_client_send, crony_string_free, CRONY_ABI_VERSION,
    CRONY_OK,
};
use cronymax::protocol::control::{ControlRequest, ControlResponse};
use cronymax::protocol::dispatch::{run, EchoHandler};
use cronymax::protocol::envelope::{ClientToRuntime, CorrelationId, RuntimeToClient};
use cronymax::protocol::version::PROTOCOL_VERSION;

fn unique_service() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    format!("ai.cronymax.runtime.cabi_{}_{}", std::process::id(), nanos)
}

/// Round-trip a `ClientToRuntime` envelope through the C ABI and
/// return the decoded `RuntimeToClient`.
unsafe fn cabi_round_trip(
    client: *mut crony::ffi::crony_client_t,
    msg: &ClientToRuntime,
) -> RuntimeToClient {
    let payload = serde_json::to_vec(msg).expect("serialize");
    let mut err: *mut c_char = ptr::null_mut();
    let send_rc = crony_client_send(
        client,
        payload.as_ptr(),
        payload.len(),
        &mut err as *mut *mut c_char,
    );
    if send_rc != CRONY_OK {
        let msg = if err.is_null() {
            "(no detail)".to_string()
        } else {
            CStr::from_ptr(err).to_string_lossy().into_owned()
        };
        if !err.is_null() {
            crony_string_free(err);
        }
        panic!("crony_client_send rc={send_rc}: {msg}");
    }

    let mut buf: *mut u8 = ptr::null_mut();
    let mut len: usize = 0;
    let mut err: *mut c_char = ptr::null_mut();
    let recv_rc = crony_client_recv(
        client,
        &mut buf as *mut *mut u8,
        &mut len as *mut usize,
        &mut err as *mut *mut c_char,
    );
    if recv_rc != CRONY_OK {
        let msg = if err.is_null() {
            "(no detail)".to_string()
        } else {
            CStr::from_ptr(err).to_string_lossy().into_owned()
        };
        if !err.is_null() {
            crony_string_free(err);
        }
        panic!("crony_client_recv rc={recv_rc}: {msg}");
    }
    let bytes = std::slice::from_raw_parts(buf, len).to_vec();
    crony_bytes_free(buf, len);
    serde_json::from_slice(&bytes).expect("parse RuntimeToClient")
}

#[test]
fn abi_version_constant_is_stable() {
    assert_eq!(crony_abi_version(), CRONY_ABI_VERSION);
    assert_eq!(CRONY_ABI_VERSION, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c_abi_round_trips_through_gips() {
    let service = unique_service();
    let transport = GipsTransport::bind(service.as_str())
        .expect("bind GIPS listener");

    let dispatch_task = tokio::spawn(async move {
        run(transport, EchoHandler).await
    });

    let service_name = service.clone();
    let client_task = tokio::task::spawn_blocking(move || unsafe {
        // Give the listener a beat to register, mirroring the
        // existing gips_transport.rs test.
        std::thread::sleep(Duration::from_millis(50));

        let cname = CString::new(service_name).expect("service name not nul-free");
        let mut err: *mut c_char = ptr::null_mut();
        let client = crony_client_new(cname.as_ptr(), &mut err as *mut *mut c_char);
        if client.is_null() {
            let msg = if err.is_null() {
                "(no detail)".to_string()
            } else {
                CStr::from_ptr(err).to_string_lossy().into_owned()
            };
            if !err.is_null() {
                crony_string_free(err);
            }
            panic!("crony_client_new returned NULL: {msg}");
        }

        // Hello → Welcome.
        let hello = ClientToRuntime::Hello {
            protocol: PROTOCOL_VERSION,
            client_name: "crony-cabi-test".into(),
            client_version: "0.0.0".into(),
        };
        match cabi_round_trip(client, &hello) {
            RuntimeToClient::Welcome { protocol, .. } => {
                assert_eq!(protocol, PROTOCOL_VERSION);
            }
            other => panic!("expected Welcome, got {other:?}"),
        }

        // Ping → Pong.
        let id = CorrelationId::new();
        let ping = ClientToRuntime::Control {
            id,
            request: ControlRequest::Ping,
        };
        match cabi_round_trip(client, &ping) {
            RuntimeToClient::Control {
                id: rid,
                response: ControlResponse::Pong,
            } => assert_eq!(rid, id),
            other => panic!("expected Pong, got {other:?}"),
        }

        crony_client_close(client);
    });

    client_task.await.expect("client task panicked");

    dispatch_task.abort();
    let _ = dispatch_task.await;
}
