//! Pure utility functions with no access to RuntimeHandler state.
//! All items are `pub(super)` — only the handler sub-modules need them.

use std::sync::Arc;

use uuid::Uuid;

use crate::llm::ThinkingConfig;
use crate::protocol::control::{ControlError, ControlResponse};
use crate::runtime::authority::{AuthorityError, RuntimeAuthority};
use crate::runtime::middleware::{
    LlmDurationStore, MiddlewareChain, TimingMiddleware, TokenAccumulatorMiddleware,
    ToolDurationStore, TraceEmitterMiddleware,
};
use crate::runtime::state::{ReviewId, RunId, SpaceId};

pub(super) fn build_workspace_injection_block(
    workspace_path: &std::path::Path,
    tool_names: &[&str],
) -> String {
    let tools_line = if tool_names.is_empty() {
        "(none)".to_owned()
    } else {
        tool_names.join(", ")
    };
    format!(
        "\n---\nWorkspace: `{}`\nTools available: {}\nUse these tools to verify facts about the codebase. Never guess at structure.",
        workspace_path.display(),
        tools_line,
    )
}

/// Apply a per-run Anthropic effort override onto a model-derived
/// `ThinkingConfig`. Only the `Adaptive` variant carries an effort field;
/// other variants pass through unchanged. `None` for `override_effort`
/// leaves the existing value (typically the server default) intact.
pub(super) fn apply_anthropic_effort_override(
    cfg: Option<ThinkingConfig>,
    override_effort: Option<&str>,
) -> Option<ThinkingConfig> {
    let Some(eff) = override_effort.map(str::trim).filter(|s| !s.is_empty()) else {
        return cfg;
    };
    match cfg {
        Some(ThinkingConfig::Adaptive { summarized, .. }) => Some(ThinkingConfig::Adaptive {
            summarized,
            effort: Some(eff.to_owned()),
        }),
        // Other variants don't take an effort; leave unchanged.
        other => other,
    }
}

/// Adapter that turns a [`RuntimeAuthority`] into a dispatch.
pub(super) fn build_middleware_chain(authority: RuntimeAuthority) -> Arc<MiddlewareChain> {
    let llm_durations: LlmDurationStore =
        Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
    let tool_durations: ToolDurationStore =
        Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));
    let timing = Arc::new(TimingMiddleware::new(
        llm_durations.clone(),
        tool_durations.clone(),
    ));
    let token_accum = Arc::new(TokenAccumulatorMiddleware::new());
    let trace = Arc::new(TraceEmitterMiddleware::new(
        Arc::new(authority),
        llm_durations,
        tool_durations,
    ));
    Arc::new(MiddlewareChain(vec![timing, token_accum, trace]))
}

pub(super) fn base64_encode(data: &[u8]) -> String {
    // Simple base64 without dependencies — use the alphabet directly.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = (data[i] as u32) << 16 | (data[i + 1] as u32) << 8 | data[i + 2] as u32;
        out.push(ALPHABET[(n >> 18) as usize]);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize]);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize]);
        out.push(ALPHABET[(n & 0x3f) as usize]);
        i += 3;
    }
    match data.len() - i {
        1 => {
            let n = (data[i] as u32) << 16;
            out.push(ALPHABET[(n >> 18) as usize]);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize]);
            out.extend_from_slice(b"==");
        }
        2 => {
            let n = (data[i] as u32) << 16 | (data[i + 1] as u32) << 8;
            out.push(ALPHABET[(n >> 18) as usize]);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize]);
            out.push(ALPHABET[((n >> 6) & 0x3f) as usize]);
            out.push(b'=');
        }
        _ => {}
    }
    String::from_utf8(out).unwrap_or_default()
}

pub(super) fn parse_run(s: &str) -> Result<RunId, ControlResponse> {
    Uuid::parse_str(s)
        .map(RunId)
        .map_err(|_| ControlResponse::Err {
            error: ControlError::InvalidRequest {
                message: format!("invalid run id: {s}"),
            },
        })
}

pub(super) fn parse_space(s: &str) -> Result<SpaceId, ControlResponse> {
    Uuid::parse_str(s)
        .map(SpaceId)
        .map_err(|_| ControlResponse::Err {
            error: ControlError::InvalidRequest {
                message: format!("invalid space id: {s}"),
            },
        })
}

pub(super) fn parse_review(s: &str) -> Result<ReviewId, ControlResponse> {
    Uuid::parse_str(s)
        .map(ReviewId)
        .map_err(|_| ControlResponse::Err {
            error: ControlError::InvalidRequest {
                message: format!("invalid review id: {s}"),
            },
        })
}

pub(super) fn authority_err_to_control(
    e: AuthorityError,
    space_id: Option<&str>,
    run_id: Option<&str>,
) -> ControlError {
    match e {
        AuthorityError::UnknownSpace(id) => ControlError::UnknownSpace {
            space_id: space_id
                .map(str::to_owned)
                .unwrap_or_else(|| id.to_string()),
        },
        AuthorityError::UnknownRun(id) => ControlError::UnknownRun {
            run_id: run_id.map(str::to_owned).unwrap_or_else(|| id.to_string()),
        },
        AuthorityError::UnknownReview(_) => ControlError::InvalidRequest {
            message: "unknown review".into(),
        },
        AuthorityError::InvalidTransition { state, action, .. } => ControlError::InvalidState {
            message: format!("cannot {action} from {state:?}"),
        },
        AuthorityError::ReviewAlreadyResolved => ControlError::InvalidState {
            message: "review already resolved".into(),
        },
        AuthorityError::UnknownSession(id) => ControlError::InvalidRequest {
            message: format!("unknown session: {id}"),
        },
        AuthorityError::Persistence(p) => ControlError::Internal {
            message: format!("persistence: {p}"),
        },
    }
}
