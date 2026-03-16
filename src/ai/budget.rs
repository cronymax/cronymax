// Budget tracking — token & turn limits per day/session with automatic enforcement.
//
// Provides `BudgetTracker` which wraps the DbStore budget_usage table and
// adds configurable daily/session limits with pre-check enforcement.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::ai::db::DbStore;

// ─── Configuration ───────────────────────────────────────────────────────────

/// Per-profile budget limits configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetLimits {
    /// Maximum tokens per session (0 = unlimited).
    pub session_token_limit: i64,
    /// Maximum turns (user messages) per session (0 = unlimited).
    pub session_turn_limit: i64,
    /// Maximum tokens per day across all sessions (0 = unlimited).
    pub daily_token_limit: i64,
    /// Maximum turns per day across all sessions (0 = unlimited).
    pub daily_turn_limit: i64,
    /// Maximum context window utilization ratio (0.0–1.0). When exceeded,
    /// the system auto-compacts or refuses the request.
    pub context_window_max_ratio: f64,
}

impl Default for BudgetLimits {
    fn default() -> Self {
        Self {
            session_token_limit: 0, // unlimited
            session_turn_limit: 0,  // unlimited
            daily_token_limit: 0,   // unlimited
            daily_turn_limit: 0,    // unlimited
            context_window_max_ratio: 0.95,
        }
    }
}

/// Outcome of a budget pre-check.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetCheck {
    /// Allowed — proceed with the request.
    Allowed,
    /// Session token limit exceeded.
    SessionTokensExceeded { used: i64, limit: i64 },
    /// Session turn limit exceeded.
    SessionTurnsExceeded { used: i64, limit: i64 },
    /// Daily token limit exceeded.
    DailyTokensExceeded { used: i64, limit: i64 },
    /// Daily turn limit exceeded.
    DailyTurnsExceeded { used: i64, limit: i64 },
    /// Context window too full — needs compaction.
    ContextWindowFull { used_ratio: f64, max_ratio: f64 },
}

impl BudgetCheck {
    /// Whether the check passed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    /// Human-readable denial reason.
    pub fn denial_reason(&self) -> Option<String> {
        match self {
            Self::Allowed => None,
            Self::SessionTokensExceeded { used, limit } => Some(format!(
                "Session token limit reached ({}/{} tokens). Start a new session or increase the limit.",
                used, limit
            )),
            Self::SessionTurnsExceeded { used, limit } => Some(format!(
                "Session turn limit reached ({}/{} turns). Start a new session or increase the limit.",
                used, limit
            )),
            Self::DailyTokensExceeded { used, limit } => Some(format!(
                "Daily token limit reached ({}/{} tokens). Wait until tomorrow or increase the limit.",
                used, limit
            )),
            Self::DailyTurnsExceeded { used, limit } => Some(format!(
                "Daily turn limit reached ({}/{} turns). Wait until tomorrow or increase the limit.",
                used, limit
            )),
            Self::ContextWindowFull {
                used_ratio,
                max_ratio,
            } => Some(format!(
                "Context window {:.0}% full (max {:.0}%). Compact the conversation or start a new session.",
                used_ratio * 100.0,
                max_ratio * 100.0,
            )),
        }
    }
}

/// Budget usage snapshot (for UI display).
#[derive(Debug, Clone, Default, Serialize)]
pub struct BudgetStatus {
    pub session_tokens_used: i64,
    pub session_turns_used: i64,
    pub daily_tokens_used: i64,
    pub daily_turns_used: i64,
    pub session_token_limit: i64,
    pub session_turn_limit: i64,
    pub daily_token_limit: i64,
    pub daily_turn_limit: i64,
    pub context_window_used: usize,
    pub context_window_limit: usize,
}

// ─── Budget Tracker ──────────────────────────────────────────────────────────

/// Tracks token and turn budgets per session and per day.
pub struct BudgetTracker {
    db: DbStore,
    /// Current profile ID for budget scope.
    pub profile_id: String,
    /// Configurable limits.
    pub limits: BudgetLimits,
}

impl BudgetTracker {
    /// Create a new budget tracker for the given profile.
    pub fn new(db: DbStore, profile_id: String, limits: BudgetLimits) -> Self {
        Self {
            db,
            profile_id,
            limits,
        }
    }

    /// Pre-check whether a new turn is within budget.
    /// Call this *before* sending a request to the LLM.
    pub fn pre_check(
        &self,
        session_id: u32,
        context_tokens_used: usize,
        context_tokens_limit: usize,
    ) -> BudgetCheck {
        // 1. Context window ratio check.
        if context_tokens_limit > 0 && self.limits.context_window_max_ratio > 0.0 {
            let ratio = context_tokens_used as f64 / context_tokens_limit as f64;
            if ratio > self.limits.context_window_max_ratio {
                return BudgetCheck::ContextWindowFull {
                    used_ratio: ratio,
                    max_ratio: self.limits.context_window_max_ratio,
                };
            }
        }

        let session_key = format!("s{}", session_id);
        let daily_key = today_key();

        // 2. Session limits.
        if let Ok(Some(row)) =
            self.db
                .retrieve_budget_usage(&self.profile_id, "session", &session_key)
        {
            if self.limits.session_token_limit > 0
                && row.tokens_used >= self.limits.session_token_limit
            {
                return BudgetCheck::SessionTokensExceeded {
                    used: row.tokens_used,
                    limit: self.limits.session_token_limit,
                };
            }
            if self.limits.session_turn_limit > 0
                && row.turns_used >= self.limits.session_turn_limit
            {
                return BudgetCheck::SessionTurnsExceeded {
                    used: row.turns_used,
                    limit: self.limits.session_turn_limit,
                };
            }
        }

        // 3. Daily limits.
        if let Ok(Some(row)) = self
            .db
            .retrieve_budget_usage(&self.profile_id, "daily", &daily_key)
        {
            if self.limits.daily_token_limit > 0 && row.tokens_used >= self.limits.daily_token_limit
            {
                return BudgetCheck::DailyTokensExceeded {
                    used: row.tokens_used,
                    limit: self.limits.daily_token_limit,
                };
            }
            if self.limits.daily_turn_limit > 0 && row.turns_used >= self.limits.daily_turn_limit {
                return BudgetCheck::DailyTurnsExceeded {
                    used: row.turns_used,
                    limit: self.limits.daily_turn_limit,
                };
            }
        }

        BudgetCheck::Allowed
    }

    /// Record usage after a completed LLM turn.
    /// `tokens` is total tokens used (prompt + completion).
    pub fn record_usage(&self, session_id: u32, tokens: i64) -> anyhow::Result<()> {
        let session_key = format!("s{}", session_id);
        let daily_key = today_key();

        self.db
            .store_budget_usage(&self.profile_id, "session", &session_key, tokens, 1)?;
        self.db
            .store_budget_usage(&self.profile_id, "daily", &daily_key, tokens, 1)?;
        Ok(())
    }

    /// Get current budget status for UI display.
    pub fn status(&self, session_id: u32) -> BudgetStatus {
        let session_key = format!("s{}", session_id);
        let daily_key = today_key();

        let session = self
            .db
            .retrieve_budget_usage(&self.profile_id, "session", &session_key)
            .ok()
            .flatten();
        let daily = self
            .db
            .retrieve_budget_usage(&self.profile_id, "daily", &daily_key)
            .ok()
            .flatten();

        BudgetStatus {
            session_tokens_used: session.as_ref().map(|r| r.tokens_used).unwrap_or(0),
            session_turns_used: session.as_ref().map(|r| r.turns_used).unwrap_or(0),
            daily_tokens_used: daily.as_ref().map(|r| r.tokens_used).unwrap_or(0),
            daily_turns_used: daily.as_ref().map(|r| r.turns_used).unwrap_or(0),
            session_token_limit: self.limits.session_token_limit,
            session_turn_limit: self.limits.session_turn_limit,
            daily_token_limit: self.limits.daily_token_limit,
            daily_turn_limit: self.limits.daily_turn_limit,
            context_window_used: 0,
            context_window_limit: 0,
        }
    }

    /// Reset session-scoped budget (e.g. when starting a new session).
    pub fn reset_session(&self, session_id: u32) -> anyhow::Result<()> {
        let session_key = format!("s{}", session_id);
        self.db
            .reset_budget_usage(&self.profile_id, "session", &session_key)?;
        Ok(())
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Today's date as "YYYY-MM-DD" for daily budget scope key (stdlib only).
fn today_key() -> String {
    // Use epoch-based day calculation (UTC).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Days since epoch.
    let days = secs / 86400;
    // Civil date from day count (algorithm from Howard Hinnant).
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}
