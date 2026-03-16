use super::*;

/// Parameters for updating an onboarding wizard step.
pub struct WizardStepUpdate<'a> {
    pub id: i64,
    pub step: &'a str,
    pub lark_app_id: Option<&'a str>,
    pub oauth_token: Option<&'a str>,
    pub tenant_id: Option<&'a str>,
    pub secret_store: &'a crate::secret::SecretStore,
}

impl DbStore {
    // ── Audit Log ────────────────────────────────────────────────────────

    /// Record a sandbox audit event.
    pub fn insert_audit_log(
        &self,
        session_id: u32,
        action: &str,
        detail: &str,
        outcome: &str,
        policy_name: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        conn.execute(
            "INSERT INTO audit_logs (timestamp, session_id, action, detail, outcome, policy_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                now_millis(),
                session_id,
                action,
                detail,
                outcome,
                policy_name
            ],
        )?;
        Ok(())
    }

    /// Query recent audit entries, optionally filtered by session or action.
    pub fn audit_query(
        &self,
        session_id: Option<u32>,
        action: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<AuditEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;

        let sql = match (session_id, action) {
            (Some(sid), Some(act)) => format!(
                "SELECT id, timestamp, session_id, action, detail, outcome, policy_name
                 FROM audit_logs WHERE session_id = {} AND action = '{}'
                 ORDER BY timestamp DESC LIMIT {}",
                sid, act, limit
            ),
            (Some(sid), None) => format!(
                "SELECT id, timestamp, session_id, action, detail, outcome, policy_name
                 FROM audit_logs WHERE session_id = {}
                 ORDER BY timestamp DESC LIMIT {}",
                sid, limit
            ),
            (None, Some(act)) => format!(
                "SELECT id, timestamp, session_id, action, detail, outcome, policy_name
                 FROM audit_logs WHERE action = '{}'
                 ORDER BY timestamp DESC LIMIT {}",
                act, limit
            ),
            (None, None) => format!(
                "SELECT id, timestamp, session_id, action, detail, outcome, policy_name
                 FROM audit_logs ORDER BY timestamp DESC LIMIT {}",
                limit
            ),
        };
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(AuditEntry {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                session_id: row.get(2)?,
                action: row.get(3)?,
                detail: row.get(4)?,
                outcome: row.get(5)?,
                policy_name: row.get(6)?,
            })
        })?;
        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    // ── Budget Usage ─────────────────────────────────────────────────────

    /// Record token and turn usage. Upserts the row for the given scope+key.
    pub fn store_budget_usage(
        &self,
        profile_id: &str,
        scope: &str,
        scope_key: &str,
        tokens: i64,
        turns: i64,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        conn.execute(
            "INSERT INTO budget_usages (profile_id, scope, scope_key, tokens_used, turns_used, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(profile_id, scope, scope_key) DO UPDATE SET
                tokens_used = tokens_used + ?4,
                turns_used  = turns_used  + ?5,
                updated_at  = ?6",
            params![profile_id, scope, scope_key, tokens, turns, now_millis()],
        )?;
        Ok(())
    }

    /// Get usage for a specific scope+key.
    pub fn retrieve_budget_usage(
        &self,
        profile_id: &str,
        scope: &str,
        scope_key: &str,
    ) -> anyhow::Result<Option<BudgetRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, profile_id, scope, scope_key, tokens_used, turns_used, updated_at
             FROM budget_usages
             WHERE profile_id = ?1 AND scope = ?2 AND scope_key = ?3",
        )?;
        let mut rows = stmt.query_map(params![profile_id, scope, scope_key], |row| {
            Ok(BudgetRow {
                id: row.get(0)?,
                profile_id: row.get(1)?,
                scope: row.get(2)?,
                scope_key: row.get(3)?,
                tokens_used: row.get(4)?,
                turns_used: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;
        match rows.next() {
            Some(Ok(row)) => Ok(Some(row)),
            _ => Ok(None),
        }
    }

    /// Reset budget usage for a scope+key (e.g. at day rollover).
    pub fn reset_budget_usage(
        &self,
        profile_id: &str,
        scope: &str,
        scope_key: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        conn.execute(
            "DELETE FROM budget_usages
             WHERE profile_id = ?1 AND scope = ?2 AND scope_key = ?3",
            params![profile_id, scope, scope_key],
        )?;
        Ok(())
    }

    // ── Onboarding Wizard CRUD ───────────────────────────────────────────

    /// Create a new onboarding wizard session. Returns the row ID.
    pub fn create_wizard(&self, is_admin: bool) -> anyhow::Result<i64> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let now = now_millis();
        conn.execute(
            "INSERT INTO onboarding_wizards (current_step, is_admin, created_at, updated_at)
             VALUES ('login', ?1, ?2, ?3)",
            params![is_admin as i32, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Update the current step and optional fields of an active wizard.
    /// If an OAuth token is provided and the system keychain is available,
    /// the token is stored in the keychain instead of the SQLite column.
    pub fn update_wizard_step(&self, upd: WizardStepUpdate<'_>) -> anyhow::Result<()> {
        // Store OAuth token in keychain when available, sentinel in DB.
        let db_token = if let Some(token) = upd.oauth_token {
            if upd.secret_store.has_keychain() {
                let key = crate::secret::oauth_token("lark");
                upd.secret_store.store(&key, token)?;
                Some("[keychain]")
            } else {
                Some(token)
            }
        } else {
            None
        };

        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let now = now_millis();
        conn.execute(
            "UPDATE onboarding_wizards
             SET current_step = ?1, lark_app_id = COALESCE(?2, lark_app_id),
                 oauth_token = COALESCE(?3, oauth_token),
                 tenant_id = COALESCE(?4, tenant_id),
                 updated_at = ?5
             WHERE id = ?6",
            params![
                upd.step,
                upd.lark_app_id,
                db_token,
                upd.tenant_id,
                now,
                upd.id
            ],
        )?;
        Ok(())
    }

    /// Get the most recent active onboarding wizard (not yet completed).
    /// If the OAuth token column contains the sentinel `[keychain]`, the real
    /// token is resolved from the system keychain.
    pub fn get_active_wizard(
        &self,
        secret_store: &crate::secret::SecretStore,
    ) -> anyhow::Result<Option<OnboardingWizardRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, current_step, lark_app_id, oauth_token, tenant_id, is_admin,
                    created_at, updated_at
             FROM onboarding_wizards
             WHERE current_step != 'completed'
             ORDER BY updated_at DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query_map([], |row| {
            Ok(OnboardingWizardRow {
                id: row.get(0)?,
                current_step: row.get(1)?,
                lark_app_id: row.get(2)?,
                oauth_token: row.get(3)?,
                tenant_id: row.get(4)?,
                is_admin: row.get::<_, i32>(5)? != 0,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?;
        match rows.next() {
            Some(Ok(mut row)) => {
                // Resolve token from keychain if sentinel is present.
                if row.oauth_token.as_deref() == Some("[keychain]") {
                    let key = crate::secret::oauth_token("lark");
                    row.oauth_token = secret_store
                        .resolve(&key, None, &crate::secret::SecretStorage::Keychain)
                        .ok()
                        .flatten();
                }
                Ok(Some(row))
            }
            _ => Ok(None),
        }
    }

    /// Delete completed wizard rows (cleanup after successful onboarding).
    pub fn clear_completed_wizards(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let deleted = conn.execute(
            "DELETE FROM onboarding_wizards WHERE current_step = 'completed'",
            [],
        )?;
        Ok(deleted)
    }

    /// Migrate any plaintext OAuth tokens from the onboarding_wizards table to the
    /// system keychain. Replaces the SQLite column value with a sentinel.
    pub fn migrate_oauth_token_to_keychain(
        &self,
        secret_store: &crate::secret::SecretStore,
    ) -> anyhow::Result<u32> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let store = secret_store;
        if !store.has_keychain() {
            log::debug!("System keychain not available — skipping OAuth token migration");
            return Ok(0);
        }

        let sentinel = "[keychain]";
        let mut stmt = conn.prepare(
            "SELECT id, oauth_token FROM onboarding_wizards WHERE oauth_token IS NOT NULL AND oauth_token != ?1",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(params![sentinel], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let mut migrated = 0u32;
        for (id, token) in &rows {
            if token.is_empty() {
                continue;
            }
            let key = crate::secret::oauth_token("lark");
            if let Err(e) = store.store(&key, token) {
                log::error!(
                    "Failed to migrate OAuth token (row {}) to keychain: {}",
                    id,
                    e
                );
                continue;
            }
            conn.execute(
                "UPDATE onboarding_wizards SET oauth_token = ?1 WHERE id = ?2",
                params![sentinel, id],
            )?;
            migrated += 1;
            log::info!("Migrated OAuth token (row {}) to system keychain", id);
        }
        Ok(migrated)
    }

    // ── Starred Blocks CRUD ──────────────────────────────────────────────

    /// Get all starred message IDs for a session.
    pub fn get_starred_blocks(&self, session_id: u32) -> anyhow::Result<Vec<u32>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let mut stmt =
            conn.prepare("SELECT message_id FROM starred_blocks WHERE session_id = ?1")?;
        let rows = stmt.query_map(params![session_id], |row| row.get::<_, u32>(0))?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        Ok(ids)
    }

    /// Set or unset starred state for a message.
    pub fn set_starred_block(
        &self,
        session_id: u32,
        message_id: u32,
        starred: bool,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        if starred {
            let now = now_millis();
            conn.execute(
                "INSERT OR REPLACE INTO starred_blocks (session_id, message_id, starred_at)
                 VALUES (?1, ?2, ?3)",
                params![session_id, message_id, now],
            )?;
        } else {
            conn.execute(
                "DELETE FROM starred_blocks WHERE session_id = ?1 AND message_id = ?2",
                params![session_id, message_id],
            )?;
        }
        Ok(())
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

pub(super) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
