use super::*;

impl DbStore {
    // ── Memory CRUD ──────────────────────────────────────────────────────

    /// Insert a new memory entry. Returns the row ID.
    pub fn memory_insert(&self, row: &MemoryRow) -> anyhow::Result<i64> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        conn.execute(
            "INSERT INTO memory (profile_id, content, tag, pinned, token_count,
             created_at, last_used_at, access_count, embedding)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                row.profile_id,
                row.content,
                row.tag,
                row.pinned as i32,
                row.token_count,
                row.created_at,
                row.last_used_at,
                row.access_count,
                row.embedding,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Full-text search on memory. Returns up to `limit` results ranked by relevance.
    pub fn memory_search(
        &self,
        profile_id: &str,
        query: &str,
        tag: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;

        // Build FTS5 match query.
        let fts_query = if query.is_empty() {
            "*".to_string()
        } else {
            // Escape special FTS5 characters and add prefix matching.
            let escaped = query.replace('"', "\"\"");
            format!("\"{}\"*", escaped)
        };

        let sql = if let Some(_tag_filter) = tag {
            format!(
                "SELECT m.id, m.profile_id, m.content, m.tag, m.pinned,
                        m.token_count, m.created_at, m.last_used_at, m.access_count
                 FROM memory m
                 JOIN memory_fts f ON f.rowid = m.id
                 WHERE m.profile_id = ?1
                   AND m.tag = ?2
                   AND memory_fts MATCH ?3
                 ORDER BY rank
                 LIMIT {}",
                limit
            )
        } else {
            format!(
                "SELECT m.id, m.profile_id, m.content, m.tag, m.pinned,
                        m.token_count, m.created_at, m.last_used_at, m.access_count
                 FROM memory m
                 JOIN memory_fts f ON f.rowid = m.id
                 WHERE m.profile_id = ?1
                   AND memory_fts MATCH ?2
                 ORDER BY rank
                 LIMIT {}",
                limit
            )
        };

        let mut stmt = conn.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<MemoryRow> {
            Ok(MemoryRow {
                id: row.get(0)?,
                profile_id: row.get(1)?,
                content: row.get(2)?,
                tag: row.get(3)?,
                pinned: row.get::<_, i32>(4)? != 0,
                token_count: row.get(5)?,
                created_at: row.get(6)?,
                last_used_at: row.get(7)?,
                access_count: row.get(8)?,
                embedding: None,
            })
        };

        let mut results = Vec::new();
        if let Some(tag_filter) = tag {
            let rows = stmt.query_map(params![profile_id, tag_filter, fts_query], map_row)?;
            for row in rows {
                results.push(row?);
            }
        } else {
            let rows = stmt.query_map(params![profile_id, fts_query], map_row)?;
            for row in rows {
                results.push(row?);
            }
        }
        Ok(results)
    }

    /// List all memories for a profile, ordered by last_used_at desc.
    pub fn memory_list(&self, profile_id: &str, limit: usize) -> anyhow::Result<Vec<MemoryRow>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, profile_id, content, tag, pinned, token_count,
                    created_at, last_used_at, access_count
             FROM memory WHERE profile_id = ?1
             ORDER BY last_used_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![profile_id, limit as i64], |row| {
            Ok(MemoryRow {
                id: row.get(0)?,
                profile_id: row.get(1)?,
                content: row.get(2)?,
                tag: row.get(3)?,
                pinned: row.get::<_, i32>(4)? != 0,
                token_count: row.get(5)?,
                created_at: row.get(6)?,
                last_used_at: row.get(7)?,
                access_count: row.get(8)?,
                embedding: None,
            })
        })?;
        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    /// Touch a memory (update last_used_at and increment access_count).
    pub fn memory_touch(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let now = now_millis();
        conn.execute(
            "UPDATE memory SET last_used_at = ?1, access_count = access_count + 1
             WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    /// Delete a memory entry.
    pub fn memory_delete(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        conn.execute("DELETE FROM memory WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Delete unpinned memories exceeding max_entries for a profile (LRU eviction).
    pub fn memory_evict(&self, profile_id: &str, max_entries: usize) -> anyhow::Result<usize> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{}", e))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory WHERE profile_id = ?1",
            params![profile_id],
            |row| row.get(0),
        )?;
        if count as usize <= max_entries {
            return Ok(0);
        }
        let to_remove = count as usize - max_entries;
        conn.execute(
            "DELETE FROM memory WHERE id IN (
                SELECT id FROM memory
                WHERE profile_id = ?1 AND pinned = 0
                ORDER BY last_used_at ASC
                LIMIT ?2
             )",
            params![profile_id, to_remove as i64],
        )?;
        Ok(to_remove)
    }
}
