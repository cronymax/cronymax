//! Authentication primitives — PKCE state machine + device-flow polling.
//!
//! Mirrors `cep-idl/v1/auth.ts`. The HTTP exchanges themselves live at the
//! call sites (extensions provide their own client credentials and
//! endpoints). What this module owns:
//!
//! * [`PkcePair`] — RFC 7636 code verifier + challenge generation
//! * [`AuthState`] — opaque CSRF token used as the OAuth `state` parameter
//! * [`DeviceFlowSession`] — accountant for the polling cadence: respects
//!   `interval`, applies `slow_down`, gives up on `expires_in`
//! * [`SessionStore`] — per-extension session cache (in-memory; persistence
//!   goes through [`super::secrets`] when callers want it)
//!
//! No reqwest dependency is taken on at this layer — the auth flows
//! produce / validate URLs and tokens; making the actual HTTP call is the
//! responsibility of the caller (who already has a reqwest client).

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::extensions::error::{ExtensionError, ExtensionResult};

// ── PKCE ────────────────────────────────────────────────────────────────────

/// RFC 7636 PKCE verifier + challenge pair. The verifier is the secret;
/// the challenge is what you put in the auth URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

impl PkcePair {
    /// Generate a fresh pair using a 32-byte random verifier and the
    /// `S256` challenge method (the only one we support — RFC 7636 §4.2
    /// says S256 is mandatory for any client that can compute SHA-256).
    pub fn new_random() -> Self {
        let verifier_bytes: [u8; 32] = uuid_random_bytes();
        // Encode using URL-safe base64 alphabet without padding (RFC 7636
        // §4.1: must match `[A-Z]/[a-z]/[0-9]/-/./_/~`). We use a
        // dependency-free base64url here to avoid pulling in `base64`.
        let verifier = b64url_encode(&verifier_bytes);
        Self::from_verifier(&verifier)
    }

    /// Build the pair from an externally-supplied verifier (e.g. one
    /// recovered from persistence). The challenge is recomputed.
    pub fn from_verifier(verifier: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = b64url_encode(&hasher.finalize());
        Self {
            verifier: verifier.to_string(),
            challenge,
        }
    }

    pub fn method(&self) -> &'static str {
        "S256"
    }
}

// ── CSRF / state ────────────────────────────────────────────────────────────

/// Opaque OAuth `state` parameter. Use [`Self::new`] when starting an
/// authorization request; verify the redirect's `state` matches with
/// [`Self::verify`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthState(String);

impl AuthState {
    pub fn new() -> Self {
        let bytes: [u8; 16] = uuid_random_bytes_16();
        Self(b64url_encode(&bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant-time equality check.
    pub fn verify(&self, received: &str) -> bool {
        constant_time_eq(self.0.as_bytes(), received.as_bytes())
    }
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Device flow ─────────────────────────────────────────────────────────────

/// Outcome of one device-flow poll iteration. The auth code lives in the
/// provider's response; this enum only enumerates what the polling client
/// should do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DevicePoll {
    /// Token endpoint returned a token. Caller stops polling.
    Authorized,
    /// User hasn't approved yet — wait `interval` seconds and try again.
    Pending,
    /// Provider asked us to back off — bump the interval.
    SlowDown,
    /// User explicitly denied access. Stop polling.
    Denied,
    /// Codes have expired before user approved. Restart the flow.
    Expired,
    /// Other terminal error.
    Error(String),
}

/// State the polling client maintains between iterations.
#[derive(Debug, Clone)]
pub struct DeviceFlowSession {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: Duration,
    /// Wall-clock instant after which this session's `device_code` won't
    /// be honoured by the provider.
    pub expires_at: SystemTime,
}

impl DeviceFlowSession {
    /// Has the session's `device_code` expired according to its
    /// `expires_at`?
    pub fn is_expired_now(&self) -> bool {
        self.is_expired_at(SystemTime::now())
    }

    pub fn is_expired_at(&self, t: SystemTime) -> bool {
        t >= self.expires_at
    }

    /// Apply a `DevicePoll` outcome to the session's polling cadence.
    /// Returns the recommended next-action; the caller is responsible for
    /// actually sleeping for `interval` between polls.
    pub fn apply_poll(&mut self, outcome: &DevicePoll) -> DeviceFlowDecision {
        match outcome {
            DevicePoll::Authorized => DeviceFlowDecision::Done,
            DevicePoll::Pending => DeviceFlowDecision::Wait(self.interval),
            DevicePoll::SlowDown => {
                self.interval = self.interval.saturating_add(Duration::from_secs(5));
                DeviceFlowDecision::Wait(self.interval)
            }
            DevicePoll::Denied => DeviceFlowDecision::Abort("denied".into()),
            DevicePoll::Expired => DeviceFlowDecision::Abort("expired".into()),
            DevicePoll::Error(msg) => DeviceFlowDecision::Abort(msg.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceFlowDecision {
    /// Token in hand — stop polling.
    Done,
    /// Sleep this long, then poll again.
    Wait(Duration),
    /// Stop polling and surface this reason to the user.
    Abort(String),
}

// ── Session store ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSession {
    pub id: String,
    pub ext_id: String,
    pub provider_id: String,
    pub scopes: Vec<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix epoch seconds; `None` means the provider didn't specify
    /// expiry (treat as long-lived).
    pub expires_at: Option<u64>,
    /// Free-form per-provider metadata.
    pub account: serde_json::Value,
}

impl AuthSession {
    pub fn is_expired_now(&self) -> bool {
        self.is_expired_at(now_seconds())
    }

    pub fn is_expired_at(&self, now_secs: u64) -> bool {
        match self.expires_at {
            Some(deadline) => now_secs >= deadline,
            None => false,
        }
    }
}

/// Composite session-store key: `(ext_id, provider_id, session_id)`.
type SessionKey = (String, String, String);

/// In-memory session store. Cheap to share via clone (internal
/// `Arc<RwLock>`).
#[derive(Debug, Default, Clone)]
pub struct SessionStore {
    inner: std::sync::Arc<RwLock<HashMap<SessionKey, AuthSession>>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, session: AuthSession) -> ExtensionResult<()> {
        let key = (
            session.ext_id.clone(),
            session.provider_id.clone(),
            session.id.clone(),
        );
        let mut g = self
            .inner
            .write()
            .map_err(|_| ExtensionError::ManifestInvalid("session store poisoned".into()))?;
        g.insert(key, session);
        Ok(())
    }

    pub fn get(
        &self,
        ext_id: &str,
        provider_id: &str,
        session_id: &str,
    ) -> ExtensionResult<Option<AuthSession>> {
        let g = self
            .inner
            .read()
            .map_err(|_| ExtensionError::ManifestInvalid("session store poisoned".into()))?;
        Ok(g.get(&(
            ext_id.to_string(),
            provider_id.to_string(),
            session_id.to_string(),
        ))
        .cloned())
    }

    pub fn remove(
        &self,
        ext_id: &str,
        provider_id: &str,
        session_id: &str,
    ) -> ExtensionResult<bool> {
        let mut g = self
            .inner
            .write()
            .map_err(|_| ExtensionError::ManifestInvalid("session store poisoned".into()))?;
        Ok(g.remove(&(
            ext_id.to_string(),
            provider_id.to_string(),
            session_id.to_string(),
        ))
        .is_some())
    }

    pub fn list_for(&self, ext_id: &str, provider_id: &str) -> ExtensionResult<Vec<AuthSession>> {
        let g = self
            .inner
            .read()
            .map_err(|_| ExtensionError::ManifestInvalid("session store poisoned".into()))?;
        let mut v: Vec<AuthSession> = g
            .iter()
            .filter(|((ext, prov, _), _)| ext == ext_id && prov == provider_id)
            .map(|(_, s)| s.clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(v)
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn uuid_random_bytes() -> [u8; 32] {
    // Two UUIDs glued together — uuid v4 is already random; this avoids
    // adding `rand` as a workspace dep.
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out
}

fn uuid_random_bytes_16() -> [u8; 16] {
    *uuid::Uuid::new_v4().as_bytes()
}

/// Constant-time byte slice comparison. Both inputs must be the same length;
/// returns false if lengths differ (no early-out).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

/// URL-safe base64 encoding without padding. RFC 7636 §4.1.
fn b64url_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
    }
    out
}

/// Generate a fresh session id (UUID v4 hex). Public so callers can
/// pre-allocate ids before populating a session.
pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().as_simple().to_string()
}

// Keep references to all helper types alive for tests outside the module.
#[allow(dead_code)]
fn _all_types_export_check(
    _: PkcePair,
    _: AuthState,
    _: DeviceFlowSession,
    _: DeviceFlowDecision,
    _: AuthSession,
    _: SessionStore,
    _: Mutex<()>,
) {
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_pair_method_is_s256() {
        let p = PkcePair::new_random();
        assert_eq!(p.method(), "S256");
    }

    #[test]
    fn pkce_pair_challenge_matches_rfc7636_test_vector() {
        // RFC 7636 §B.1 test vector (verifier → S256 challenge):
        //   verifier  = dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk
        //   challenge = E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM
        let p = PkcePair::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn pkce_pair_verifier_is_url_safe() {
        let p = PkcePair::new_random();
        for c in p.verifier.chars() {
            assert!(
                c.is_ascii_alphanumeric() || c == '-' || c == '_',
                "verifier char {c:?} is not url-safe",
            );
        }
        // 32 random bytes → 43-char base64url string.
        assert_eq!(p.verifier.len(), 43);
    }

    #[test]
    fn pkce_pair_random_is_unique() {
        let a = PkcePair::new_random();
        let b = PkcePair::new_random();
        assert_ne!(a, b, "two random pairs must differ");
    }

    #[test]
    fn auth_state_verify_is_constant_time_and_correct() {
        let s = AuthState::new();
        assert!(s.verify(s.as_str()));
        assert!(!s.verify("not the state"));
        // length mismatch still returns false safely
        assert!(!s.verify("short"));
    }

    #[test]
    fn auth_state_is_url_safe_and_long_enough() {
        let s = AuthState::new();
        // 16 random bytes → 22-char base64url string (no padding).
        assert_eq!(s.as_str().len(), 22);
        for c in s.as_str().chars() {
            assert!(c.is_ascii_alphanumeric() || c == '-' || c == '_');
        }
    }

    fn fresh_session() -> DeviceFlowSession {
        DeviceFlowSession {
            device_code: "DEV".into(),
            user_code: "ABCD-1234".into(),
            verification_uri: "https://example.com/device".into(),
            interval: Duration::from_secs(5),
            expires_at: SystemTime::now() + Duration::from_secs(300),
        }
    }

    #[test]
    fn device_flow_pending_keeps_interval() {
        let mut s = fresh_session();
        let d = s.apply_poll(&DevicePoll::Pending);
        assert_eq!(d, DeviceFlowDecision::Wait(Duration::from_secs(5)));
    }

    #[test]
    fn device_flow_slow_down_increases_interval() {
        let mut s = fresh_session();
        let _ = s.apply_poll(&DevicePoll::SlowDown);
        assert_eq!(s.interval, Duration::from_secs(10));
        let _ = s.apply_poll(&DevicePoll::SlowDown);
        assert_eq!(s.interval, Duration::from_secs(15));
    }

    #[test]
    fn device_flow_terminal_outcomes_abort() {
        for (poll, want) in [
            (DevicePoll::Denied, "denied"),
            (DevicePoll::Expired, "expired"),
            (DevicePoll::Error("oops".into()), "oops"),
        ] {
            let mut s = fresh_session();
            let d = s.apply_poll(&poll);
            match d {
                DeviceFlowDecision::Abort(msg) => assert_eq!(msg, want),
                other => panic!("{poll:?} should abort, got {other:?}"),
            }
        }
    }

    #[test]
    fn device_flow_authorized_returns_done() {
        let mut s = fresh_session();
        let d = s.apply_poll(&DevicePoll::Authorized);
        assert_eq!(d, DeviceFlowDecision::Done);
    }

    #[test]
    fn device_flow_expiry_check() {
        let mut s = fresh_session();
        let past = SystemTime::now() - Duration::from_secs(10);
        s.expires_at = past;
        assert!(s.is_expired_now());
        s.expires_at = SystemTime::now() + Duration::from_secs(60);
        assert!(!s.is_expired_now());
    }

    fn fake_session(id: &str, scopes: Vec<&str>) -> AuthSession {
        AuthSession {
            id: id.into(),
            ext_id: "alice.x".into(),
            provider_id: "github".into(),
            scopes: scopes.into_iter().map(String::from).collect(),
            access_token: "AT".into(),
            refresh_token: Some("RT".into()),
            expires_at: Some(now_seconds() + 3600),
            account: serde_json::json!({"login": "alice"}),
        }
    }

    #[test]
    fn session_store_round_trip() {
        let s = SessionStore::new();
        s.insert(fake_session("s1", vec!["repo"])).unwrap();
        let got = s.get("alice.x", "github", "s1").unwrap().unwrap();
        assert_eq!(got.access_token, "AT");
    }

    #[test]
    fn session_store_list_filters_by_ext_and_provider() {
        let s = SessionStore::new();
        s.insert(fake_session("s1", vec!["a"])).unwrap();
        s.insert(fake_session("s2", vec!["b"])).unwrap();
        // A different (ext, provider) pair must not leak in.
        let mut other = fake_session("s3", vec!["c"]);
        other.provider_id = "gitlab".into();
        s.insert(other).unwrap();
        let listed = s.list_for("alice.x", "github").unwrap();
        let ids: Vec<&str> = listed.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["s1", "s2"]);
    }

    #[test]
    fn session_store_remove_returns_bool() {
        let s = SessionStore::new();
        s.insert(fake_session("s1", vec!["repo"])).unwrap();
        assert!(s.remove("alice.x", "github", "s1").unwrap());
        assert!(!s.remove("alice.x", "github", "s1").unwrap());
    }

    #[test]
    fn auth_session_expiry_uses_supplied_now() {
        let mut sess = fake_session("s1", vec!["repo"]);
        sess.expires_at = Some(100);
        assert!(sess.is_expired_at(101));
        assert!(!sess.is_expired_at(99));
        sess.expires_at = None;
        assert!(!sess.is_expired_at(u64::MAX), "no expiry is never expired");
    }

    #[test]
    fn b64url_round_trip_known_vectors() {
        // Bare bytes → expected base64url (no padding)
        assert_eq!(b64url_encode(b""), "");
        assert_eq!(b64url_encode(b"f"), "Zg");
        assert_eq!(b64url_encode(b"fo"), "Zm8");
        assert_eq!(b64url_encode(b"foo"), "Zm9v");
        assert_eq!(b64url_encode(b"foob"), "Zm9vYg");
        assert_eq!(b64url_encode(b"fooba"), "Zm9vYmE");
        assert_eq!(b64url_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn constant_time_eq_examples() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn new_session_id_is_unique_each_call() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32, "uuid v4 simple form is 32 hex chars");
    }
}
