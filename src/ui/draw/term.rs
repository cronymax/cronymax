//! Terminal pane rendering (wgpu text + cursor quads)
//!
//! Split into two phases:
//! - **prepare** (CPU, business logic) — [`Ui::prepare_terminal_panes`] reads
//!   sessions, config, cursor state and produces a [`TerminalOutput`].
//! - **render** (GPU, business-free) — driven by
//!   [`TerminalRenderer::render_prepared`] inside the submit closure.

use crate::renderer::atlas::{CellSize, TerminalOutput};
use crate::renderer::cursor::{CursorRect, CursorShape};
use crate::renderer::text;
use crate::renderer::viewport::Viewport;
use crate::ui::Ui;
use crate::ui::model::AppCtx;
use crate::ui::tiles;

impl Ui {
    /// Prepare terminal panes for the current frame.
    ///
    /// Returns a [`TerminalOutput`] ready to be passed to
    /// [`ViewMut::run`](crate::ui::ViewMut::run).
    pub(crate) fn prepare_terminal_panes(
        &mut self,
        ctx: &mut AppCtx,
        scale: f32,
    ) -> TerminalOutput {
        let colors = ctx.config.resolve_colors();
        let text_color = glyphon::Color::rgb(
            colors.text_caption.r(),
            colors.text_caption.g(),
            colors.text_caption.b(),
        );

        let terminal_rects: Vec<&tiles::TileRect> = self
            .tile_rects
            .iter()
            .filter(|tr| matches!(tr, tiles::TileRect::Terminal { .. }))
            .collect();

        if terminal_rects.is_empty() {
            return TerminalOutput {
                quads: Vec::new(),
                text_buffers: Vec::new(),
                text_color,
            };
        }

        let mut all_quads = Vec::new();

        let effective_rects: Vec<(u32, Viewport)> = terminal_rects
            .iter()
            .filter_map(|tr| match tr {
                tiles::TileRect::Terminal { session_id, rect } => {
                    let pad = ctx.config.styles.spacing.medium * scale;
                    let x = rect.left() * scale + pad;
                    let y = rect.top() * scale + pad;
                    let width = (rect.width() * scale - 2.0 * pad).max(0.0);
                    let height = (rect.height() * scale - 2.0 * pad).max(0.0);
                    Some((
                        *session_id,
                        Viewport {
                            x,
                            y,
                            width,
                            height,
                        },
                    ))
                }
                _ => None,
            })
            .collect();

        let mut all_text_buffers: Vec<(_, Viewport)> = Vec::new();
        let mut scratch = std::mem::take(&mut self.frame.terminal.text_scratch);

        let phys_cell_w = self.frame.terminal.cell_size.width * scale;
        let phys_cell_h = self.frame.terminal.cell_size.height * scale;
        let phys_font_size = ctx.config.font.size * scale;

        for &(sid, vp) in &effective_rects {
            let session = match ctx.sessions.get(&sid) {
                Some(s) => s,
                None => continue,
            };

            let cols = (vp.width / phys_cell_w).floor().max(1.0) as usize;
            let rows = (vp.height / phys_cell_h).floor().max(1.0) as usize;

            let term = session.state.term();
            let display_offset = session.state.display_offset();
            let cursor_point = term.grid().cursor.point;
            let cursor_col = cursor_point.column.0.min(cols.saturating_sub(1));
            let cursor_visual_row = cursor_point.line.0 as usize + display_offset;
            let phys_cell = CellSize {
                width: phys_cell_w,
                height: phys_cell_h,
            };
            if ctx.cursor_visible && cursor_visual_row < rows {
                let cursor_rect = CursorRect::new(
                    cursor_col,
                    cursor_visual_row,
                    &phys_cell,
                    CursorShape::from_str(&ctx.config.terminal.cursor_style),
                    colors.primary.to_normalized_gamma_f32(),
                    vp.x,
                    vp.y,
                );
                all_quads.push(crate::renderer::quad::Quad {
                    x: cursor_rect.x,
                    y: cursor_rect.y,
                    width: cursor_rect.width,
                    height: cursor_rect.height,
                    color: cursor_rect.color,
                });
            }

            // ── Selection highlight quads ────────────────────────────
            if let Some(sel) = &self.terminal_selection
                && sel.session_id == sid {
                    let (sc, sr, ec, er) = sel.normalized();
                    // selection_bg = "#264f78" → (38, 79, 120) with alpha
                    let sel_color: [f32; 4] = [
                        38.0 / 255.0,
                        79.0 / 255.0,
                        120.0 / 255.0,
                        0.6,
                    ];
                    for row in sr..=er {
                        if row >= rows {
                            continue;
                        }
                        let col_start = if row == sr { sc } else { 0 };
                        let col_end = if row == er { ec } else { cols.saturating_sub(1) };
                        let x = vp.x + col_start as f32 * phys_cell_w;
                        let y = vp.y + row as f32 * phys_cell_h;
                        let w = (col_end - col_start + 1) as f32 * phys_cell_w;
                        all_quads.push(crate::renderer::quad::Quad {
                            x,
                            y,
                            width: w,
                            height: phys_cell_h,
                            color: sel_color,
                        });
                    }
                }

            let history = session.state.history_size();
            if history > 0 {
                let total = history + rows;
                let thumb_frac = rows as f32 / total as f32;
                let thumb_h = (vp.height * thumb_frac).max(10.0 * scale);
                let scroll_frac = display_offset as f32 / history as f32;
                let thumb_y = vp.y + (vp.height - thumb_h) * (1.0 - scroll_frac);
                let bar_width = 4.0 * scale;
                all_quads.push(crate::renderer::quad::Quad {
                    x: vp.x + vp.width - bar_width,
                    y: thumb_y,
                    width: bar_width,
                    height: thumb_h,
                    color: [1.0, 1.0, 1.0, 0.3],
                });
            }

            let buffer = text::build_terminal_buffer_reuse(
                &mut self.frame.terminal.font_system,
                session.state.term(),
                &text::TerminalFontParams {
                    cols,
                    rows,
                    font_size: phys_font_size,
                    line_height: phys_font_size * ctx.config.font.line_height,
                    font_family: &ctx.config.font.family,
                },
                &mut scratch,
            );
            all_text_buffers.push((buffer, vp));
        }
        self.frame.terminal.text_scratch = scratch;

        // Link underline quad (when hovering a link with Cmd/Ctrl).
        if let Some(link) = &self.hovered_link {
            let cell = &self.frame.terminal.cell_size;
            let underline_height = 1.0_f32.max(cell.height * 0.05);
            let x = self.viewport.x + (link.start_col as f32) * cell.width;
            let y = self.viewport.y + (link.row as f32 + 1.0) * cell.height - underline_height;
            let width = ((link.end_col - link.start_col) as f32) * cell.width;
            all_quads.push(crate::renderer::quad::Quad {
                x,
                y,
                width,
                height: underline_height,
                color: colors.text_caption.to_normalized_gamma_f32(),
            });
        }

        TerminalOutput {
            quads: all_quads,
            text_buffers: all_text_buffers,
            text_color,
        }
    }
}
