// Memory service — cross-session LLM memory & context, scoped by profile.
//
// Provides long-term memory entries with tag-based categorization, full-text search,
// and token-budget-aware prompt injection. Each profile maintains its own memory store.

use serde::{Deserialize, Serialize};

// ─── Memory Configuration ────────────────────────────────────────────────────

/// Memory configuration per profile — controls memory behavior and limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub max_entries: usize,
    pub max_tokens: usize,
    pub llm_can_write: bool,
    pub auto_extract: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 200,
            max_tokens: 2048,
            llm_can_write: true,
            auto_extract: false,
        }
    }
}

// ─── Memory Tag ──────────────────────────────────────────────────────────────

/// Memory tag categories — used for prioritized retrieval and filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryTag {
    General,
    Project,
    Preference,
    Fact,
    Instruction,
    Context,
}

impl MemoryTag {
    /// Sort priority: instruction > preference > fact > context > project > general.
    pub fn sort_priority(&self) -> u8 {
        match self {
            MemoryTag::Instruction => 5,
            MemoryTag::Preference => 4,
            MemoryTag::Fact => 3,
            MemoryTag::Context => 2,
            MemoryTag::Project => 1,
            MemoryTag::General => 0,
        }
    }
}

// ─── Memory Entry ────────────────────────────────────────────────────────────

/// A single memory entry — a fact, preference, or context note stored per profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub content: String,
    pub tag: MemoryTag,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub token_count: usize,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub last_used_at: u64,
    #[serde(default)]
    pub access_count: u32,
}

// ─── Memory Store ────────────────────────────────────────────────────────────

/// In-memory store for profile memories — CRUD, search, prompt rendering, LRU eviction.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryStore {
    #[serde(default)]
    pub profile_id: String,
    #[serde(default)]
    pub entries: Vec<MemoryEntry>,
}

impl MemoryStore {
    pub fn new(profile_id: &str) -> Self {
        Self {
            profile_id: profile_id.to_string(),
            entries: Vec::new(),
        }
    }

    /// Insert a new memory entry.
    pub fn insert(&mut self, entry: MemoryEntry) {
        self.entries.push(entry);
    }

    /// Update content of an existing entry by ID. Returns true if found.
    pub fn update(&mut self, id: &str, content: &str) -> bool {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.content = content.to_string();
            true
        } else {
            false
        }
    }

    /// Delete an entry by ID. Returns true if found.
    pub fn delete(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() < before
    }

    /// Touch an entry — update last_used_at and increment access_count.
    pub fn touch(&mut self, id: &str) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.last_used_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            e.access_count += 1;
        }
    }

    /// Search entries by substring match, optionally filtered by tag. Returns up to 10.
    pub fn search(&self, query: &str, tag: Option<&MemoryTag>) -> Vec<&MemoryEntry> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                let matches_tag = tag.is_none_or(|t| &e.tag == t);
                let matches_query =
                    query.is_empty() || e.content.to_lowercase().contains(&query_lower);
                matches_tag && matches_query
            })
            .take(10)
            .collect()
    }

    /// Render memories for system prompt injection.
    /// Sorted: pinned first, then by tag priority, then by access_count desc.
    /// Accumulates within the token budget.
    pub fn render_for_prompt(&self, budget: usize) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let mut sorted: Vec<&MemoryEntry> = self.entries.iter().collect();
        sorted.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.tag.sort_priority().cmp(&a.tag.sort_priority()))
                .then(b.access_count.cmp(&a.access_count))
        });

        let mut result = String::from("<memory>\n");
        let mut tokens_used = 0;

        for entry in sorted {
            let line = format!("[{:?}] {}\n", entry.tag, entry.content);
            let line_tokens = entry.token_count;
            if tokens_used + line_tokens > budget {
                break;
            }
            result.push_str(&line);
            tokens_used += line_tokens;
        }

        result.push_str("</memory>");
        result
    }

    /// Evict least-recently-used non-pinned entries beyond max_entries.
    pub fn evict_lru(&mut self, max_entries: usize) {
        if self.entries.len() <= max_entries {
            return;
        }
        self.entries.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.last_used_at.cmp(&a.last_used_at))
        });
        self.entries.truncate(max_entries);
    }
}
