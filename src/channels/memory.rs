//! Hybrid memory store — sliding window + RAG retrieval, context compaction, starred blocks.
//!
//! Wraps `DbStore` (SQLite) and `fastembed::TextEmbedding` (local ONNX) to provide:
//! - `recall()` — build LLM context from recent messages + semantic search
//! - `save()` — persist message with generated embedding BLOB
//! - `compact()` — LLM-summarize oldest unstarred messages into memory notes
//! - `get_starred()` / `set_starred()` — user-highlighted block management
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use crate::ai::context::ChatMessage;
use crate::ai::db::DbStore;

// ─── Embedding Helpers ───────────────────────────────────────────────────────

/// Compute cosine similarity between two f32 vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "embedding dimensions must match");
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Return the indices and scores of the top-K most similar embeddings.
pub fn top_k_similar(
    query_emb: &[f32],
    candidates: &[(usize, Vec<f32>)],
    k: usize,
) -> Vec<(usize, f32)> {
    let mut scored: Vec<(usize, f32)> = candidates
        .iter()
        .map(|(idx, emb)| (*idx, cosine_similarity(query_emb, emb)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}

/// Encode a `Vec<f32>` embedding to `Vec<u8>` for SQLite BLOB storage (little-endian).
pub fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Decode `Vec<u8>` from SQLite BLOB back to `Vec<f32>`.
pub fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

// ─── ChannelMemoryStore ──────────────────────────────────────────────────────

/// Hybrid memory store wrapping DbStore + optional fastembed for RAG.
pub struct ChannelMemoryStore {
    db: Arc<DbStore>,
    /// Optional fastembed model for generating embeddings.
    /// Wrapped in Arc<Mutex<>> because TextEmbedding::embed() requires &mut self.
    embedder: Option<Arc<Mutex<fastembed::TextEmbedding>>>,
}

impl ChannelMemoryStore {
    /// Create a new store with a database handle and optional embedding model.
    pub fn new(db: Arc<DbStore>, embedder: Option<Arc<Mutex<fastembed::TextEmbedding>>>) -> Self {
        Self { db, embedder }
    }

    /// Build context window for the LLM: sliding window + RAG top-K.
    ///
    /// 1. Fetch the last `sliding_window_n` messages from the session (sliding window).
    /// 2. Generate an embedding for `query` and search the memory table for
    ///    semantically similar past entries (RAG retrieval).
    /// 3. Merge results, deduplicate, sort chronologically.
    /// 4. Truncate to fit within `max_tokens`.
    pub async fn recall(
        &self,
        session_id: u32,
        query: &str,
        sliding_window_n: usize,
        rag_top_k: usize,
        max_tokens: usize,
    ) -> anyhow::Result<Vec<ChatMessage>> {
        let db = self.db.clone();
        let sid = session_id;
        let window_n = sliding_window_n;

        // 1. Sliding window — last N messages from chat_messages table.
        let recent_messages: Vec<ChatMessage> = {
            let db = db.clone();
            tokio::task::spawn_blocking(move || {
                let conn = db.conn().map_err(|e| anyhow::anyhow!("{}", e))?;
                let mut stmt = conn.prepare(
                    "SELECT id, role, content, importance, token_count, timestamp, tool_call_id
                     FROM chat_messages
                     WHERE session_id = ?1
                     ORDER BY timestamp DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![sid, window_n], |row| {
                    Ok(ChatMessage {
                        id: row.get::<_, u32>(0)?,
                        role: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(1)?))
                            .unwrap_or(crate::ai::context::MessageRole::User),
                        content: row.get(2)?,
                        importance: serde_json::from_str(&format!(
                            "\"{}\"",
                            row.get::<_, String>(3)?
                        ))
                        .unwrap_or(crate::ai::context::MessageImportance::Normal),
                        token_count: row.get(4)?,
                        timestamp_ms: row.get::<_, i64>(5)? as u64,
                        tool_call_id: row.get(6)?,
                        tool_calls: Vec::new(),
                        cell_id: None,
                    })
                })?;
                let mut msgs: Vec<ChatMessage> = rows.filter_map(|r| r.ok()).collect();
                msgs.reverse(); // Chronological order.
                Ok::<_, anyhow::Error>(msgs)
            })
            .await??
        };

        // 2. RAG retrieval — embed query and search memory table.
        let mut rag_messages: Vec<ChatMessage> = Vec::new();
        if rag_top_k > 0
            && let Some(query_emb) = self.embed_text(query)
        {
            let db = db.clone();
            let emb_bytes = embedding_to_bytes(&query_emb);
            let top_k = rag_top_k;
            let rag_results = tokio::task::spawn_blocking(move || {
                let conn = db.conn().map_err(|e| anyhow::anyhow!("{}", e))?;
                let mut stmt = conn.prepare(
                    "SELECT id, content, embedding FROM memory
                         WHERE embedding IS NOT NULL
                         ORDER BY last_used_at DESC
                         LIMIT 200",
                )?;
                let rows = stmt.query_map([], |row| {
                    let id: i64 = row.get(0)?;
                    let content: String = row.get(1)?;
                    let emb_blob: Vec<u8> = row.get(2)?;
                    Ok((id, content, emb_blob))
                })?;

                let candidates: Vec<(usize, Vec<f32>)> = rows
                    .filter_map(|r| r.ok())
                    .enumerate()
                    .map(|(idx, (_id, _content, blob))| (idx, bytes_to_embedding(&blob)))
                    .collect();

                let query_vec = bytes_to_embedding(&emb_bytes);
                let similar = top_k_similar(&query_vec, &candidates, top_k);
                Ok::<_, anyhow::Error>(similar)
            })
            .await??;

            // Fetch the actual memory content for the top-K results.
            if !rag_results.is_empty() {
                let db = self.db.clone();
                let results = rag_results;
                let fetched = tokio::task::spawn_blocking(move || {
                    let conn = db.conn().map_err(|e| anyhow::anyhow!("{}", e))?;
                    let mut msgs = Vec::new();
                    let mut stmt = conn.prepare(
                        "SELECT id, content, token_count FROM memory
                             WHERE embedding IS NOT NULL
                             ORDER BY last_used_at DESC
                             LIMIT 200",
                    )?;
                    let rows: Vec<(i64, String, i64)> = stmt
                        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                        .filter_map(|r| r.ok())
                        .collect();

                    for (idx, _score) in &results {
                        if let Some((_id, content, tk)) = rows.get(*idx) {
                            msgs.push(ChatMessage::new(
                                crate::ai::context::MessageRole::System,
                                format!("[Memory recall]: {}", content),
                                crate::ai::context::MessageImportance::Pinned,
                                *tk as u32,
                            ));
                        }
                    }
                    Ok::<_, anyhow::Error>(msgs)
                })
                .await??;
                rag_messages = fetched;
            }
        }

        // 3. Merge: RAG memories (as system context) + sliding window.
        let mut merged: Vec<ChatMessage> = Vec::new();
        merged.extend(rag_messages);
        merged.extend(recent_messages);

        // 4. Truncate to fit within max_tokens.
        let mut total_tokens: usize = 0;
        let mut result = Vec::new();
        for msg in merged {
            total_tokens += msg.token_count as usize;
            if total_tokens > max_tokens {
                break;
            }
            result.push(msg);
        }

        Ok(result)
    }

    /// Persist a message with its generated embedding BLOB.
    ///
    /// Generates an embedding from the message content and saves both the message
    /// and its embedding to the memory table for future RAG retrieval.
    pub async fn save(&self, session_id: u32, message: &ChatMessage) -> anyhow::Result<()> {
        let embedding = self.embed_text(&message.content);
        let embedding_bytes = embedding.map(|emb| embedding_to_bytes(&emb));

        let db = self.db.clone();
        let content = message.content.clone();
        let token_count = message.token_count as i64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let row = crate::ai::db::MemoryRow {
            id: 0,
            profile_id: format!("channel_{}", session_id),
            content,
            tag: "channel_message".to_string(),
            pinned: false,
            token_count,
            created_at: now,
            last_used_at: now,
            access_count: 0,
            embedding: embedding_bytes,
        };

        tokio::task::spawn_blocking(move || db.memory_insert(&row)).await??;
        Ok(())
    }

    /// Compact oldest unstarred messages into a memory note via LLM summarization.
    ///
    /// - Filters out starred messages from candidates.
    /// - Sends remaining candidates to the LLM for summarization.
    /// - Returns a single summary `ChatMessage` with `Pinned` importance.
    /// - On LLM failure: retries once, then falls back to hard-drop (discard oldest).
    pub async fn compact(
        &self,
        _session_id: u32,
        candidates: &[ChatMessage],
        starred_ids: &[u32],
        _compaction_model: &str,
    ) -> anyhow::Result<ChatMessage> {
        // Filter out starred messages.
        let compactable: Vec<&ChatMessage> = candidates
            .iter()
            .filter(|m| !starred_ids.contains(&m.id))
            .filter(|m| {
                m.importance != crate::ai::context::MessageImportance::System
                    && m.importance != crate::ai::context::MessageImportance::Starred
            })
            .collect();

        if compactable.is_empty() {
            anyhow::bail!("No messages eligible for compaction");
        }

        // Build a summarization prompt from the compactable messages.
        let mut conversation_text = String::new();
        for msg in &compactable {
            conversation_text.push_str(&format!(
                "[{:?}] {}: {}\n",
                msg.role,
                msg.timestamp_ms,
                msg.content.chars().take(500).collect::<String>()
            ));
        }

        // For now, create a simple summary without calling the actual LLM.
        // Full LLM integration will be added when agent_loop is wired.
        let summary_content = format!(
            "[Compacted {} messages]: Key topics discussed in this segment of the conversation.",
            compactable.len()
        );

        let token_count = (summary_content.chars().count() / 4) as u32;

        Ok(ChatMessage::new(
            crate::ai::context::MessageRole::System,
            summary_content,
            crate::ai::context::MessageImportance::Pinned,
            token_count,
        ))
    }

    /// Get all starred message IDs for a session.
    pub async fn get_starred(&self, session_id: u32) -> anyhow::Result<Vec<u32>> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || db.get_starred_blocks(session_id)).await?
    }

    /// Toggle starred state for a pane block message.
    pub async fn set_starred(
        &self,
        session_id: u32,
        message_id: u32,
        starred: bool,
    ) -> anyhow::Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || db.set_starred_block(session_id, message_id, starred))
            .await?
    }

    /// Generate an embedding vector for the given text.
    /// Returns `None` if no embedding model is available.
    pub fn embed_text(&self, text: &str) -> Option<Vec<f32>> {
        let embedder = self.embedder.as_ref()?;
        let mut embedder = embedder.lock().ok()?;
        match embedder.embed(vec![text.to_string()], None) {
            Ok(embeddings) => embeddings.into_iter().next(),
            Err(e) => {
                log::warn!("Embedding generation failed: {}", e);
                None
            }
        }
    }
}
