//! Cursor rendering (block, underline, beam).
//!
//! Draws the terminal cursor as a colored rectangle at the cursor's grid position.

use crate::renderer::atlas::CellSize;

/// The visual style of the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
}

impl CursorShape {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "underline" => CursorShape::Underline,
            "beam" => CursorShape::Beam,
            _ => CursorShape::Block,
        }
    }
}

/// Describes a cursor to be drawn.
#[derive(Debug, Clone, Copy)]
pub struct CursorRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: [f32; 4],
}

impl CursorRect {
    /// Compute the cursor rectangle from grid position, cell size, and cursor shape.
    pub fn new(
        col: usize,
        row: usize,
        cell: &CellSize,
        shape: CursorShape,
        color: [f32; 4],
        padding_x: f32,
        padding_y: f32,
    ) -> Self {
        let x = padding_x + col as f32 * cell.width;
        let y = padding_y + row as f32 * cell.height;

        let (w, h) = match shape {
            CursorShape::Block => (cell.width, cell.height),
            CursorShape::Underline => (cell.width, 2.0),
            CursorShape::Beam => (2.0, cell.height),
        };

        // For underline, position at the bottom of the cell.
        let y = if shape == CursorShape::Underline {
            y + cell.height - 2.0
        } else {
            y
        };

        Self {
            x,
            y,
            width: w,
            height: h,
            color,
        }
    }
}
