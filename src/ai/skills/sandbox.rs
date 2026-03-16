//! Sandbox skills — governance and observability capabilities for the LLM agent:
//! budget status and audit log queries.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};

use crate::ai::budget::BudgetTracker;
use crate::ai::db::DbStore;
use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};

/// Register sandbox governance skills (budget status, audit log).
pub fn register_sandbox_skills(
    registry: &mut SkillRegistry,
    db: DbStore,
    budget: Arc<std::sync::Mutex<BudgetTracker>>,
) {
    register_get_budget_status(registry, budget);
    register_query_audit_log(registry, db);
}

// ─── Budget Status ───────────────────────────────────────────────────────────

fn register_get_budget_status(
    registry: &mut SkillRegistry,
    budget: Arc<std::sync::Mutex<BudgetTracker>>,
) {
    let skill = Skill {
        name: "cronymax.sandbox.budget_status".into(),
        description: "Get current token and turn budget usage for this session and today. \
            Shows remaining budget before limits are hit."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "integer",
                    "description": "Session ID to check (default: 0 = current)"
                }
            }
        }),
        category: "sandbox".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let budget = budget.clone();
        Box::pin(async move {
            let session_id = args["session_id"].as_u64().unwrap_or(0) as u32;
            let tracker = budget
                .lock()
                .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
            let status = tracker.status(session_id);
            Ok(serde_json::to_value(&status)?)
        })
    });

    registry.register(skill, handler);
}

// ─── Audit Log Query ─────────────────────────────────────────────────────────

fn register_query_audit_log(registry: &mut SkillRegistry, db: DbStore) {
    let skill = Skill {
        name: "cronymax.sandbox.query_audit_log".into(),
        description: "Query sandbox audit logs to review past actions, outcomes, and \
            policy decisions. Useful for understanding what happened in previous sessions."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "integer",
                    "description": "Filter by session ID (optional)"
                },
                "action": {
                    "type": "string",
                    "description": "Filter by action type (optional)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max entries to return (default: 20)"
                }
            }
        }),
        category: "sandbox".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let db = db.clone();
        Box::pin(async move {
            let session_id = args["session_id"].as_u64().map(|v| v as u32);
            let action = args["action"].as_str();
            let limit = args["limit"].as_u64().unwrap_or(20) as usize;

            let entries = db.audit_query(session_id, action, limit)?;

            let items: Vec<Value> = entries
                .iter()
                .map(|e| {
                    json!({
                        "id": e.id,
                        "timestamp": e.timestamp,
                        "session_id": e.session_id,
                        "action": e.action,
                        "detail": e.detail,
                        "outcome": e.outcome,
                        "policy_name": e.policy_name,
                    })
                })
                .collect();

            Ok(json!({
                "entries": items,
                "count": items.len(),
            }))
        })
    });

    registry.register(skill, handler);
}
