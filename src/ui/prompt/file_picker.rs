//! File picker widget, state update, and keyboard handling.

use crate::{
    renderer::terminal::SessionId,
    ui::{
        file_picker::FilePickerState,
        i18n::{t, t_fmt},
        prompt::PromptState,
        widget::Widget,
    },
};

// ─── FilePickerWidget ────────────────────────────────────────────────────────

/// Floating file picker popup rendered on a foreground layer above the input.
pub(super) struct FilePickerWidget<'a> {
    pub state: &'a mut FilePickerState,
    pub anchor_rect: egui::Rect,
}

impl Widget<egui::Ui> for FilePickerWidget<'_> {
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <egui::Ui as crate::ui::widget::Painter>::Ref<'a>,
        #[allow(unused)] mut ctx: crate::ui::widget::Context<'a>,
    ) {
        let styles = ctx.styles;
        let match_count = self.state.matches_count();
        if match_count == 0 {
            return;
        }

        let n_visible = match_count.min(FilePickerState::MAX_VISIBLE_ROWS);
        let row_h = styles.suggestion_row_height();
        let popup_h = (n_visible as f32) * row_h
            + styles.spacing.medium * 2.0
            + styles.typography.line_height;
        let popup_w = self.anchor_rect.width();

        let popup_rect = egui::Rect::from_min_max(
            egui::pos2(self.anchor_rect.min.x, self.anchor_rect.min.y - popup_h),
            egui::pos2(self.anchor_rect.min.x + popup_w, self.anchor_rect.min.y),
        );

        let layer_id =
            egui::LayerId::new(egui::Order::Foreground, egui::Id::new("file_picker_popup"));
        let mut popup_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(popup_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT))
                .layer_id(layer_id),
        );
        popup_ui.set_clip_rect(popup_rect);

        let query = self.state.query.clone();

        // Header
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(t("picker.header")).small());
            if !query.is_empty() {
                ui.label(egui::RichText::new(t_fmt("picker.query_prefix", &query)).small());
            }
        });

        ui.separator();
        ui.add_space(styles.spacing.small);

        // Rows
        egui::ScrollArea::vertical()
            .max_height(n_visible as f32 * row_h)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for i in 0..match_count {
                    ctx.bind::<egui::Ui>(ui).add(FilePickerRow {
                        state: self.state,
                        index: i,
                    });
                }
            });
    }
}

// ─── FilePickerRow ───────────────────────────────────────────────────────────

struct FilePickerRow<'a> {
    state: &'a mut FilePickerState,
    index: usize,
}

impl Widget<egui::Ui> for FilePickerRow<'_> {
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <egui::Ui as crate::ui::widget::Painter>::Ref<'a>,
        #[allow(unused)] ctx: crate::ui::widget::Context<'a>,
    ) {
        let styles = ctx.styles;
        let colors = &ctx.colors;
        let is_selected = self.index == self.state.selected;
        let path = self.state.current_matches()[self.index].path.clone();

        let item_id = egui::Id::new("file_picker_suggestion_item").with(self.index);
        let hover_t = ui
            .ctx()
            .animate_bool_with_time(item_id.with("hover"), is_selected, 0.15);

        let inner_response = egui::Frame::new()
            .fill(colors.fill_active.gamma_multiply(hover_t))
            .corner_radius(styles.radii.sm - 1.0)
            .inner_margin(egui::Margin::symmetric(
                styles.spacing.medium as i8,
                styles.spacing.small as i8,
            ))
            .show(ui, |ui| {
                let resp = ui.add(
                    egui::Label::new(egui::RichText::new(&path).small())
                        .sense(egui::Sense::click()),
                );
                if resp.clicked() {
                    self.state.picked_path = Some(path.clone());
                }
            });

        if is_selected {
            inner_response
                .response
                .scroll_to_me(Some(egui::Align::Center));
        }
    }
}

// ─── PromptState: file picker state management ──────────────────────────────

impl PromptState {
    /// Extract the `#query` fragment from the current text.
    pub(super) fn extract_hash_query(&self) -> Option<(usize, &str)> {
        let hash_pos = self.text.rfind('#')?;
        let after_hash = &self.text[hash_pos + 1..];
        Some((hash_pos, after_hash))
    }

    /// Update file picker state based on current text (activate/deactivate/query).
    /// Does NOT render the popup — use [`FilePickerWidget`] for that.
    pub(super) fn update_file_picker_state(
        &mut self,
        ui: &mut egui::Ui,
        te_id: egui::Id,
        display_text_start: usize,
    ) {
        let cursor_at_end = if let Some(te_state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
            te_state
                .cursor
                .char_range()
                .map(|r| {
                    let cursor_in_display = r.primary.index;
                    let cursor_in_full = display_text_start + cursor_in_display;
                    cursor_in_full >= self.text.rfind('#').unwrap_or(usize::MAX)
                        && cursor_in_full >= self.text.len().saturating_sub(0)
                })
                .unwrap_or(true)
        } else {
            true
        };

        let hash_info = self
            .extract_hash_query()
            .map(|(pos, q)| (pos, q.to_string()));

        if let Some((_hash_pos, query)) = hash_info {
            if !cursor_at_end {
                if self.file_picker.active {
                    self.file_picker.deactivate();
                }
            } else {
                if !self.file_picker.active {
                    let cwd = self.cwd.as_deref().unwrap_or(".");
                    self.file_picker.activate(std::path::Path::new(cwd));
                }
                if self.file_picker.query != query {
                    self.file_picker.set_query(&query);
                }
                if !query.is_empty() && self.file_picker.matches_count() == 0 {
                    self.file_picker.deactivate();
                }
            }
        } else if self.file_picker.active {
            self.file_picker.deactivate();
        }
    }

    /// Handle keyboard events when the file picker is active.
    pub(super) fn handle_file_picker_keys(
        &mut self,
        ui: &mut egui::Ui,
        te_response_id: egui::Id,
        focused: bool,
        _sid: SessionId,
        _submitted: &mut Vec<(SessionId, String)>,
    ) {
        if !focused {
            return;
        }

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.file_picker.deactivate();
            if let Some((hash_pos, _)) = self.extract_hash_query() {
                self.text.truncate(hash_pos);
            }
            return;
        }

        if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            self.file_picker.select_prev();
            ui.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::ArrowUp,
                            pressed: true,
                            ..
                        }
                    )
                });
            });
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.file_picker.select_next();
            ui.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::ArrowDown,
                            pressed: true,
                            ..
                        }
                    )
                });
            });
        }

        let tab_pressed: bool = ui.ctx().data(|d| {
            d.get_temp(egui::Id::new("__global_tab_pressed"))
                .unwrap_or(false)
        });
        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter_pressed || tab_pressed {
            if let Some(path) = self.file_picker.selected_path().map(|s| s.to_string()) {
                self.insert_file_pick(&path);
            }
            if enter_pressed {
                ui.input_mut(|i| {
                    i.events.retain(|e| {
                        !matches!(
                            e,
                            egui::Event::Key {
                                key: egui::Key::Enter,
                                pressed: true,
                                ..
                            }
                        )
                    });
                });
            }
            ui.ctx().memory_mut(|mem| mem.request_focus(te_response_id));
            ui.ctx().request_repaint();
        }
    }

    /// Replace `#query` in self.text with the selected file path and close picker.
    pub(super) fn insert_file_pick(&mut self, path: &str) {
        if let Some((hash_pos, _)) = self.extract_hash_query() {
            self.text.truncate(hash_pos);
            self.text.push_str(path);
            self.text.push(' ');
        }
        self.file_picker.deactivate();
    }
}
