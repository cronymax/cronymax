//! Terminal pane rendering (wgpu text + cursor quads)

use crate::app::*;

pub(super) fn render_terminal_panes(
    state: &mut AppState,
    pane_rects: &[tiles::TileRect],
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    scale: f32,
    sw: f32,
    sh: f32,
) {
    let colors = state.config.resolve_colors();
    // ── 4. Render terminal panes using tile_rects ────────────
    // These render ON TOP of the egui frame so the terminal text
    // and cursor are visible through the panel background.
    // Only Classic-mode panes push Terminal TileRects; Editor-mode
    // panes render all their content through egui blocks/overlays.
    // When no terminal rects exist (all panes are Editor mode),
    // skip the wgpu terminal passes entirely.
    let terminal_rects: Vec<&tiles::TileRect> = pane_rects
        .iter()
        .filter(|tr| matches!(tr, tiles::TileRect::Terminal { .. }))
        .collect();
    if !terminal_rects.is_empty() {
        let mut all_quads = Vec::new();

        let effective_rects: Vec<(u32, f32, f32, f32, f32)> = terminal_rects
            .iter()
            .filter_map(|tr| match tr {
                tiles::TileRect::Terminal { session_id, rect } => {
                    // Inset the pane rect by terminal_padding (logical → physical).
                    let pad = state.config.styles.spacing.medium * scale;
                    let px = rect.left() * scale + pad;
                    let py = rect.top() * scale + pad;
                    let pw = (rect.width() * scale - 2.0 * pad).max(0.0);
                    let ph = (rect.height() * scale - 2.0 * pad).max(0.0);
                    Some((*session_id, px, py, pw, ph))
                }
                _ => None,
            })
            .collect();

        let mut all_text_buffers: Vec<(_, f32, f32, f32, f32)> = Vec::new();
        let mut scratch = std::mem::take(&mut state.text_scratch);

        // Scale cell size to physical pixels to match the viewport coords.
        let phys_cell_w = state.renderer.cell_size.width * scale;
        let phys_cell_h = state.renderer.cell_size.height * scale;
        let phys_font_size = state.config.font.size * scale;

        for &(sid, vp_x, vp_y, vp_w, vp_h) in &effective_rects {
            let session = match state.sessions.get(&sid) {
                Some(s) => s,
                None => continue,
            };

            // Grid dimensions from pane physical size.
            let cols = (vp_w / phys_cell_w).floor().max(1.0) as usize;
            let rows = (vp_h / phys_cell_h).floor().max(1.0) as usize;

            // Cursor quad for this pane.
            let term = session.state.term();
            let display_offset = session.state.display_offset();
            let cursor_point = term.grid().cursor.point;
            let cursor_col = cursor_point.column.0.min(cols.saturating_sub(1));
            // When scrolled, offset cursor row by display_offset so it
            // stays visually aligned with the shifted text content.
            let cursor_visual_row = cursor_point.line.0 as usize + display_offset;
            let phys_cell = crate::renderer::atlas::CellSize {
                width: phys_cell_w,
                height: phys_cell_h,
            };
            // Only show cursor if it falls within the visible viewport.
            if state.cursor_visible && cursor_visual_row < rows {
                let cursor_rect = CursorRect::new(
                    cursor_col,
                    cursor_visual_row,
                    &phys_cell,
                    CursorShape::from_str(&state.config.terminal.cursor_style),
                    colors.primary.to_normalized_gamma_f32(),
                    vp_x,
                    vp_y,
                );
                all_quads.push(crate::renderer::quad::Quad {
                    x: cursor_rect.x,
                    y: cursor_rect.y,
                    width: cursor_rect.width,
                    height: cursor_rect.height,
                    color: cursor_rect.color,
                });
            }

            // Scrollbar indicator (thin bar on the right edge).
            let history = session.state.history_size();
            if history > 0 {
                let total = history + rows;
                let thumb_frac = rows as f32 / total as f32;
                let thumb_h = (vp_h * thumb_frac).max(10.0 * scale);
                let scroll_frac = display_offset as f32 / history as f32;
                // thumb_y: scrolled-to-bottom → thumb at bottom; scrolled-to-top → thumb at top
                let thumb_y = vp_y + (vp_h - thumb_h) * (1.0 - scroll_frac);
                let bar_width = 4.0 * scale;
                all_quads.push(crate::renderer::quad::Quad {
                    x: vp_x + vp_w - bar_width,
                    y: thumb_y,
                    width: bar_width,
                    height: thumb_h,
                    color: [1.0, 1.0, 1.0, 0.3],
                });
            }

            // Terminal text buffer for this pane.
            let buffer = text::build_terminal_buffer_reuse(
                &mut state.renderer.font_system,
                session.state.term(),
                &text::TerminalFontParams {
                    cols,
                    rows,
                    font_size: phys_font_size,
                    line_height: phys_font_size * state.config.font.line_height,
                    font_family: &state.config.font.family,
                },
                &mut scratch,
            );
            all_text_buffers.push((buffer, vp_x, vp_y, vp_w, vp_h));
        }
        state.text_scratch = scratch;

        // Build text areas from collected buffers (buffers live long enough).
        let all_text_areas: Vec<_> = all_text_buffers
            .iter()
            .map(|(buf, vp_x, vp_y, vp_w, vp_h)| {
                text::terminal_text_area(
                    buf,
                    *vp_x,
                    *vp_y,
                    *vp_w as i32,
                    *vp_h as i32,
                    glyphon::Color::rgb(
                        colors.text_caption.r(),
                        colors.text_caption.g(),
                        colors.text_caption.b(),
                    ),
                )
            })
            .collect();

        // Link underline quad (when hovering a link with Cmd/Ctrl).
        if let Some(ref link) = state.hovered_link {
            let cell = &state.renderer.cell_size;
            let underline_height = 1.0_f32.max(cell.height * 0.05);
            let x = state.viewport.x + (link.start_col as f32) * cell.width;
            let y = state.viewport.y + (link.row as f32 + 1.0) * cell.height - underline_height;
            let width = ((link.end_col - link.start_col) as f32) * cell.width;
            all_quads.push(crate::renderer::quad::Quad {
                x,
                y,
                width,
                height: underline_height,
                color: colors.text_caption.to_normalized_gamma_f32(),
            });
        }

        // Render cursor + link quads.
        state
            .renderer
            .quad_renderer
            .prepare(&state.gpu.queue, &all_quads, sw, sh);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("quad-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            state.renderer.quad_renderer.render(&mut pass);
        }

        // Render terminal text (all panes at once).
        let win_size = state.window.inner_size();
        state
            .renderer
            .update_viewport(&state.gpu.queue, win_size.width, win_size.height);

        state
            .renderer
            .text_renderer
            .prepare(
                &state.gpu.device,
                &state.gpu.queue,
                &mut state.renderer.font_system,
                &mut state.renderer.atlas,
                &state.renderer.viewport,
                all_text_areas,
                &mut state.renderer.swash_cache,
            )
            .expect("Failed to prepare text rendering");

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            state
                .renderer
                .text_renderer
                .render(&state.renderer.atlas, &state.renderer.viewport, &mut pass)
                .expect("Failed to render text");
        }
    } // end if !terminal_rects.is_empty()
}
