//! Tree creation and pane management helpers.

use super::*;

// ─── Tree Helper Functions ───────────────────────────────────────────────────

/// Create an initial tree with a single chat pane in a Tabs container.
pub fn create_initial_tree(session_id: SessionId, title: &str) -> egui_tiles::Tree<Pane> {
    let pane = Pane::Chat {
        session_id,
        title: title.to_string(),
    };
    egui_tiles::Tree::new_tabs("tile_tree", vec![pane])
}

/// Create an initial tree with a single terminal pane in a Tabs container.
pub fn create_initial_terminal_tree(session_id: SessionId, title: &str) -> egui_tiles::Tree<Pane> {
    let pane = Pane::Terminal {
        session_id,
        title: title.to_string(),
    };
    egui_tiles::Tree::new_tabs("tile_tree", vec![pane])
}

/// Return true if the tree contains at least one Webview pane.
pub fn has_browser_view_pane(tree: &egui_tiles::Tree<Pane>) -> bool {
    for (_, tile) in tree.tiles.iter() {
        if matches!(tile, egui_tiles::Tile::Pane(Pane::BrowserView { .. })) {
            return true;
        }
    }
    false
}

/// Get the webview_id of the currently active webview pane in the tree, if any.
pub fn active_browser_view_id(tree: &egui_tiles::Tree<Pane>) -> Option<u32> {
    for tile_id in tree.active_tiles() {
        if let Some(egui_tiles::Tile::Pane(Pane::BrowserView { webview_id, .. })) =
            tree.tiles.get(tile_id)
        {
            return Some(*webview_id);
        }
    }
    None
}

/// Add a new chat tab (`Pane::Chat`) to the root container of the tree.
pub fn add_chat_tab(tree: &mut egui_tiles::Tree<Pane>, session_id: SessionId, title: &str) {
    let pane = Pane::Chat {
        session_id,
        title: title.to_string(),
    };
    let pane_id = tree.tiles.insert_pane(pane);

    if let Some(root_id) = tree.root() {
        // Try to insert into the root container's children.
        if let Some(egui_tiles::Tile::Container(container)) = tree.tiles.get_mut(root_id) {
            container.add_child(pane_id);
        } else {
            // Root is a pane — wrap it in a tabs container.
            let new_tabs = tree.tiles.insert_tab_tile(vec![root_id, pane_id]);
            tree.root = Some(new_tabs);
        }
    } else {
        // Empty tree — make this pane the root.
        tree.root = Some(pane_id);
    }

    // Make the new tab active.
    tree.make_active(|_, tile| {
        matches!(tile, egui_tiles::Tile::Pane(Pane::Chat { session_id: sid, .. } | Pane::Terminal { session_id: sid, .. }) if *sid == session_id)
    });
}

/// Add a new terminal tab (`Pane::Terminal`) to the root container of the tree.
pub fn add_terminal_tab(tree: &mut egui_tiles::Tree<Pane>, session_id: SessionId, title: &str) {
    let pane = Pane::Terminal {
        session_id,
        title: title.to_string(),
    };
    let pane_id = tree.tiles.insert_pane(pane);

    if let Some(root_id) = tree.root() {
        if let Some(egui_tiles::Tile::Container(container)) = tree.tiles.get_mut(root_id) {
            container.add_child(pane_id);
        } else {
            let new_tabs = tree.tiles.insert_tab_tile(vec![root_id, pane_id]);
            tree.root = Some(new_tabs);
        }
    } else {
        tree.root = Some(pane_id);
    }

    tree.make_active(|_, tile| {
        matches!(tile, egui_tiles::Tile::Pane(Pane::Chat { session_id: sid, .. } | Pane::Terminal { session_id: sid, .. }) if *sid == session_id)
    });
}

/// Split the active pane, creating a Linear container with the original and new pane.
/// `insert_after`: if true new pane goes after active (right/below), if false it goes before (left/above).
pub fn split_pane_dir(
    tree: &mut egui_tiles::Tree<Pane>,
    active_tile: egui_tiles::TileId,
    new_pane: Pane,
    direction: egui_tiles::LinearDir,
    insert_after: bool,
) {
    // Capture the parent BEFORE inserting the new Linear container.
    // Once the Linear is inserted, `active_tile` appears as a child
    // of both the original parent and the new Linear.  Because
    // `parent_of` iterates a HashMap (non-deterministic order), it
    // could return the new Linear instead of the real parent,
    // corrupting the tree.
    let parent_id = tree.tiles.parent_of(active_tile);

    let new_tile_id = tree.tiles.insert_pane(new_pane);

    let tiles_vec = if insert_after {
        vec![active_tile, new_tile_id]
    } else {
        vec![new_tile_id, active_tile]
    };

    let linear_id = match direction {
        egui_tiles::LinearDir::Horizontal => tree.tiles.insert_horizontal_tile(tiles_vec),
        egui_tiles::LinearDir::Vertical => tree.tiles.insert_vertical_tile(tiles_vec),
    };

    if let Some(parent_id) = parent_id {
        if let Some(egui_tiles::Tile::Container(container)) = tree.tiles.get_mut(parent_id) {
            container.retain(|id| id != active_tile);
            container.add_child(linear_id);
            // Update the active tab to point to the new linear container
            // that now wraps the original pane.
            if let egui_tiles::Container::Tabs(tabs) = container {
                tabs.set_active(linear_id);
            }
        }
    } else {
        tree.root = Some(linear_id);
    }
}

/// Walk to the first leaf pane from a tile ID, traversing containers.
/// Returns a reference to the first `Pane` found, or `None`.
pub(super) fn first_leaf_pane(
    tiles: &egui_tiles::Tiles<Pane>,
    tile_id: egui_tiles::TileId,
) -> Option<&Pane> {
    let mut current = tile_id;
    loop {
        match tiles.get(current) {
            Some(egui_tiles::Tile::Pane(pane)) => return Some(pane),
            Some(egui_tiles::Tile::Container(container)) => {
                if let Some(&first_child) = container.children().next() {
                    current = first_child;
                } else {
                    return None;
                }
            }
            None => return None,
        }
    }
}

/// Find the TileId for a terminal pane with the given session_id.
pub fn find_terminal_tile(
    tree: &egui_tiles::Tree<Pane>,
    session_id: SessionId,
) -> Option<egui_tiles::TileId> {
    // Try Chat variant first, then Terminal — PartialEq only compares
    // within the same variant, so we must probe both.
    let chat_needle = Pane::Chat {
        session_id,
        title: String::new(),
    };
    if let Some(id) = tree.tiles.find_pane(&chat_needle) {
        return Some(id);
    }
    let terminal_needle = Pane::Terminal {
        session_id,
        title: String::new(),
    };
    tree.tiles.find_pane(&terminal_needle)
}

/// Remove a terminal pane from the tree by session ID.
pub fn remove_terminal_pane(tree: &mut egui_tiles::Tree<Pane>, session_id: SessionId) {
    if let Some(tile_id) = find_terminal_tile(tree, session_id) {
        tree.remove_recursively(tile_id);
    }
}

/// Get the active (focused) terminal session ID from the tree, if any.
pub fn active_terminal_session(tree: &egui_tiles::Tree<Pane>) -> Option<SessionId> {
    for tile_id in tree.active_tiles() {
        if let Some(egui_tiles::Tile::Pane(
            Pane::Chat { session_id, .. } | Pane::Terminal { session_id, .. },
        )) = tree.tiles.get(tile_id)
        {
            return Some(*session_id);
        }
    }
    // Fallback: return the first terminal pane.
    for (_, tile) in tree.tiles.iter() {
        if let egui_tiles::Tile::Pane(
            Pane::Chat { session_id, .. } | Pane::Terminal { session_id, .. },
        ) = tile
        {
            return Some(*session_id);
        }
    }
    None
}

/// Update the title of a terminal pane in the tree.
#[allow(dead_code)]
pub fn update_terminal_title(
    tree: &mut egui_tiles::Tree<Pane>,
    session_id: SessionId,
    new_title: &str,
) {
    if let Some(tile_id) = find_terminal_tile(tree, session_id)
        && let Some(egui_tiles::Tile::Pane(Pane::Chat { title, .. } | Pane::Terminal { title, .. })) =
            tree.tiles.get_mut(tile_id)
    {
        *title = new_title.to_string();
    }
}

/// Update the title of a webview pane in the tree.
pub fn update_browser_view_title(
    tree: &mut egui_tiles::Tree<Pane>,
    webview_id: u32,
    new_title: &str,
) {
    if let Some(tile_id) = find_browser_view_tile(tree, webview_id)
        && let Some(egui_tiles::Tile::Pane(Pane::BrowserView { title, .. })) =
            tree.tiles.get_mut(tile_id)
    {
        *title = new_title.to_string();
    }
}

/// Update the URL of a webview pane in the tree.
///
/// Called when the webview navigates (link clicks, redirects, etc.) so
/// the Pane's own `url` field stays in sync with the actual webview URL.
pub fn update_browser_view_url(tree: &mut egui_tiles::Tree<Pane>, webview_id: u32, new_url: &str) {
    if let Some(tile_id) = find_browser_view_tile(tree, webview_id)
        && let Some(egui_tiles::Tile::Pane(Pane::BrowserView { url, .. })) =
            tree.tiles.get_mut(tile_id)
    {
        *url = new_url.to_string();
    }
}

/// Activate a specific terminal pane by session ID.
pub fn activate_terminal_tab(tree: &mut egui_tiles::Tree<Pane>, session_id: SessionId) {
    tree.make_active(|_, tile| {
        matches!(
            tile,
            egui_tiles::Tile::Pane(Pane::Chat { session_id: s, .. } | Pane::Terminal { session_id: s, .. }) if *s == session_id
        )
    });
}

/// Activate a specific webview pane by webview ID.
pub fn activate_browser_view_tab(tree: &mut egui_tiles::Tree<Pane>, webview_id: u32) {
    tree.make_active(|_, tile| {
        matches!(
            tile,
            egui_tiles::Tile::Pane(Pane::BrowserView { webview_id: wid, .. }) if *wid == webview_id
        )
    });
}

/// Collect all terminal session IDs from the tree in iteration order.
pub fn terminal_session_ids(tree: &egui_tiles::Tree<Pane>) -> Vec<SessionId> {
    let mut ids = Vec::new();
    for (_, tile) in tree.tiles.iter() {
        if let egui_tiles::Tile::Pane(
            Pane::Chat { session_id, .. } | Pane::Terminal { session_id, .. },
        ) = tile
        {
            ids.push(*session_id);
        }
    }
    ids
}

/// Switch to the next terminal tab (wraps around).
pub fn next_terminal_tab(tree: &mut egui_tiles::Tree<Pane>) {
    let ids = terminal_session_ids(tree);
    if ids.is_empty() {
        return;
    }
    let current = active_terminal_session(tree);
    let idx = current
        .and_then(|c| ids.iter().position(|&id| id == c))
        .unwrap_or(0);
    let next_idx = (idx + 1) % ids.len();
    activate_terminal_tab(tree, ids[next_idx]);
}

/// Switch to the previous terminal tab (wraps around).
pub fn prev_terminal_tab(tree: &mut egui_tiles::Tree<Pane>) {
    let ids = terminal_session_ids(tree);
    if ids.is_empty() {
        return;
    }
    let current = active_terminal_session(tree);
    let idx = current
        .and_then(|c| ids.iter().position(|&id| id == c))
        .unwrap_or(0);
    let prev_idx = if idx == 0 { ids.len() - 1 } else { idx - 1 };
    activate_terminal_tab(tree, ids[prev_idx]);
}

// ─── Webview Pane Helpers ────────────────────────────────────────────────────

/// Add a webview as a new tab in the root container of the tree.
pub fn add_browser_view_tab(
    tree: &mut egui_tiles::Tree<Pane>,
    webview_id: u32,
    title: &str,
    url: &str,
) {
    let pane = Pane::BrowserView {
        webview_id,
        title: title.to_string(),
        url: url.to_string(),
    };
    let pane_id = tree.tiles.insert_pane(pane);

    if let Some(root_id) = tree.root() {
        if let Some(egui_tiles::Tile::Container(container)) = tree.tiles.get_mut(root_id) {
            container.add_child(pane_id);
        } else {
            let new_tabs = tree.tiles.insert_tab_tile(vec![root_id, pane_id]);
            tree.root = Some(new_tabs);
        }
    } else {
        tree.root = Some(pane_id);
    }

    // Make the new webview tab active.
    tree.make_active(|_, tile| {
        matches!(tile, egui_tiles::Tile::Pane(Pane::BrowserView { webview_id: wid, .. }) if *wid == webview_id)
    });
}

/// Find the TileId for a webview pane with the given webview_id.
pub fn find_browser_view_tile(
    tree: &egui_tiles::Tree<Pane>,
    webview_id: u32,
) -> Option<egui_tiles::TileId> {
    let needle = Pane::BrowserView {
        webview_id,
        title: String::new(),
        url: String::new(),
    };
    tree.tiles.find_pane(&needle)
}

/// Remove a webview pane from the tree by webview ID.
pub fn remove_browser_view_pane(tree: &mut egui_tiles::Tree<Pane>, webview_id: u32) {
    if let Some(tile_id) = find_browser_view_tile(tree, webview_id) {
        tree.remove_recursively(tile_id);
    }
}

// ─── Channel Pane Helpers ────────────────────────────────────────────────────

/// Add a channel as a new tab in the root container of the tree.
pub fn add_channel_tab(tree: &mut egui_tiles::Tree<Pane>, channel_id: &str, channel_name: &str) {
    // Avoid duplicates — if a tab for this channel already exists, just activate it.
    if find_channel_tile(tree, channel_id).is_some() {
        activate_channel_tab(tree, channel_id);
        return;
    }
    let pane = Pane::Channel {
        channel_id: channel_id.to_string(),
        channel_name: channel_name.to_string(),
    };
    let pane_id = tree.tiles.insert_pane(pane);

    if let Some(root_id) = tree.root() {
        if let Some(egui_tiles::Tile::Container(container)) = tree.tiles.get_mut(root_id) {
            container.add_child(pane_id);
        } else {
            let new_tabs = tree.tiles.insert_tab_tile(vec![root_id, pane_id]);
            tree.root = Some(new_tabs);
        }
    } else {
        tree.root = Some(pane_id);
    }

    let cid = channel_id.to_string();
    tree.make_active(|_, tile| {
        matches!(tile, egui_tiles::Tile::Pane(Pane::Channel { channel_id, .. }) if *channel_id == cid)
    });
}

/// Find the TileId for a channel pane with the given channel_id.
pub fn find_channel_tile(
    tree: &egui_tiles::Tree<Pane>,
    channel_id: &str,
) -> Option<egui_tiles::TileId> {
    let needle = Pane::Channel {
        channel_id: channel_id.to_string(),
        channel_name: String::new(), // PartialEq checks channel_id only
    };
    tree.tiles.find_pane(&needle)
}

/// Remove a channel pane from the tree by channel ID.
pub fn remove_channel_pane(tree: &mut egui_tiles::Tree<Pane>, channel_id: &str) {
    if let Some(tile_id) = find_channel_tile(tree, channel_id) {
        tree.remove_recursively(tile_id);
    }
}

/// Activate a specific channel pane by channel ID.
pub fn activate_channel_tab(tree: &mut egui_tiles::Tree<Pane>, channel_id: &str) {
    let cid = channel_id.to_string();
    tree.make_active(|_, tile| {
        matches!(tile, egui_tiles::Tile::Pane(Pane::Channel { channel_id, .. }) if *channel_id == cid)
    });
}
