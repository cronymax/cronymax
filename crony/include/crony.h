/*
 * crony.h — C ABI for the cronymax runtime.
 *
 * Authored as part of the `rust-runtime-cpp-cutover` change (task 1.1
 * / 1.3). The full implementation lives in `crony/src/ffi.rs`. This
 * header is the only contract the C++ host (`app/runtime_bridge/`)
 * compiles against; the Rust side stays free to evolve internally.
 *
 * Two surfaces are exposed:
 *
 *   1. In-process lifecycle  (`crony_boot`, `crony_shutdown`, ...).
 *      Used when the runtime is embedded in the same process — for
 *      example by the standalone `cronymax-runtime` binary's host
 *      side or for diagnostic embedding scenarios.
 *
 *   2. Out-of-process client (`crony_client_*`). Used by the CEF
 *      host to talk to a separately-spawned `cronymax-runtime`
 *      process over GIPS without re-implementing the wire format
 *      in C++.
 *
 * Threading notes for the client surface:
 *
 *   * `crony_client_send` and `crony_client_recv` serialize through
 *     an internal mutex on the same handle. A `recv` parked waiting
 *     for the next frame will block any `send` on the same handle.
 *     The host (`RuntimeProxy`) owns concurrency policy: typically
 *     a dedicated recv pump thread plus request threads that call
 *     send.
 *   * `crony_client_close` is safe to call from any thread; it
 *     releases the underlying gips endpoint, which causes the
 *     parked `recv` to unblock with `CRONY_ERR_CLOSED` once the
 *     mutex is reacquired.
 */

#ifndef CRONY_H_
#define CRONY_H_

#include <cstdint>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * ABI version. The host must compare `crony_abi_version()` against
 * this constant at startup and refuse to run if they differ. Bumped
 * on any signature change in this header.
 */
constexpr uint32_t CRONY_ABI_VERSION = 1u;

/* Status codes shared by `crony_client_*`. Negative values are
 * errors; 0 is success. */
constexpr int CRONY_OK = 0;
constexpr int CRONY_ERR_NULL = -1;
constexpr int CRONY_ERR_UTF8 = -2;
constexpr int CRONY_ERR_CONNECT = -10;
constexpr int CRONY_ERR_SEND = -11;
constexpr int CRONY_ERR_RECV = -12;
constexpr int CRONY_ERR_CLOSED = -13;
constexpr int CRONY_ERR_WOULD_BLOCK = -14;

/* ---------- ABI version ---------- */

uint32_t crony_abi_version(void);

/* ---------- In-process lifecycle ---------- */

/* Boot the runtime in-process from a JSON-encoded `RuntimeConfig`.
 * Returns 0 on success, negative on failure. On failure and if
 * `out_err` is non-null, writes an owned C string describing the
 * error that the caller must release via `crony_string_free`. */
int crony_boot(const char* config_json, char** out_err);

/* Stop the in-process runtime. Safe to call before `crony_boot`. */
void crony_shutdown(void);

/* Returns 1 if the runtime is started and running, 0 otherwise. */
int crony_is_healthy(void);

/* Returns the protocol version this build advertises, packed as
 * (major << 32) | (minor << 16) | patch. */
uint64_t crony_protocol_version(void);

/* Free a string returned by any `crony_*` function. */
void crony_string_free(char* s);

/* ---------- Out-of-process GIPS client ---------- */

typedef struct crony_client_t crony_client_t;

/* Connect to a runtime GIPS service. Returns NULL on failure; if
 * `out_err` is non-null and a string is available, writes an owned
 * C string to *out_err that the caller must free via
 * `crony_string_free`. */
crony_client_t* crony_client_new(const char* service_name, char** out_err);

/* Send a payload (typically a JSON-encoded `ClientToRuntime`
 * envelope) to the runtime. Blocks until gips accepts the frame or
 * returns an error. Returns CRONY_OK on success. */
int crony_client_send(crony_client_t* client,
                      const uint8_t* payload,
                      size_t payload_len,
                      char** out_err);

/* Receive one payload from the runtime. Blocks until a frame
 * arrives or the connection is closed. On success, allocates a
 * buffer for the payload and writes its pointer to *out_buf and
 * length to *out_len. Caller must release the buffer with
 * `crony_bytes_free(buf, len)`. Returns CRONY_OK on success. */
int crony_client_recv(crony_client_t* client,
                      uint8_t** out_buf,
                      size_t* out_len,
                      char** out_err);

/* Free a buffer returned by `crony_client_recv`. */
void crony_bytes_free(uint8_t* buf, size_t len);

/* Non-blocking variant of `crony_client_recv`. Returns
 * CRONY_ERR_WOULD_BLOCK immediately if no message is available,
 * CRONY_OK (with buffer) if one was ready, or another error code.
 * The pump thread should use this so it never holds the endpoint
 * lock long enough to starve concurrent `crony_client_send` calls. */
int crony_client_try_recv(crony_client_t* client,
                          uint8_t** out_buf,
                          size_t* out_len,
                          char** out_err);

/* Close and free a client handle. Safe to call with NULL. After
 * this call the handle must not be used again. */
void crony_client_close(crony_client_t* client);

#ifdef __cplusplus
}  /* extern "C" */
#endif

#endif  /* CRONY_H_ */
