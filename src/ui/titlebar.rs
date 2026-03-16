//! Titlebar widget (§1) — macOS controls, pinned tabs, right-side actions.

use super::actions::UiAction;
use super::i18n::t;
use super::icons::{self, Icon};
use super::styles::Styles;
use super::styles::colors::Colors;
use super::types::UiState;
use super::widget::{Fragment, Widget};

/// Titlebar panel widget — macOS controls, pinned tabs, right-side actions.
pub struct TitlebarWidget;

impl Widget for TitlebarWidget {
    /// Draw the titlebar: macOS traffic lights (left), drag area (center), split
    /// buttons and non-macOS window controls (right). Tabs are rendered by
    /// egui_tiles' native tab bar inside the CentralPanel, so the titlebar only
    /// needs chrome.
    fn render(&mut self, #[allow(unused)] mut f: Fragment<'_, egui::Context>) {
        let ctx = f.ctx();
        let styles = f.styles;
        let ui_state = &mut *f.ui_state;
        let colors = &f.colors;
        let actions = &mut f.dirties.actions;
        egui::TopBottomPanel::top("titlebar")
            .exact_height(styles.titlebar_height())
            .frame(
                egui::Frame::new()
                    .fill(colors.bg_float)
                    .stroke(egui::Stroke::new(styles.sizes.border, colors.border))
                    .inner_margin(egui::Margin {
                        left: styles.spacing.medium as i8,
                        right: styles.spacing.large as i8,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // macOS: draw custom window control buttons (close/minimize/zoom).
                    #[cfg(target_os = "macos")]
                    self.draw_macos_window_controls(ui, styles, actions);

                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        self.draw_pinned_tabs(ui, ui_state, styles, colors, actions);
                        self.draw_drag_area(ui, actions);
                    });

                    // Right-aligned area: split buttons + non-macOS window controls.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Linux/Windows window controls (rightmost).
                        #[cfg(not(target_os = "macos"))]
                        {
                            let close_btn = icons::icon_button(
                                ui,
                                icons::IconButtonCfg {
                                    icon: Icon::ChromeClose,
                                    tooltip: t("titlebar.close"),
                                    base_color: colors.text_title,
                                    hover_color: colors.text_title,
                                    pixel_size: styles.typography.title3,
                                    margin: styles.spacing.medium,
                                },
                            );
                            if close_btn.clicked() {
                                actions.push(UiAction::CloseWindow);
                            }

                            if icons::icon_button(
                                ui,
                                icons::IconButtonCfg {
                                    icon: Icon::ChromeMaximize,
                                    tooltip: t("titlebar.maximize"),
                                    base_color: colors.text_title,
                                    hover_color: colors.text_title,
                                    pixel_size: styles.typography.title3,
                                    margin: styles.spacing.medium,
                                },
                            )
                            .clicked()
                            {
                                actions.push(UiAction::ToggleMaximize);
                            }

                            if icons::icon_button(
                                ui,
                                icons::IconButtonCfg {
                                    icon: Icon::ChromeMinimize,
                                    tooltip: t("titlebar.minimize"),
                                    base_color: colors.text_title,
                                    hover_color: colors.text_title,
                                    pixel_size: styles.typography.title3,
                                    margin: styles.spacing.medium,
                                },
                            )
                            .clicked()
                            {
                                actions.push(UiAction::Minimize);
                            }
                        }

                        // Settings gear icon.
                        if icons::icon_button(
                            ui,
                            icons::IconButtonCfg {
                                icon: Icon::SettingsGear,
                                tooltip: "Settings",
                                base_color: colors.text_title,
                                hover_color: colors.text_title,
                                pixel_size: styles.typography.title3,
                                margin: styles.spacing.medium,
                            },
                        )
                        .clicked()
                        {
                            actions.push(UiAction::OpenSettings);
                        }

                        // ── Profile selector ComboBox ────────────────────────
                        self.draw_profile_selector(ui, ui_state, styles, colors, actions);

                        ui.add(
                            egui::Separator::default()
                                .vertical()
                                .shrink(styles.spacing.small * 2.0),
                        );

                        // New Chat button — the default "+" action.
                        if icons::icon_button(
                            ui,
                            icons::IconButtonCfg {
                                icon: Icon::ChatSparkle,
                                tooltip: "New Chat",
                                base_color: colors.text_title,
                                hover_color: colors.text_title,
                                pixel_size: styles.typography.title3,
                                margin: styles.spacing.medium,
                            },
                        )
                        .clicked()
                        {
                            actions.push(UiAction::NewChat);
                        }

                        // New Terminal button.
                        if icons::icon_button(
                            ui,
                            icons::IconButtonCfg {
                                icon: Icon::Terminal,
                                tooltip: "New Terminal",
                                base_color: colors.text_title,
                                hover_color: colors.text_title,
                                pixel_size: styles.typography.title3,
                                margin: styles.spacing.medium,
                            },
                        )
                        .clicked()
                        {
                            actions.push(UiAction::NewTerminal);
                        }

                        // Popup Overlay (Globe icon).
                        if icons::icon_button(
                            ui,
                            icons::IconButtonCfg {
                                icon: Icon::Globe,
                                tooltip: "Popup Overlay",
                                base_color: colors.text_title,
                                hover_color: colors.text_title,
                                pixel_size: styles.typography.title3,
                                margin: styles.spacing.medium,
                            },
                        )
                        .clicked()
                        {
                            actions.push(UiAction::OpenOverlay);
                        }

                        // Feishu/Lark channel icon (visible only when Claw mode is enabled).
                        if ui_state.claw_enabled {
                            let (status_color, tooltip) = match ui_state.channel_connection_state {
                                crate::channel::ConnectionState::Connected => (
                                    Some(colors.success), //from_rgb(40, 200, 64)),
                                    "Feishu (Connected)",
                                ),
                                crate::channel::ConnectionState::Connecting
                                | crate::channel::ConnectionState::Reconnecting => (
                                    Some(colors.warning), //from_rgb(255, 189, 46)),
                                    "Feishu (Connecting...)",
                                ),
                                crate::channel::ConnectionState::Error => (
                                    Some(colors.danger), //from_rgb(255, 96, 92)),
                                    "Feishu (Error)",
                                ),
                                _ => (None, "Feishu (Disconnected)"),
                            };
                            if icons::icon_button_with_status(
                                ui,
                                icons::IconButtonStatusCfg {
                                    icon: Icon::Feishu,
                                    tooltip,
                                    pixel_size: styles.typography.title3,
                                    stroke: status_color
                                        .map(|c| egui::Stroke::new(styles.sizes.border, c)),
                                    corner_radius: styles.radii.md.into(),
                                },
                            )
                            .clicked()
                            {
                                actions.push(UiAction::OpenChannelTab {
                                    channel_id: "lark".to_string(),
                                    channel_name: "Feishu".to_string(),
                                });
                            }
                        }
                    });
                });
            });
        f.add(super::relaunch_dialog::RelaunchDialog);
    }
}

impl TitlebarWidget {
    #[cfg(target_os = "macos")]
    fn draw_macos_window_controls(
        &self,
        ui: &mut egui::Ui,
        styles: &Styles,
        actions: &mut Vec<UiAction>,
    ) {
        let btn_radius = styles.radii.sm;
        let spacing = styles.spacing.large + styles.spacing.small;
        let start_x = styles.spacing.medium;

        // Platform-standard macOS traffic light colors — intentionally not themed.
        let close_color = egui::Color32::from_rgb(255, 96, 92); // red
        let minimize_color = egui::Color32::from_rgb(255, 189, 46); // yellow
        let zoom_color = egui::Color32::from_rgb(40, 200, 64); // green

        let buttons = [
            (close_color, "×", UiAction::CloseWindow, t("titlebar.close")),
            (
                minimize_color,
                "−",
                UiAction::Minimize,
                t("titlebar.minimize"),
            ),
            (
                zoom_color,
                "+",
                UiAction::ToggleMaximize,
                t("titlebar.maximize"),
            ),
        ];

        let center_y = ui.available_rect_before_wrap().center().y;

        for (i, (color, icon, action, tooltip)) in buttons.iter().enumerate() {
            let cx = start_x + btn_radius + (i as f32) * spacing;
            let center = egui::pos2(cx, center_y);
            let response = ui.allocate_rect(
                egui::Rect::from_center_size(
                    center,
                    egui::vec2(btn_radius * 2.0, btn_radius * 2.0),
                ),
                egui::Sense::click(),
            );
            ui.painter().circle_filled(center, btn_radius, *color);
            // Show icon on hover.
            if response.hovered() {
                ui.painter().text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    icon,
                    egui::FontId::proportional(styles.typography.caption1),
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 200),
                );
            }
            if response.on_hover_text(*tooltip).clicked() {
                actions.push(action.clone());
            }
        }
    }

    fn draw_drag_area(&self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        // Drag area: fills remaining left space.
        let drag_w = ui.available_width().max(0.0);
        if drag_w > 0.0 {
            let (_, drag_resp) = ui.allocate_exact_size(
                egui::vec2(drag_w, ui.available_height()),
                egui::Sense::click_and_drag(),
            );
            if drag_resp.drag_started() {
                actions.push(UiAction::StartWindowDrag);
            }
            if drag_resp.double_clicked() {
                actions.push(UiAction::ToggleMaximize);
            }
        }
    }

    fn draw_pinned_tabs(
        &self,
        ui: &mut egui::Ui,
        ui_state: &mut UiState,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
    ) {
        if ui_state.pinned_tabs.is_empty() {
            return;
        }

        // Pinned tabs (rendered right-to-left, so they appear left of split buttons).
        ui.add_space(styles.spacing.medium);
        let pinned_tabs: Vec<u32> = ui_state.pinned_tabs.clone();
        let active_tab_id = ui_state.tabs.get(ui_state.active_tab).map(|t| t.id());

        // Render in reverse so left-most pinned tab appears first
        // (right-to-left layout).
        for &sid in pinned_tabs.iter().rev() {
            // Look up title from unified tabs.
            let entry = ui_state.tabs.iter().find(|t| t.id() == sid);
            let is_webview = entry.is_some_and(|t| t.is_webview());
            let title: String = entry
                .map(|t| t.title().to_string())
                .unwrap_or_else(|| "?".into());
            // Truncate to first 6 chars for a compact chip.
            let label: String = title.chars().take(6).collect();
            let is_active = if is_webview {
                ui_state.active_webview_id == Some(sid)
            } else {
                active_tab_id == Some(sid)
            };

            egui::Frame::new()
                .fill(colors.bg_base)
                .inner_margin(egui::Margin::same(styles.spacing.medium as i8))
                .show(ui, |ui| {
                    ui.set_height(ui.available_height());

                    let label_resp = ui.add(egui::Label::new(
                        egui::RichText::new(&label)
                            .color(if is_active {
                                colors.primary
                            } else {
                                colors.text_title
                            })
                            .size(styles.typography.title5),
                    ));
                    label_resp.clone().on_hover_text(&title);
                    if label_resp.clicked() {
                        if is_webview {
                            actions.push(UiAction::ActivateWebviewPane(sid));
                        } else {
                            actions.push(UiAction::SwitchTab(
                                ui_state
                                    .tabs
                                    .iter()
                                    .position(|t| t.id() == sid)
                                    .unwrap_or(0),
                            ));
                        }
                    }

                    // × unpin button — always allocated to keep chip size stable,
                    // but only visible when the chip is hovered.
                    let chip_hovered = ui.ui_contains_pointer();
                    let unpin_btn = ui.add_visible(
                        chip_hovered,
                        egui::Button::new(
                            egui::RichText::new("×")
                                .color(colors.text_caption)
                                .size(styles.typography.caption1),
                        )
                        .corner_radius(egui::CornerRadius::from(styles.spacing.small)),
                    );
                    if chip_hovered && unpin_btn.on_hover_text(t("tabs.unpin_short")).clicked() {
                        actions.push(UiAction::UnpinTab(sid));
                    }
                });

            ui.add(
                egui::Separator::default()
                    .vertical()
                    .shrink(styles.spacing.small * 2.0),
            );
        }
        ui.add_space(styles.spacing.medium);
    }

    fn draw_profile_selector(
        &self,
        ui: &mut egui::Ui,
        ui_state: &mut UiState,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
    ) {
        if ui_state.profile_list.is_empty() {
            return;
        }

        let active_name = ui_state
            .profile_list
            .iter()
            .find(|(id, _)| *id == ui_state.active_profile_id)
            .map(|(_, name)| name.as_str())
            .unwrap_or("Profile");

        let mut selected_id = ui_state.active_profile_id.clone();

        // Temporarily reduce button_padding so the ComboBox button
        // stays compact enough to center within the titlebar height.
        let prev_padding = ui.spacing().button_padding;
        ui.spacing_mut().button_padding = egui::vec2(prev_padding.x, 2.0);

        egui::ComboBox::from_id_salt(egui::Id::new("titlebar_profile_selector"))
            .selected_text(
                egui::RichText::new(active_name)
                    .color(colors.text_title)
                    .size(styles.typography.body0),
            )
            .width(styles.typography.body0 * 7.0)
            .height(ui.available_height())
            .show_ui(ui, |ui| {
                // ── Profile entries (switch active profile) ──────────────
                for (id, name) in &ui_state.profile_list {
                    let is_active = *id == ui_state.active_profile_id;
                    let label = if is_active {
                        format!("● {}", name)
                    } else {
                        name.clone()
                    };
                    if ui.selectable_label(is_active, &label).clicked() {
                        selected_id = id.clone();
                    }
                }

                ui.separator();

                // ── "New Window" sub-menu per profile ────────────────────
                ui.label(
                    egui::RichText::new("New Window")
                        .color(colors.text_title)
                        .size(styles.typography.caption1),
                );
                for (id, name) in &ui_state.profile_list {
                    if ui
                        .selectable_label(false, format!("  ↗ {}", name))
                        .clicked()
                    {
                        actions.push(UiAction::NewWindowWithProfile(id.clone()));
                    }
                }
            });

        ui.spacing_mut().button_padding = prev_padding;

        // Emit action if user picked a different profile.
        if selected_id != ui_state.active_profile_id {
            actions.push(UiAction::SetActiveProfile(selected_id));
        }
    }
}
