//! Drag-and-drop handling and block overlay rendering.

use super::*;

/// Format a Unix timestamp (millis) into a short time string (HH:MM).
pub(super) fn format_timestamp(timestamp_ms: i64) -> String {
    let secs = timestamp_ms / 1000;
    let hours = (secs / 3600) % 24;
    let mins = (secs / 60) % 60;
    format!("{:02}:{:02}", hours, mins)
}

/// Handle drag-and-drop interaction for a pane area.
/// Shows a directional preview highlight when a tab is dragged over the zone,
/// and emits `UiAction::DockTab` on drop.
pub(super) fn handle_pane_dnd(
    ui: &mut egui::Ui,
    sid: SessionId,
    rect: egui::Rect,
    response: &egui::Response,
    actions: &mut Vec<UiAction>,
) {
    let is_dragging = egui::DragAndDrop::has_payload_of_type::<TabDragPayload>(ui.ctx());
    if is_dragging
        && response.contains_pointer()
        && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
    {
        let dir = dock_direction(pointer, rect);
        let preview_rect = dock_preview_rect(rect, dir);
        // Paint on a foreground layer so the split preview covers the
        // prompt editor and any other pane content.
        ui.ctx()
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("dnd_overlay").with(sid),
            ))
            .rect_filled(
                preview_rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(100, 140, 255, 40),
            );
    }
    if let Some(payload) = response.dnd_release_payload::<TabDragPayload>()
        && payload.session_id != sid
    {
        let pointer = ui
            .input(|i| i.pointer.interact_pos())
            .unwrap_or(rect.center());
        let dir = dock_direction(pointer, rect);
        actions.push(crate::ui::UiAction::DockTab {
            source: payload.session_id,
            target: sid,
            direction: dir,
        });
    }
}

// ─── Dock Direction Helpers ──────────────────────────────────────────────────

/// Determine which dock zone the pointer is in, based on its position within a pane rect.
/// Divides the rect into edge zones (25% from each edge) and a center zone.
fn dock_direction(pointer: egui::Pos2, rect: egui::Rect) -> DockDirection {
    let rel_x = (pointer.x - rect.min.x) / rect.width();
    let rel_y = (pointer.y - rect.min.y) / rect.height();

    // Edge threshold (25% from each edge).
    const T: f32 = 0.25;

    if rel_y < T {
        DockDirection::Top
    } else if rel_y > (1.0 - T) {
        DockDirection::Bottom
    } else if rel_x < T {
        DockDirection::Left
    } else {
        DockDirection::Right
    }
}

/// Return the highlighted preview rect for a given dock direction.
fn dock_preview_rect(rect: egui::Rect, dir: DockDirection) -> egui::Rect {
    let cx = rect.center().x;
    let cy = rect.center().y;
    match dir {
        DockDirection::Left => egui::Rect::from_min_max(rect.min, egui::pos2(cx, rect.max.y)),
        DockDirection::Right => egui::Rect::from_min_max(egui::pos2(cx, rect.min.y), rect.max),
        DockDirection::Top => egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, cy)),
        DockDirection::Bottom => egui::Rect::from_min_max(egui::pos2(rect.min.x, cy), rect.max),
    }
}
// ─── Editor-Mode Cell Overlay ────────────────────────────────────────────────

/// Draw egui frame borders and filter icons on top of the wgpu terminal
/// content for each command block recorded in editor mode.
///
/// Each block spans from its `abs_row` to the next block's `abs_row` (or the
/// bottom of the viewport for the last block).  Borders are drawn via the
/// painter; the filter icon is an interactive egui widget.
pub(super) fn draw_block_overlays(
    ui: &mut egui::Ui,
    sid: SessionId,
    term_rect: egui::Rect,
    info: &BlockGrid,
    command_blocks: &mut [crate::ui::prompt::CommandBlock],
    styles: &Styles,
) {
    if command_blocks.is_empty() {
        return;
    }

    let h = info.history_size as i32;
    let d = info.display_offset as i32;
    let n = info.screen_lines as i32;
    let ch = info.block_height;

    // Convert abs_row → viewport row (0 = top of visible viewport).
    let to_vp = |abs: i32| -> i32 { abs - h + d };

    // Map each block to its viewport row; keep only those near visible area.
    let vp_rows: Vec<i32> = command_blocks.iter().map(|b| to_vp(b.abs_row)).collect();

    // Iterate over visible blocks.
    for i in 0..command_blocks.len() {
        let row_start = vp_rows[i];
        let row_end = vp_rows.get(i + 1).copied().unwrap_or(n);

        // Skip blocks completely outside the visible area.
        if row_end <= 0 || row_start >= n {
            continue;
        }

        // Pixel bounds — clipped to term_rect.
        let y_top = (term_rect.min.y + row_start.max(0) as f32 * ch).max(term_rect.min.y);
        let y_bottom = (term_rect.min.y + row_end.min(n) as f32 * ch).min(term_rect.max.y);

        if y_bottom <= y_top + 1.0 {
            continue;
        }

        let cell_rect = egui::Rect::from_min_max(
            egui::pos2(term_rect.min.x + 1.0, y_top),
            egui::pos2(term_rect.max.x - 1.0, y_bottom),
        );

        // Header row — only when the block's first row is within view.
        let header_visible = row_start >= 0 && row_start < n;
        let header_rect = egui::Rect::from_min_size(
            egui::pos2(term_rect.min.x + 1.0, y_top),
            egui::vec2(term_rect.width() - 2.0, ch),
        );

        // Hover detection (pointer inside cell rect).
        let is_hovered = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| cell_rect.contains(p));

        let block = &command_blocks[i];
        let filter_open = block.filter_open;

        // ── Draw cell border ──────────────────────────────────────────
        let border_color = if is_hovered {
            ui.visuals().text_color()
        } else {
            let c = ui.visuals().weak_text_color();
            // Subtle separator-colored border always present.
            egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 120)
        };
        ui.painter().rect_stroke(
            cell_rect,
            egui::CornerRadius::same(styles.radii.sm as u8),
            egui::Stroke::new(styles.sizes.border, border_color),
            egui::StrokeKind::Outside,
        );

        // ── Interactive filter affordance (hover or open) ─────────────
        if header_visible && (is_hovered || filter_open) {
            let block_id = i; // stable index
            ui.allocate_new_ui(
                egui::UiBuilder::new()
                    .max_rect(header_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    let block = &mut command_blocks[block_id];

                    // Toggle button: filter icon when closed, × when open.
                    let btn = icons::icon_button(
                        ui,
                        icons::IconButtonCfg {
                            icon: if block.filter_open {
                                Icon::Close
                            } else {
                                Icon::Search
                            },
                            tooltip: if block.filter_open {
                                t("filter.close")
                            } else {
                                t("filter.open")
                            },
                            base_color: ui.visuals().weak_text_color(),
                            hover_color: ui.visuals().text_color(),
                            pixel_size: styles.typography.body0,
                            margin: styles.spacing.small,
                        },
                    );
                    if btn.clicked() {
                        block.filter_open = !block.filter_open;
                        if !block.filter_open {
                            block.filter_text.clear();
                        }
                    }

                    // Filter TextEdit (expands to the left of the icon).
                    if block.filter_open {
                        let te_id = egui::Id::new("cell_filter_te").with(sid).with(block_id);
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut block.filter_text)
                                .id(te_id)
                                .hint_text(t("filter.pattern_hint"))
                                .desired_width(styles.typography.line_height * 6.0)
                                .font(egui::TextStyle::Small),
                        );
                        // Auto-focus the filter input when it first opens.
                        if resp.gained_focus()
                            || (!ui.ctx().wants_keyboard_input() && block.filter_text.is_empty())
                        {
                            resp.request_focus();
                        }
                        // Close on Escape.
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            block.filter_open = false;
                            block.filter_text.clear();
                        }
                    }
                },
            );
        }
    }
}
