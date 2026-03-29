// Session persistence — save/restore tab layout, chat history, and command history.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ai::context::ChatMessage;

// ─── Data Types ─────────────────────────────────────────────────────────────

/// Top-level session snapshot — serialized to `layout.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub version: u32,
    pub root: LayoutNode,
    pub active_tab_index: usize,
    pub timestamp: u64,
}

/// Recursive tree node representing the tab/pane layout topology.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LayoutNode {
    Tabs {
        children: Vec<TabDescriptor>,
    },
    Linear {
        direction: LinearDir,
        children: Vec<LayoutNode>,
        fractions: Vec<f32>,
    },
}

/// Direction for a split-pane layout.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinearDir {
    Horizontal,
    Vertical,
}

/// Serializable description of a single tab/pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabDescriptor {
    pub tab_type: TabType,
    pub persistent_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_selection: Option<ModelSelectionSnapshot>,
}

/// Discriminant for tab types.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TabType {
    Chat,
    Terminal,
    Browser,
    Channel,
}

/// Provider + model snapshot for a chat tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSelectionSnapshot {
    pub provider: String,
    pub model: String,
}

/// Persistent chat history for a single chat tab. Stored as `sessions/{uuid}.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSessionRecord {
    pub session_id: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub token_count: u32,
    pub created_at: u64,
    pub updated_at: u64,

    // ── Thread (branching) fields ──
    /// If this is a thread, the persistent UUID of the parent session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// If this is a thread, the cell_id of the block it branched from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_cell_id: Option<u32>,
    /// Map from block cell_id → child thread persistent UUID.
    #[serde(
        default,
        skip_serializing_if = "std::collections::HashMap::is_empty",
        alias = "topic_channels"
    )]
    pub threads: std::collections::HashMap<u32, String>,
}

/// A previously entered command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHistoryEntry {
    pub command: String,
    pub timestamp: u64,
}

/// Maximum command history entries (FIFO eviction).
const MAX_COMMAND_HISTORY: usize = 500;
/// Layout file name.
const LAYOUT_FILE: &str = "layout.json";
/// Command history file name.
const COMMAND_HISTORY_FILE: &str = "command_history.json";
/// Sessions subfolder.
const SESSIONS_DIR: &str = "sessions";

// ─── Save / Load Functions ──────────────────────────────────────────────────

/// Save a single chat session record with atomic write.
pub fn save_session_file(
    uuid: &str,
    record: &ChatSessionRecord,
    profile_dir: &Path,
) -> anyhow::Result<()> {
    let dir = profile_dir.join(SESSIONS_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{uuid}.json"));
    let tmp = dir.join(format!("{uuid}.json.tmp"));
    let json = serde_json::to_string_pretty(record)?;
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Load a single chat session record from disk.
pub fn load_session_file(uuid: &str, profile_dir: &Path) -> anyhow::Result<ChatSessionRecord> {
    let path = profile_dir.join(SESSIONS_DIR).join(format!("{uuid}.json"));
    let data = std::fs::read_to_string(&path)?;
    let record: ChatSessionRecord = serde_json::from_str(&data)?;
    Ok(record)
}

/// Save the layout snapshot to `layout.json`.
pub fn save_layout(snapshot: &SessionSnapshot, profile_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(profile_dir)?;
    let path = profile_dir.join(LAYOUT_FILE);
    let json = serde_json::to_string_pretty(snapshot)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Load the layout snapshot from `layout.json`.
pub fn load_layout(profile_dir: &Path) -> anyhow::Result<SessionSnapshot> {
    let path = profile_dir.join(LAYOUT_FILE);
    let data = std::fs::read_to_string(&path)?;
    let snapshot: SessionSnapshot = serde_json::from_str(&data)?;
    if snapshot.version != 1 {
        anyhow::bail!("Unsupported layout version: {}", snapshot.version);
    }
    Ok(snapshot)
}

/// Save command history (bounded to MAX_COMMAND_HISTORY).
pub fn save_command_history(
    entries: &[CommandHistoryEntry],
    profile_dir: &Path,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(profile_dir)?;
    // FIFO: keep only last N entries.
    let start = entries.len().saturating_sub(MAX_COMMAND_HISTORY);
    let bounded = &entries[start..];
    let json = serde_json::to_string_pretty(bounded)?;
    std::fs::write(profile_dir.join(COMMAND_HISTORY_FILE), json)?;
    Ok(())
}

/// Load command history from disk.
pub fn load_command_history(profile_dir: &Path) -> anyhow::Result<Vec<CommandHistoryEntry>> {
    let path = profile_dir.join(COMMAND_HISTORY_FILE);
    let data = std::fs::read_to_string(&path)?;
    let entries: Vec<CommandHistoryEntry> = serde_json::from_str(&data)?;
    Ok(entries)
}

// ─── Layout Tree Serialization ──────────────────────────────────────────────

use crate::ui::tiles::Pane;

/// Walk `egui_tiles::Tree<Pane>` and convert to `SessionSnapshot`.
pub fn serialize_layout(
    tree: &egui_tiles::Tree<Pane>,
    session_chats: &std::collections::HashMap<
        crate::renderer::terminal::SessionId,
        crate::ui::chat::SessionChat,
    >,
) -> SessionSnapshot {
    let root = match tree.root() {
        Some(root_id) => serialize_tile(tree, root_id, session_chats),
        None => LayoutNode::Tabs { children: vec![] },
    };

    // Determine active tab index.
    let active_tab_index = 0; // Default; the tree doesn't expose a simple active index.

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    SessionSnapshot {
        version: 1,
        root,
        active_tab_index,
        timestamp,
    }
}

fn serialize_tile(
    tree: &egui_tiles::Tree<Pane>,
    tile_id: egui_tiles::TileId,
    session_chats: &std::collections::HashMap<
        crate::renderer::terminal::SessionId,
        crate::ui::chat::SessionChat,
    >,
) -> LayoutNode {
    match tree.tiles.get(tile_id) {
        Some(egui_tiles::Tile::Pane(pane)) => LayoutNode::Tabs {
            children: vec![pane_to_descriptor(pane, session_chats)],
        },
        Some(egui_tiles::Tile::Container(container)) => match container {
            egui_tiles::Container::Tabs(tabs) => {
                let children: Vec<TabDescriptor> = tabs
                    .children
                    .iter()
                    .filter_map(|&child_id| {
                        if let Some(egui_tiles::Tile::Pane(pane)) = tree.tiles.get(child_id) {
                            Some(pane_to_descriptor(pane, session_chats))
                        } else {
                            None
                        }
                    })
                    .collect();
                LayoutNode::Tabs { children }
            }
            egui_tiles::Container::Linear(linear) => {
                let direction = match linear.dir {
                    egui_tiles::LinearDir::Horizontal => LinearDir::Horizontal,
                    egui_tiles::LinearDir::Vertical => LinearDir::Vertical,
                };
                let children: Vec<LayoutNode> = linear
                    .children
                    .iter()
                    .map(|&child_id| serialize_tile(tree, child_id, session_chats))
                    .collect();
                let fractions = linear.shares.iter().map(|(_, &s)| s).collect();
                LayoutNode::Linear {
                    direction,
                    children,
                    fractions,
                }
            }
            _ => LayoutNode::Tabs { children: vec![] },
        },
        None => LayoutNode::Tabs { children: vec![] },
    }
}

fn pane_to_descriptor(
    pane: &Pane,
    session_chats: &std::collections::HashMap<
        crate::renderer::terminal::SessionId,
        crate::ui::chat::SessionChat,
    >,
) -> TabDescriptor {
    match pane {
        Pane::Chat { session_id, title } => {
            let model_selection = session_chats.get(session_id).and_then(|chat| {
                chat.model_override
                    .as_ref()
                    .map(|sel| ModelSelectionSnapshot {
                        provider: format!("{:?}", sel.provider),
                        model: sel.model.clone(),
                    })
            });
            TabDescriptor {
                tab_type: TabType::Chat,
                persistent_id: session_chats
                    .get(session_id)
                    .and_then(|c| c.persistent_id.clone())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                title: title.clone(),
                model_selection,
            }
        }
        Pane::Terminal {
            session_id: _,
            title,
        } => TabDescriptor {
            tab_type: TabType::Terminal,
            persistent_id: uuid::Uuid::new_v4().to_string(),
            title: title.clone(),
            model_selection: None,
        },
        Pane::BrowserView { title, url, .. } => TabDescriptor {
            tab_type: TabType::Browser,
            persistent_id: uuid::Uuid::new_v4().to_string(),
            title: title.clone(),
            model_selection: Some(ModelSelectionSnapshot {
                provider: "browser".into(),
                model: url.clone(),
            }),
        },
        Pane::Channel {
            channel_id,
            channel_name,
        } => TabDescriptor {
            tab_type: TabType::Channel,
            persistent_id: channel_id.clone(),
            title: channel_name.clone(),
            model_selection: None,
        },
    }
}

/// Convert chat session data → `ChatSessionRecord` for persistence.
///
/// `all_chats` is used to resolve runtime `SessionId`s in `threads`
/// and `parent_session_id` to persistent UUIDs.
pub fn chat_to_record(
    persistent_id: &str,
    chat: &crate::ui::chat::SessionChat,
    all_chats: &std::collections::HashMap<
        crate::renderer::terminal::SessionId,
        crate::ui::chat::SessionChat,
    >,
) -> ChatSessionRecord {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // Filter out Info messages — they are not persisted.
    let messages: Vec<ChatMessage> = chat
        .messages
        .iter()
        .filter(|m| m.role != crate::ai::context::MessageRole::Info)
        .cloned()
        .collect();

    let (provider, model) = chat
        .model_override
        .as_ref()
        .map(|sel| (format!("{:?}", sel.provider), sel.model.clone()))
        .unwrap_or_else(|| ("default".into(), "default".into()));

    // Resolve parent_session_id: runtime SessionId → persistent UUID.
    let parent_session_id = chat.parent_session_id.and_then(|parent_sid| {
        all_chats
            .get(&parent_sid)
            .and_then(|parent_chat| parent_chat.persistent_id.clone())
    });

    // Build threads map: cell_id → child thread persistent UUID.
    let mut threads_map = std::collections::HashMap::new();
    for (&cell_id, &child_sid) in &chat.threads {
        if let Some(child_chat) = all_chats.get(&child_sid)
            && let Some(ref child_pid) = child_chat.persistent_id
        {
            threads_map.insert(cell_id, child_pid.clone());
        }
    }

    ChatSessionRecord {
        session_id: persistent_id.to_string(),
        provider,
        model,
        system_prompt: String::new(),
        messages,
        token_count: chat.tokens_used,
        created_at: now,
        updated_at: now,
        parent_session_id,
        branch_cell_id: chat.branch_cell_id,
        threads: threads_map,
    }
}

// ─── Block Reconstruction ───────────────────────────────────────────────────

/// Rebuild `PromptState.blocks` from a saved `ChatSessionRecord`.
///
/// Pairs consecutive User + Assistant messages into `Block::Stream` entries
/// so that restored chat history is visible in the UI. Returns the rebuilt
/// blocks and the next free `cell_id` counter.
pub fn rebuild_blocks_from_record(
    record: &ChatSessionRecord,
) -> (Vec<crate::ui::blocks::Block>, u32) {
    use crate::ai::context::MessageRole;
    use crate::ui::blocks::Block;

    let mut blocks = Vec::new();
    let mut next_cell_id: u32 = 0;

    let mut i = 0;
    let msgs = &record.messages;
    while i < msgs.len() {
        let msg = &msgs[i];
        match msg.role {
            MessageRole::User => {
                let cell_id = msg.cell_id.unwrap_or(next_cell_id);
                if cell_id >= next_cell_id {
                    next_cell_id = cell_id + 1;
                }
                // Collect the response from the following Assistant message(s).
                let mut response = String::new();
                if i + 1 < msgs.len() && msgs[i + 1].role == MessageRole::Assistant {
                    response = msgs[i + 1].content.clone();
                    i += 1;
                }
                blocks.push(Block::Stream {
                    id: cell_id,
                    prompt: msg.content.clone(),
                    response,
                    is_streaming: false,
                    tool_status: None,
                    tool_calls_log: Vec::new(),
                });
            }
            MessageRole::Assistant => {
                // Orphaned assistant message (no preceding user msg) — wrap in a block.
                let cell_id = msg.cell_id.unwrap_or(next_cell_id);
                if cell_id >= next_cell_id {
                    next_cell_id = cell_id + 1;
                }
                blocks.push(Block::Stream {
                    id: cell_id,
                    prompt: String::new(),
                    response: msg.content.clone(),
                    is_streaming: false,
                    tool_status: None,
                    tool_calls_log: Vec::new(),
                });
            }
            _ => {
                // Skip System / Tool / Info messages — not rendered as blocks.
            }
        }
        i += 1;
    }

    (blocks, next_cell_id)
}

// ─── Layout Tree Reconstruction ─────────────────────────────────────────────

/// Information about a restored tab. Returned by `reconstruct_tree` so the
/// caller can create the matching `TerminalSession`, `PromptBlock`, and
/// `SessionChat` for each restored pane.
#[derive(Debug)]
pub struct RestoredTab {
    pub session_id: u32,
    pub tab_type: TabType,
    pub title: String,
    pub persistent_id: String,
    pub model_selection: Option<ModelSelectionSnapshot>,
}

/// Reconstruct an `egui_tiles::Tree<Pane>` from a `SessionSnapshot`.
///
/// Returns the tree and a list of `RestoredTab` entries. The caller is
/// responsible for creating the corresponding runtime objects (PTY sessions,
/// prompt editors, chats) for each entry.
///
/// `next_id` is the starting session-ID counter; after the call the caller
/// should advance `state.next_id` past the maximum allocated ID.
pub fn reconstruct_tree(
    snapshot: &SessionSnapshot,
    next_id: &mut u32,
) -> (egui_tiles::Tree<Pane>, Vec<RestoredTab>) {
    let mut tabs = Vec::new();
    let mut tiles = egui_tiles::Tiles::default();
    let root = reconstruct_node(&snapshot.root, next_id, &mut tabs, &mut tiles);
    let mut tree = egui_tiles::Tree::new("tile_tree", root, tiles);

    // Activate the tab matching the saved active index (best-effort).
    if let Some(RestoredTab { session_id, .. }) = tabs.get(snapshot.active_tab_index) {
        let sid = *session_id;
        tree.make_active(|_, tile| {
            matches!(
                tile,
                egui_tiles::Tile::Pane(Pane::Chat { session_id: s, .. }
                    | Pane::Terminal { session_id: s, .. })
                if *s == sid
            )
        });
    }

    (tree, tabs)
}

fn reconstruct_node(
    node: &LayoutNode,
    next_id: &mut u32,
    tabs: &mut Vec<RestoredTab>,
    tiles: &mut egui_tiles::Tiles<Pane>,
) -> egui_tiles::TileId {
    match node {
        LayoutNode::Tabs { children } => {
            let child_ids: Vec<egui_tiles::TileId> = children
                .iter()
                .map(|desc| {
                    let sid = *next_id;
                    *next_id += 1;
                    let pane = match desc.tab_type {
                        TabType::Chat => Pane::Chat {
                            session_id: sid,
                            title: desc.title.clone(),
                        },
                        TabType::Terminal => Pane::Terminal {
                            session_id: sid,
                            title: desc.title.clone(),
                        },
                        TabType::Browser => Pane::BrowserView {
                            webview_id: sid,
                            title: desc.title.clone(),
                            url: desc
                                .model_selection
                                .as_ref()
                                .map(|m| m.model.clone())
                                .unwrap_or_default(),
                        },
                        TabType::Channel => Pane::Channel {
                            channel_id: desc.persistent_id.clone(),
                            channel_name: desc.title.clone(),
                        },
                    };
                    tabs.push(RestoredTab {
                        session_id: sid,
                        tab_type: desc.tab_type,
                        title: desc.title.clone(),
                        persistent_id: desc.persistent_id.clone(),
                        model_selection: desc.model_selection.clone(),
                    });
                    tiles.insert_pane(pane)
                })
                .collect();
            tiles.insert_tab_tile(child_ids)
        }
        LayoutNode::Linear {
            direction,
            children,
            fractions,
        } => {
            let child_ids: Vec<egui_tiles::TileId> = children
                .iter()
                .map(|child| reconstruct_node(child, next_id, tabs, tiles))
                .collect();
            let dir = match direction {
                LinearDir::Horizontal => egui_tiles::LinearDir::Horizontal,
                LinearDir::Vertical => egui_tiles::LinearDir::Vertical,
            };
            let mut linear = egui_tiles::Linear::new(dir, child_ids.clone());
            // Apply saved share fractions.
            for (i, &child_id) in child_ids.iter().enumerate() {
                if let Some(&frac) = fractions.get(i) {
                    linear.shares.set_share(child_id, frac);
                }
            }
            tiles.insert_container(egui_tiles::Container::Linear(linear))
        }
    }
}
