//! Behavior struct constructor and egui_tiles::Behavior trait implementation.

use crate::ui::tiles::browser::BrowserViewPane;

use super::channel::ChannelPaneView;
use super::chat::ChatPaneView;
use super::terminal::TerminalPaneView;
use super::*;

/// Drives egui_tiles layout: tells it how to render tabs and pane areas.
pub struct Behavior<'a> {
    /// Parent fragment — pane widgets compose through view types dispatched via `fragment.add(…)`.
    pub fragment: Fragment<'a>,
    /// Mutable per-frame state (editors, blocks, etc.).
    pub state: FrameState<'a>,
    /// Persistent pane widget instances.
    pub widgets: &'a mut PaneWidgetStore,
    /// Whether the address bar is being edited (prevents auto-focus stealing).
    pub address_bar_editing: bool,
    /// Session IDs of pinned tabs (used to show pin/unpin state).
    pub pinned_tabs: Vec<u32>,
    /// Number of visible (non-pinned) tabs for dynamic width calc.
    pub visible_tab_count: usize,
    /// Available tab bar width (pixels) for dynamic width calc.
    pub tab_bar_width: f32,
    /// Tooltip request from a docked webview address bar button hover.
    pub tooltip: Option<TooltipRequest>,
    /// Pending star toggle request: (session_id, message_id).
    pub pending_star_toggle: Option<(u32, u32)>,
    /// Session ID of the terminal pane that received a pointer click this frame.
    pub clicked_terminal_session: Option<SessionId>,
}

impl<'a> egui_tiles::Behavior<Pane> for Behavior<'a> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        match pane {
            Pane::Chat { title, .. } | Pane::Terminal { title, .. } => title.as_str().into(),
            Pane::BrowserView { title, .. } => format!("● {title}").into(),
            Pane::Channel { channel_name, .. } => format!("# {channel_name}").into(),
        }
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut Pane,
    ) -> egui_tiles::UiResponse {
        ui.painter().rect_filled(
            ui.max_rect(),
            egui::CornerRadius::ZERO,
            self.fragment.colors.bg_base,
        );

        match pane {
            Pane::Chat { session_id, .. } => {
                let widget = self.widgets.chat_widget(*session_id);
                let state = &mut self.state;
                self.fragment
                    .duplicate()
                    .with(ui)
                    .add(ChatPaneView { widget, state });
                if let Some(sid) = self.fragment.ui_state.focused_terminal_session {
                    self.clicked_terminal_session = Some(sid);
                }
                egui_tiles::UiResponse::None
            }
            Pane::Terminal { session_id, .. } => {
                let widget = self.widgets.terminal_widget(*session_id);
                self.fragment
                    .duplicate()
                    .with(ui)
                    .add(TerminalPaneView { widget });
                if let Some(sid) = self.fragment.ui_state.focused_terminal_session {
                    self.clicked_terminal_session = Some(sid);
                }
                egui_tiles::UiResponse::None
            }
            Pane::BrowserView {
                webview_id, url, ..
            } => {
                let widget = self.widgets.browser_widget(*webview_id);
                // Only overwrite the widget URL when the user is NOT
                // editing — otherwise their in-progress typing gets
                // discarded every frame, causing the text to flash.
                if !widget.editing {
                    widget.url.clone_from(url);
                }
                self.fragment
                    .duplicate()
                    .with(ui)
                    .add::<BrowserViewPane, _>(widget);
                // Sync state back from widget.
                let widget = self.widgets.browser_widget(*webview_id);
                url.clone_from(&widget.url);
                self.address_bar_editing = widget.editing;
                if let Some(tt) = widget.tooltip.take() {
                    self.tooltip = Some(tt);
                }
                egui_tiles::UiResponse::None
            }
            Pane::Channel {
                channel_id,
                channel_name,
            } => {
                let widget = self
                    .widgets
                    .channel_widget(channel_id.clone(), channel_name.clone());
                let state = &mut self.state;
                self.fragment
                    .duplicate()
                    .with(ui)
                    .add(ChannelPaneView { widget, state });
                egui_tiles::UiResponse::None
            }
        }
    }

    fn is_tab_closable(
        &self,
        _tiles: &egui_tiles::Tiles<Pane>,
        _tile_id: egui_tiles::TileId,
    ) -> bool {
        true
    }

    fn on_tab_close(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> bool {
        if let Some(egui_tiles::Tile::Pane(pane)) = tiles.get(tile_id) {
            match pane {
                Pane::Chat { session_id, .. } | Pane::Terminal { session_id, .. } => {
                    self.widgets.remove_terminal(*session_id);
                    self.fragment
                        .dirties
                        .actions
                        .push(UiAction::CloseTab(*session_id));
                }
                Pane::BrowserView { webview_id, .. } => {
                    self.widgets.remove_browser(*webview_id);
                    self.fragment
                        .dirties
                        .actions
                        .push(UiAction::CloseWebview(*webview_id));
                }
                Pane::Channel { channel_id, .. } => {
                    self.widgets.remove_channel(channel_id);
                    self.fragment
                        .dirties
                        .actions
                        .push(UiAction::CloseChannel(channel_id.clone()));
                }
            }
        }
        true // Allow egui_tiles to remove the tile from the tree.
    }

    fn top_bar_right_ui(
        &mut self,
        tiles: &egui_tiles::Tiles<Pane>,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        tabs: &egui_tiles::Tabs,
        _scroll_offset: &mut f32,
    ) {
        // Right padding to prevent buttons from being clipped by the window edge.
        ui.add_space(self.fragment.styles.spacing.medium);

        // Walk through Linear containers to find the first leaf pane,
        // so split views also get tab-bar actions.
        let first_leaf = tabs.active.and_then(|id| first_leaf_pane(tiles, id));
        if let Some(pane) = first_leaf {
            let is_browser_view = matches!(pane, Pane::BrowserView { .. });

            // Split pane buttons (terminal & webview).
            // For webviews emit DockWebview* instead of Split* so the
            // webview is split within the tile tree rather than spawning
            // a new terminal session.
            if icons::icon_button(
                ui,
                icons::IconButtonCfg {
                    icon: Icon::SplitHorizontal,
                    tooltip: t("browser.split_horizontal"),
                    base_color: ui.visuals().weak_text_color(),
                    hover_color: ui.visuals().text_color(),
                    pixel_size: self.fragment.styles.typography.body0,
                    margin: self.fragment.styles.spacing.small,
                },
            )
            .clicked()
            {
                if is_browser_view {
                    self.fragment
                        .dirties
                        .actions
                        .push(UiAction::DockWebviewRight);
                } else {
                    self.fragment.dirties.actions.push(UiAction::SplitRight);
                }
            }
            if icons::icon_button(
                ui,
                icons::IconButtonCfg {
                    icon: Icon::SplitVertical,
                    tooltip: t("browser.split_vertical"),
                    base_color: ui.visuals().weak_text_color(),
                    hover_color: ui.visuals().text_color(),
                    pixel_size: self.fragment.styles.typography.body0,
                    margin: self.fragment.styles.spacing.small,
                },
            )
            .clicked()
            {
                if is_browser_view {
                    self.fragment
                        .dirties
                        .actions
                        .push(UiAction::DockWebviewDown);
                } else {
                    self.fragment.dirties.actions.push(UiAction::SplitDown);
                }
            }

            // Webview-only actions: open-as-tab, pop-out overlay, open in system browser.
            if is_browser_view {
                let webview_id: u32 = match pane {
                    Pane::BrowserView { webview_id, .. } => *webview_id,
                    _ => 0,
                };
                if icons::icon_button(
                    ui,
                    icons::IconButtonCfg {
                        icon: Icon::OpenInProduct,
                        tooltip: t("browser.open_as_tab"),
                        base_color: ui.visuals().weak_text_color(),
                        hover_color: ui.visuals().text_color(),
                        pixel_size: self.fragment.styles.typography.body0,
                        margin: self.fragment.styles.spacing.small,
                    },
                )
                .clicked()
                {
                    self.fragment
                        .dirties
                        .actions
                        .push(UiAction::WebviewToTab(webview_id));
                }
                if icons::icon_button(
                    ui,
                    icons::IconButtonCfg {
                        icon: Icon::ChromeMaximize,
                        tooltip: t("browser.pop_out"),
                        base_color: ui.visuals().weak_text_color(),
                        hover_color: ui.visuals().text_color(),
                        pixel_size: self.fragment.styles.typography.body0,
                        margin: self.fragment.styles.spacing.small,
                    },
                )
                .clicked()
                {
                    self.fragment.dirties.actions.push(UiAction::PopOutOverlay);
                }
                if icons::icon_button(
                    ui,
                    icons::IconButtonCfg {
                        icon: Icon::Globe,
                        tooltip: t("browser.open_system"),
                        base_color: ui.visuals().weak_text_color(),
                        hover_color: ui.visuals().text_color(),
                        pixel_size: self.fragment.styles.typography.body0,
                        margin: self.fragment.styles.spacing.small,
                    },
                )
                .clicked()
                {
                    self.fragment
                        .dirties
                        .actions
                        .push(UiAction::OpenInSystemBrowser);
                }
            }

            // Pin/Unpin — works for both terminal and webview tabs.
            let pin_id: u32 = match pane {
                Pane::Chat { session_id, .. } | Pane::Terminal { session_id, .. } => *session_id,
                Pane::BrowserView { webview_id, .. } => *webview_id,
                Pane::Channel { .. } => 0,
            };
            if pin_id != 0 {
                let is_pinned = self.pinned_tabs.contains(&pin_id);
                let (icon, tooltip) = if is_pinned {
                    (Icon::Pinned, t("tabs.unpin"))
                } else {
                    (Icon::Pin, t("tabs.pin"))
                };
                if icons::icon_button(
                    ui,
                    icons::IconButtonCfg {
                        icon,
                        tooltip,
                        base_color: ui.visuals().weak_text_color(),
                        hover_color: ui.visuals().text_color(),
                        pixel_size: self.fragment.styles.typography.body0,
                        margin: self.fragment.styles.spacing.small,
                    },
                )
                .clicked()
                {
                    if is_pinned {
                        self.fragment
                            .dirties
                            .actions
                            .push(UiAction::UnpinTab(pin_id));
                    } else {
                        self.fragment.dirties.actions.push(UiAction::PinTab(pin_id));
                    }
                }
            }
        }
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<Pane>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            self.fragment.colors.bg_base
        } else {
            egui::Color32::TRANSPARENT
        }
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        self.fragment.styles.tab_bar_height()
    }
    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        // self.fragment.colors.bg_body
        egui::Color32::TRANSPARENT
    }

    /// Custom tab_ui: hide pinned tabs from the tab bar (zero-width) while
    /// keeping them in the tile tree so switching still works via titlebar chips.
    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        // If this tile (or its first leaf pane) is pinned, hide it completely.
        let is_pinned = first_leaf_pane(tiles, tile_id).is_some_and(|p| {
            let id: u32 = match p {
                Pane::Chat { session_id, .. } | Pane::Terminal { session_id, .. } => *session_id,
                Pane::BrowserView { webview_id, .. } => *webview_id,
                _ => 0,
            };
            id != 0 && self.pinned_tabs.contains(&id)
        });
        if is_pinned {
            let (_, response) = ui.allocate_exact_size(egui::Vec2::ZERO, egui::Sense::hover());
            return response;
        }

        // ── Dynamic-width tab rendering ──
        let colors = self.fragment.colors.clone();
        let text = self
            .tab_title_for_tile(tiles, tile_id)
            .color(if state.active {
                colors.primary
            } else {
                colors.text_title
            });
        let close_btn_size = egui::Vec2::splat(self.close_button_outer_size());
        let close_btn_left_padding = self.fragment.styles.spacing.small;
        let font_id = egui::TextStyle::Button.resolve(ui.style());

        let styles = &self.fragment.styles;
        let x_margin = self.tab_title_spacing(ui.visuals());
        let browser_w = 2.0 * x_margin
            + f32::from(state.closable) * (close_btn_left_padding + close_btn_size.x);

        // Compute per-tab width budget: evenly divide available bar width.
        // Reserve space for right-side buttons (new tab, pin, scroll arrows).
        let right_buttons_w = self.fragment.styles.typography.line_height * 4.5;
        let avail = (self.tab_bar_width - right_buttons_w).max(0.0);
        let n = self.visible_tab_count.max(1) as f32;
        let min_tab_w = self.fragment.styles.typography.line_height * 3.0;
        let max_tab_w = self.fragment.styles.typography.line_height * 9.0;
        let budget_per_tab = (avail / n).clamp(min_tab_w, max_tab_w);
        let max_text_w =
            (budget_per_tab - browser_w).max(self.fragment.styles.typography.line_height);

        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Truncate), max_text_w, font_id);

        let button_width =
            (galley.size().x.min(max_text_w) + browser_w).clamp(min_tab_w, budget_per_tab);
        let (_tab_rect_id, tab_rect) =
            ui.allocate_space(egui::vec2(button_width, ui.available_height()));

        // Use a custom ID (not the tile's egui ID) so egui_tiles'
        // internal DnD does not activate.  Our own TabDragPayload
        // system is the sole drag mechanism, ensuring tabs can only
        // be split — never rearranged into nested Tabs containers.
        let tab_response = ui
            .interact(tab_rect, id.with("tab_dnd"), egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Grab);

        if state.active {
            ui.painter().rect_filled(
                tab_rect,
                egui::CornerRadius::ZERO,
                self.fragment.colors.bg_base,
            );
        }

        // Initiate custom DnD payload for terminal tabs.
        if tab_response.drag_started()
            && let Some(egui_tiles::Tile::Pane(
                Pane::Chat { session_id, .. } | Pane::Terminal { session_id, .. },
            )) = tiles.get(tile_id)
        {
            egui::DragAndDrop::set_payload(
                ui.ctx(),
                TabDragPayload {
                    session_id: *session_id,
                },
            );
        }

        let is_custom_dragging = tab_response.dragged()
            && egui::DragAndDrop::has_payload_of_type::<TabDragPayload>(ui.ctx());

        if ui.is_rect_visible(tab_rect) && !state.is_being_dragged && !is_custom_dragging {
            // Hover detection (before painting so the animation drives the fill).
            let pointer_in_tab = ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| tab_rect.contains(p));

            ui.painter().vline(
                tab_rect.right(),
                tab_rect
                    .y_range()
                    .shrink(self.fragment.styles.spacing.small),
                egui::Stroke::new(
                    self.fragment.styles.sizes.border,
                    self.fragment.colors.bg_base,
                ),
            );

            ui.painter().galley(
                egui::Align2::LEFT_CENTER
                    .align_size_within_rect(galley.size(), tab_rect.shrink(x_margin))
                    .min,
                galley,
                self.fragment.colors.text_title,
            );

            // Reuse pointer_in_tab from hover animation above.
            if state.closable && pointer_in_tab {
                // Place close button at the right edge inside the tab rect.
                let btn_size =
                    self.fragment.styles.typography.body0 + self.fragment.styles.spacing.small;
                let close_rect = egui::Rect::from_center_size(
                    egui::pos2(
                        tab_rect.right()
                            - btn_size / 2.0
                            - self.fragment.styles.spacing.small / 2.0,
                        tab_rect.center().y,
                    ),
                    egui::vec2(btn_size, btn_size),
                );
                let close_btn_response = crate::ui::icons::icon_button_at(
                    ui,
                    close_rect,
                    icons::IconButtonCfg {
                        icon: Icon::Close,
                        tooltip: t("browser.close"),
                        base_color: ui.visuals().weak_text_color(),
                        hover_color: ui.visuals().text_color(),
                        pixel_size: styles.typography.body0,
                        margin: styles.spacing.small,
                    },
                );

                if close_btn_response.clicked() && self.on_tab_close(tiles, tile_id) {
                    tiles.remove(tile_id);
                }
            }
        }

        // Render a floating ghost label near the cursor while dragging.
        if is_custom_dragging && let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            let ghost_title = self.tab_title_for_tile(tiles, tile_id);
            egui::Area::new(egui::Id::new("tab_drag_ghost"))
                .order(egui::Order::Tooltip)
                .fixed_pos(
                    pos + egui::vec2(
                        self.fragment.styles.spacing.medium + self.fragment.styles.spacing.small,
                        self.fragment.styles.spacing.medium + self.fragment.styles.spacing.small,
                    ),
                )
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(ghost_title);
                    });
                });
        }

        self.on_tab_button(tiles, tile_id, tab_response)
    }

    /// For split containers (Linear), walk to the first leaf pane and use
    /// its title so the tab bar shows a meaningful name instead of
    /// "Horizontal" / "Vertical".
    fn tab_title_for_tile(
        &mut self,
        tiles: &egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
    ) -> egui::WidgetText {
        let mut current = tile_id;
        loop {
            match tiles.get(current) {
                Some(egui_tiles::Tile::Pane(pane)) => return self.tab_title_for_pane(pane),
                Some(egui_tiles::Tile::Container(container)) => {
                    let mut children_iter = container.children();
                    if let Some(&first_child) = children_iter.next() {
                        current = first_child;
                    } else {
                        return format!("{:?}", container.kind()).into();
                    }
                }
                None => return "".into(),
            }
        }
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            // Split panes live directly in Linear containers without
            // individual tab bars. Only multi-tab containers show a bar.
            all_panes_must_have_tabs: false,
            prune_single_child_tabs: false,
            ..Default::default()
        }
    }
}
