//! Terminal grid → glyph rendering.
//!
//! Iterates alacritty_terminal's renderable content and builds glyphon TextArea
//! buffers for the GPU text pipeline.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Term;
use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, TextArea, TextBounds};

use crate::terminal::state::EventProxy;

/// Terminal font/grid layout parameters for text buffer construction.
pub struct TerminalFontParams<'a> {
    pub cols: usize,
    pub rows: usize,
    pub font_size: f32,
    pub line_height: f32,
    pub font_family: &'a str,
}

/// Build a glyphon Buffer from the terminal's grid content.
/// Reads directly from the Term grid to assemble a text string.
pub fn build_terminal_buffer(
    font_system: &mut FontSystem,
    term: &Term<EventProxy>,
    params: &TerminalFontParams<'_>,
) -> Buffer {
    let mut scratch = String::with_capacity(params.cols * params.rows + params.rows);
    build_terminal_buffer_reuse(font_system, term, params, &mut scratch)
}

/// Build a glyphon Buffer, reusing the provided scratch String to reduce allocations.
pub fn build_terminal_buffer_reuse(
    font_system: &mut FontSystem,
    term: &Term<EventProxy>,
    params: &TerminalFontParams<'_>,
    scratch: &mut String,
) -> Buffer {
    let metrics = Metrics::new(params.font_size, params.line_height);
    let mut buffer = Buffer::new(font_system, metrics);

    // Set buffer size so glyphon knows the layout area.
    // Width = cols * approximate cell width, height = rows * line_height.
    // Use a generous width to avoid wrapping; the TextArea bounds clip the output.
    let buf_width = (params.cols as f32) * params.font_size;
    let buf_height = (params.rows as f32) * params.line_height;
    buffer.set_size(font_system, Some(buf_width), Some(buf_height));

    let family_val = if params.font_family == "monospace" || params.font_family.is_empty() {
        Family::Monospace
    } else {
        Family::Name(params.font_family)
    };

    // Build the text content from the grid by reading cells directly.
    // Reuse the scratch buffer to avoid per-frame allocation.
    scratch.clear();
    let grid = term.grid();

    // Clamp to actual grid dimensions to prevent out-of-bounds access
    // (the viewport may be larger than the terminal grid on the first frame
    // before the PTY resize takes effect).
    let grid_cols = term.columns();
    let grid_rows = term.screen_lines();
    let cols = params.cols.min(grid_cols);
    let rows = params.rows.min(grid_rows);

    for row_idx in 0..rows {
        let line = alacritty_terminal::index::Line(row_idx as i32);
        for col_idx in 0..cols {
            let col = alacritty_terminal::index::Column(col_idx);
            let cell = &grid[line][col];
            let c = cell.c;
            if c.is_control() || c == '\0' {
                scratch.push(' ');
            } else {
                scratch.push(c);
            }
        }
        // Trim trailing spaces for the line
        let trimmed_len = scratch.len() - scratch.chars().rev().take_while(|c| *c == ' ').count();
        scratch.truncate(trimmed_len);
        if row_idx < rows - 1 {
            scratch.push('\n');
        }
    }

    buffer.set_text(
        font_system,
        scratch,
        &Attrs::new().family(family_val),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);

    buffer
}

/// Create a TextArea from a buffer positioned at the terminal viewport origin.
pub fn terminal_text_area<'a>(
    buffer: &'a Buffer,
    left: f32,
    top: f32,
    width: i32,
    height: i32,
    fg_color: glyphon::Color,
) -> TextArea<'a> {
    TextArea {
        buffer,
        left,
        top,
        scale: 1.0,
        bounds: TextBounds {
            left: left as i32,
            top: top as i32,
            right: left as i32 + width,
            bottom: top as i32 + height,
        },
        default_color: fg_color,
        custom_glyphs: &[],
    }
}

/// Convert a hex color string to a glyphon Color.
pub fn hex_to_glyphon_color(hex: &str) -> glyphon::Color {
    let hex = hex.trim_start_matches('#');
    let (r, g, b) = match hex.len() {
        6 => (
            u8::from_str_radix(&hex[0..2], 16).unwrap_or(0),
            u8::from_str_radix(&hex[2..4], 16).unwrap_or(0),
            u8::from_str_radix(&hex[4..6], 16).unwrap_or(0),
        ),
        _ => (192, 192, 192),
    };
    glyphon::Color::rgb(r, g, b)
}
