//! Terminal grid → glyph rendering.
//!
//! Iterates alacritty_terminal's renderable content and builds glyphon TextArea
//! buffers for the GPU text pipeline.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags as CellFlags;
use glyphon::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, TextArea, TextBounds};

use crate::renderer::terminal::state::EventProxy;

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
    use unicode_width::UnicodeWidthChar;

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

    // Account for scrollback: shift line indices by -display_offset so we
    // read the correct slice of the grid when scrolled up.
    let display_offset = term.grid().display_offset() as i32;

    // Track which characters are wide (CJK) so we can use a different
    // font family for them. cosmic-text produces w=inf for CJK glyphs
    // when using Family::Monospace, so we fall back to Family::SansSerif
    // for wide characters.
    let mut is_wide_char: Vec<bool> = Vec::with_capacity(cols * rows);

    for row_idx in 0..rows {
        let line = alacritty_terminal::index::Line(row_idx as i32 - display_offset);
        for col_idx in 0..cols {
            let col = alacritty_terminal::index::Column(col_idx);
            let cell = &grid[line][col];
            let flags = cell.flags;

            // Skip spacer cells that follow a wide (CJK) character.
            if flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }

            // A leading wide-char spacer occupies the last column when
            // the actual wide character wrapped to the next row.
            if flags.contains(CellFlags::LEADING_WIDE_CHAR_SPACER) {
                scratch.push(' ');
                is_wide_char.push(false);
                continue;
            }

            let c = cell.c;
            if c.is_control() || c == '\0' {
                scratch.push(' ');
                is_wide_char.push(false);
            } else {
                scratch.push(c);
                // Use the cell flag or Unicode width to detect wide chars.
                let wide = flags.contains(CellFlags::WIDE_CHAR)
                    || c.width().unwrap_or(0) > 1;
                is_wide_char.push(wide);
            }
        }
        // Trim trailing spaces for the line
        let trimmed_len = scratch.len() - scratch.chars().rev().take_while(|c| *c == ' ').count();
        let chars_removed = scratch[trimmed_len..].chars().count();
        scratch.truncate(trimmed_len);
        is_wide_char.truncate(is_wide_char.len() - chars_removed);
        if row_idx < rows - 1 {
            scratch.push('\n');
            is_wide_char.push(false);
        }
    }

    // Check if any wide chars exist; if not, use the fast path.
    let has_wide = is_wide_char.iter().any(|w| *w);

    if !has_wide {
        // Fast path: all ASCII/narrow — use set_text with monospace font.
        buffer.set_text(
            font_system,
            scratch,
            &Attrs::new().family(family_val),
            Shaping::Advanced,
            None,
        );
    } else {
        // Slow path: split into spans with different font families.
        // CJK/wide chars use SansSerif (which has correct glyph metrics),
        // narrow chars use the configured monospace font.
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        let mut span_start_byte = 0;
        let mut current_is_wide = false;

        let make_attrs = |wide: bool| -> Attrs {
            if wide {
                Attrs::new().family(Family::SansSerif)
            } else {
                Attrs::new().family(family_val)
            }
        };

        for (char_idx, (byte_pos, _c)) in scratch.char_indices().enumerate() {
            let w = is_wide_char.get(char_idx).copied().unwrap_or(false);
            if char_idx > 0 && w != current_is_wide {
                spans.push((
                    &scratch[span_start_byte..byte_pos],
                    make_attrs(current_is_wide),
                ));
                span_start_byte = byte_pos;
            }
            current_is_wide = w;
        }
        if span_start_byte < scratch.len() {
            spans.push((
                &scratch[span_start_byte..],
                make_attrs(current_is_wide),
            ));
        }

        let default_attrs = Attrs::new().family(family_val);
        buffer.set_rich_text(
            font_system,
            spans,
            &default_attrs,
            Shaping::Advanced,
            None,
        );
    }

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
