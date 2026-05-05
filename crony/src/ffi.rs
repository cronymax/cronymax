//! C FFI surface used by the CEF host.
//!
//! Two surfaces live here:
//!
//! * **In-process lifecycle** (`crony_boot` / `crony_shutdown` / etc.)
//!   — used by the standalone runtime binary's host side and for
//!   diagnostic/embedding scenarios.
//! * **Out-of-process client** (`crony_client_*`) — used by the CEF
//!   host to talk to a separately-spawned `cronymax-runtime` over
//!   GIPS without re-implementing gips' wire format in C++. This is
//!   the surface task 1.1 of the `rust-runtime-cpp-cutover` change
//!   adds.
//!
//! All `crony_client_*` functions are blocking and serialize
//! send/recv through an internal mutex. Callers (the C++
//! `RuntimeProxy`) are expected to own concurrency policy: typically
//! a dedicated recv pump thread plus request threads that call
//! `crony_client_send`.

use std::ffi::{c_char, c_int, CStr, CString};
use std::ptr;
use std::slice;

use cronymax::RuntimeConfig;
use gips::ipc::Endpoint;
use parking_lot::Mutex;

use crate::lifecycle;
use crate::logging;

/// ABI version for the `crony_client_*` surface. Bumped on any
/// signature change. The host should refuse to load if the linked
/// library reports a different version than its header expects.
pub const CRONY_ABI_VERSION: u32 = 1;

/// Status codes shared by the client-side ABI. Negative values are
/// errors; `0` is success.
pub const CRONY_OK: c_int = 0;
pub const CRONY_ERR_NULL: c_int = -1;
pub const CRONY_ERR_UTF8: c_int = -2;
pub const CRONY_ERR_CONNECT: c_int = -10;
pub const CRONY_ERR_SEND: c_int = -11;
pub const CRONY_ERR_RECV: c_int = -12;
pub const CRONY_ERR_CLOSED: c_int = -13;
pub const CRONY_ERR_WOULD_BLOCK: c_int = -14;

/// Returns the ABI version this build advertises. The host should
/// compare against its compiled-in `CRONY_ABI_VERSION` and abort if
/// they differ.
#[no_mangle]
pub extern "C" fn crony_abi_version() -> u32 {
    CRONY_ABI_VERSION
}

/// Boot the runtime from a JSON-encoded `RuntimeConfig`.
///
/// Returns `0` on success. On failure, returns a negative code and, if
/// `out_err` is non-null, writes an owned C string describing the error
/// that the caller must release via `crony_string_free`.
///
/// # Safety
/// `config_json` must be a valid NUL-terminated UTF-8 string. `out_err`
/// may be null or must point to a writable `*mut c_char` slot.
#[no_mangle]
pub unsafe extern "C" fn crony_boot(
    config_json: *const c_char,
    out_err: *mut *mut c_char,
) -> c_int {
    if config_json.is_null() {
        write_err(out_err, "config_json is null");
        return -1;
    }
    let json = match CStr::from_ptr(config_json).to_str() {
        Ok(s) => s,
        Err(e) => {
            write_err(out_err, &format!("config_json not utf-8: {e}"));
            return -2;
        }
    };
    let config: RuntimeConfig = match serde_json::from_str(json) {
        Ok(c) => c,
        Err(e) => {
            write_err(out_err, &format!("config json parse error: {e}"));
            return -3;
        }
    };

    logging::install(config.logging.filter.as_deref());

    match lifecycle::boot(config) {
        Ok(_) => 0,
        Err(e) => {
            write_err(out_err, &format!("runtime boot failed: {e:#}"));
            -4
        }
    }
}

/// Stop the runtime. Safe to call before `crony_boot`.
#[no_mangle]
pub extern "C" fn crony_shutdown() {
    lifecycle::shutdown();
}

/// Returns `1` if the runtime is started and running, `0` otherwise.
#[no_mangle]
pub extern "C" fn crony_is_healthy() -> c_int {
    if lifecycle::is_healthy() {
        1
    } else {
        0
    }
}

/// Returns the protocol version this build advertises, packed as
/// `(major << 32) | (minor << 16) | patch`.
#[no_mangle]
pub extern "C" fn crony_protocol_version() -> u64 {
    let v = cronymax::PROTOCOL_VERSION;
    ((v.major as u64) << 32) | ((v.minor as u64) << 16) | (v.patch as u64)
}

/// Free a string returned by this library.
///
/// # Safety
/// `s` must have been returned by a `crony_*` function and not freed yet.
#[no_mangle]
pub unsafe extern "C" fn crony_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(CString::from_raw(s));
}

unsafe fn write_err(out_err: *mut *mut c_char, msg: &str) {
    if out_err.is_null() {
        return;
    }
    match CString::new(msg) {
        Ok(c) => *out_err = c.into_raw(),
        Err(_) => *out_err = std::ptr::null_mut(),
    }
}

// ---------------------------------------------------------------------------
// Out-of-process client surface (rust-runtime-cpp-cutover task 1.1).
// ---------------------------------------------------------------------------

/// Opaque GIPS client handle. Owns one `gips::ipc::Endpoint`.
///
/// `crony_client_t` is a forward declaration in the C header; the
/// concrete layout is private to Rust.
#[allow(non_camel_case_types)]
pub struct crony_client_t {
    inner: Mutex<Option<Endpoint>>,
}

impl crony_client_t {
    fn new(endpoint: Endpoint) -> Self {
        Self {
            inner: Mutex::new(Some(endpoint)),
        }
    }
}

impl std::fmt::Debug for crony_client_t {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("crony_client_t")
            .field("connected", &self.inner.lock().is_some())
            .finish()
    }
}

/// Connect to a runtime GIPS service. Returns NULL on failure; if
/// `out_err` is non-null and a string error is available, writes an
/// owned C string to `*out_err` that the caller must release via
/// `crony_string_free`.
///
/// # Safety
/// `service_name` must be a valid NUL-terminated UTF-8 string.
/// `out_err` may be null or must point to a writable `*mut c_char`
/// slot.
#[no_mangle]
pub unsafe extern "C" fn crony_client_new(
    service_name: *const c_char,
    out_err: *mut *mut c_char,
) -> *mut crony_client_t {
    if service_name.is_null() {
        write_err(out_err, "service_name is null");
        return ptr::null_mut();
    }
    let name = match CStr::from_ptr(service_name).to_str() {
        Ok(s) => s,
        Err(e) => {
            write_err(out_err, &format!("service_name not utf-8: {e}"));
            return ptr::null_mut();
        }
    };
    match Endpoint::connect(name) {
        Ok(endpoint) => Box::into_raw(Box::new(crony_client_t::new(endpoint))),
        Err(e) => {
            write_err(out_err, &format!("gips connect failed: {e}"));
            ptr::null_mut()
        }
    }
}

/// Send a payload (typically a JSON-encoded `ClientToRuntime`
/// envelope) to the runtime. Blocks until gips accepts the frame or
/// returns an error.
///
/// Returns `CRONY_OK` (0) on success, or a negative error code. On
/// non-`CRONY_ERR_NULL` errors and if `out_err` is non-null, writes
/// a string description that the caller must free via
/// `crony_string_free`.
///
/// # Safety
/// `client` must be a non-null handle returned by
/// `crony_client_new` and not yet closed. `payload` must be a
/// readable buffer of length `payload_len`. `out_err` may be null.
#[no_mangle]
pub unsafe extern "C" fn crony_client_send(
    client: *mut crony_client_t,
    payload: *const u8,
    payload_len: usize,
    out_err: *mut *mut c_char,
) -> c_int {
    if client.is_null() || (payload.is_null() && payload_len != 0) {
        write_err(out_err, "client or payload is null");
        return CRONY_ERR_NULL;
    }
    let bytes = if payload_len == 0 {
        &[][..]
    } else {
        slice::from_raw_parts(payload, payload_len)
    };
    let client_ref = &*client;
    let mut guard = client_ref.inner.lock();
    let endpoint = match guard.as_mut() {
        Some(ep) => ep,
        None => {
            write_err(out_err, "client is closed");
            return CRONY_ERR_CLOSED;
        }
    };
    match endpoint.send(bytes, &[]) {
        Ok(()) => CRONY_OK,
        Err(e) => {
            write_err(out_err, &format!("gips send failed: {e}"));
            CRONY_ERR_SEND
        }
    }
}

/// Receive one payload from the runtime. Blocks until a frame
/// arrives or the connection is closed.
///
/// On success, allocates a buffer for the payload and writes its
/// pointer to `*out_buf` and length to `*out_len`. The caller must
/// release the buffer via `crony_bytes_free(buf, len)`.
///
/// Returns `CRONY_OK` on success or a negative error code on
/// failure. On error the output buffer is set to NULL and
/// `*out_err`, if non-null, receives a string description (free
/// via `crony_string_free`).
///
/// # Safety
/// `client` must be a non-null handle returned by
/// `crony_client_new` and not yet closed. `out_buf` and `out_len`
/// must be writable pointers. `out_err` may be null.
#[no_mangle]
pub unsafe extern "C" fn crony_client_recv(
    client: *mut crony_client_t,
    out_buf: *mut *mut u8,
    out_len: *mut usize,
    out_err: *mut *mut c_char,
) -> c_int {
    if client.is_null() || out_buf.is_null() || out_len.is_null() {
        write_err(out_err, "client/out_buf/out_len is null");
        return CRONY_ERR_NULL;
    }
    *out_buf = ptr::null_mut();
    *out_len = 0;

    let client_ref = &*client;
    let mut guard = client_ref.inner.lock();
    let endpoint = match guard.as_mut() {
        Some(ep) => ep,
        None => {
            write_err(out_err, "client is closed");
            return CRONY_ERR_CLOSED;
        }
    };
    match endpoint.recv() {
        Ok(pod) => {
            // Box::into_raw on a boxed slice gives us a stable ptr +
            // length we hand to the host. The host returns both to
            // `crony_bytes_free`, which reconstructs the boxed slice.
            let boxed: Box<[u8]> = pod.payload.into_boxed_slice();
            let len = boxed.len();
            let raw = Box::into_raw(boxed) as *mut u8;
            *out_buf = raw;
            *out_len = len;
            CRONY_OK
        }
        Err(e) => {
            write_err(out_err, &format!("gips recv failed: {e}"));
            CRONY_ERR_RECV
        }
    }
}

/// Free a buffer returned by `crony_client_recv`.
///
/// # Safety
/// `buf` must have been returned by `crony_client_recv` paired with
/// the same `len`, and must not have been freed already.
#[no_mangle]
pub unsafe extern "C" fn crony_bytes_free(buf: *mut u8, len: usize) {
    if buf.is_null() || len == 0 {
        return;
    }
    let slice = slice::from_raw_parts_mut(buf, len);
    drop(Box::from_raw(slice as *mut [u8]));
}

/// Non-blocking variant of `crony_client_recv`. Returns `CRONY_ERR_WOULD_BLOCK`
/// immediately if no message is available, instead of blocking.
/// All other semantics are identical to `crony_client_recv`.
///
/// This allows the pump thread to yield the endpoint lock between polls so that
/// concurrent `crony_client_send` calls are never starved.
///
/// # Safety
/// Same as `crony_client_recv`.
#[no_mangle]
pub unsafe extern "C" fn crony_client_try_recv(
    client: *mut crony_client_t,
    out_buf: *mut *mut u8,
    out_len: *mut usize,
    out_err: *mut *mut c_char,
) -> c_int {
    if client.is_null() || out_buf.is_null() || out_len.is_null() {
        write_err(out_err, "client/out_buf/out_len is null");
        return CRONY_ERR_NULL;
    }
    *out_buf = ptr::null_mut();
    *out_len = 0;

    let client_ref = &*client;
    let mut guard = client_ref.inner.lock();
    let endpoint = match guard.as_mut() {
        Some(ep) => ep,
        None => {
            write_err(out_err, "client is closed");
            return CRONY_ERR_CLOSED;
        }
    };
    match endpoint.try_recv() {
        Ok(msg) => {
            let boxed: Box<[u8]> = msg.payload.into_boxed_slice();
            let len = boxed.len();
            let raw = Box::into_raw(boxed) as *mut u8;
            *out_buf = raw;
            *out_len = len;
            CRONY_OK
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => CRONY_ERR_WOULD_BLOCK,
        Err(e) => {
            write_err(out_err, &format!("gips try_recv failed: {e}"));
            CRONY_ERR_RECV
        }
    }
}

/// Close and free a client handle. Safe to call with NULL. After
/// this call the handle must not be used again.
///
/// # Safety
/// `client` must be a handle returned by `crony_client_new` and not
/// already freed.
#[no_mangle]
pub unsafe extern "C" fn crony_client_close(client: *mut crony_client_t) {
    if client.is_null() {
        return;
    }
    let owned = Box::from_raw(client);
    // Drop the endpoint while holding the lock so any concurrent
    // send/recv that happens to be parked on the mutex returns
    // `CRONY_ERR_CLOSED` after we release.
    *owned.inner.lock() = None;
    drop(owned);
}
