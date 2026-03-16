use cronymax::renderer::atlas::CellSize;
use cronymax::ui::styles::Styles;
use cronymax::ui::tiles;
use cronymax::ui::{self, Viewport};

#[test]
fn test_viewport_from_window_size() {
    let vp = Viewport::from_window_size(800, 600);
    assert_eq!(vp.x, 4.0);
    assert_eq!(vp.y, 4.0);
    assert_eq!(vp.width, 792.0); // 800 - 2*4
    assert_eq!(vp.height, 592.0); // 600 - 2*4
}

#[test]
fn test_viewport_grid_dimensions() {
    let vp = Viewport {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };
    let cell = CellSize {
        width: 10.0,
        height: 20.0,
    };
    let (cols, rows) = vp.grid_dimensions(&cell);
    assert_eq!(cols, 80);
    assert_eq!(rows, 30);
}

#[test]
fn test_compute_single_pane() {
    let cell = CellSize {
        width: 10.0,
        height: 20.0,
    };
    let styles = Styles::default();
    let (vp, cols, rows) = ui::compute_single_pane(808, 608, &cell, &styles);
    assert_eq!(vp.x, 4.0);
    // y = tab_bar_height (34) + PADDING(0) = 34
    assert_eq!(vp.y, 34.0);
    assert_eq!(cols, 80);
    // height = 608 - 34 - 4 = 570, rows = floor(570/20) = 28
    assert_eq!(rows, 28);
}

#[test]
fn test_tile_tree_basic() {
    let mut tree = tiles::create_initial_tree(1, "tab1");
    assert_eq!(tiles::active_terminal_session(&tree), Some(1));
    let ids = tiles::terminal_session_ids(&tree);
    assert_eq!(ids.len(), 1);

    tiles::add_chat_tab(&mut tree, 2, "tab2");
    let ids = tiles::terminal_session_ids(&tree);
    assert_eq!(ids.len(), 2);
    // New tab is activated.
    assert_eq!(tiles::active_terminal_session(&tree), Some(2));

    tiles::next_terminal_tab(&mut tree);
    assert_eq!(tiles::active_terminal_session(&tree), Some(1));

    tiles::prev_terminal_tab(&mut tree);
    assert_eq!(tiles::active_terminal_session(&tree), Some(2));
}

#[test]
fn test_tile_tree_remove() {
    let mut tree = tiles::create_initial_tree(1, "tab1");
    tiles::add_chat_tab(&mut tree, 2, "tab2");
    tiles::add_chat_tab(&mut tree, 3, "tab3");
    assert_eq!(tiles::active_terminal_session(&tree), Some(3));

    tiles::remove_terminal_pane(&mut tree, 3);
    let ids = tiles::terminal_session_ids(&tree);
    assert_eq!(ids.len(), 2);
    // Active should fall back to another session.
    let active = tiles::active_terminal_session(&tree);
    assert!(active == Some(1) || active == Some(2));
}

#[test]
fn test_tile_tree_empty() {
    let mut tree = tiles::create_initial_tree(1, "tab1");
    tiles::remove_terminal_pane(&mut tree, 1);
    assert_eq!(tiles::active_terminal_session(&tree), None);
}

#[test]
fn test_small_window_grid_minimum() {
    let cell = CellSize {
        width: 10.0,
        height: 20.0,
    };
    let styles = Styles::default();
    // Very small window
    let (_, cols, rows) = ui::compute_single_pane(10, 10, &cell, &styles);
    assert!(cols >= 1);
    assert!(rows >= 1);
}
