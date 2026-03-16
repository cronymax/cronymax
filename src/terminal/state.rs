#![allow(dead_code)]
//! Terminal state wrapper around alacritty_terminal::Term.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{self, Config as TermConfig, Term};
use alacritty_terminal::vte::ansi;

/// Simple event proxy that logs events but otherwise discards them.
#[derive(Clone)]
pub struct EventProxy;

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match &event {
            Event::Title(t) => log::debug!("Terminal title: {}", t),
            Event::Bell => log::trace!("Terminal bell"),
            _ => log::trace!("Terminal event: {:?}", event),
        }
    }
}

/// Sizing info for the terminal grid; implements alacritty_terminal::grid::Dimensions.
pub struct TermSize {
    pub cols: usize,
    pub rows: usize,
}

impl TermSize {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self { cols, rows }
    }
}

impl Dimensions for TermSize {
    fn columns(&self) -> usize {
        self.cols
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn total_lines(&self) -> usize {
        self.rows
    }
}

/// Wrapper around alacritty_terminal::Term providing a simplified API.
pub struct TermState {
    pub term: Term<EventProxy>,
    processor: ansi::Processor,
}

impl TermState {
    /// Create a new terminal state with the given grid size and scrollback history.
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Self {
        let config = TermConfig {
            scrolling_history: scrollback,
            ..Default::default()
        };
        let size = TermSize::new(cols, rows);
        let term = Term::new(config, &size, EventProxy);
        let processor = ansi::Processor::new();

        Self { term, processor }
    }

    /// Feed raw PTY bytes into the terminal state machine.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    /// Resize the terminal grid.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let size = TermSize::new(cols, rows);
        self.term.resize(size);
    }

    /// Get the renderable content for the current terminal state.
    pub fn renderable_content(&self) -> term::RenderableContent<'_> {
        self.term.renderable_content()
    }

    /// Get the current window title (set via OSC sequences).
    pub fn title(&self) -> Option<String> {
        // Term exposes title as a private field; we track it via EventProxy instead.
        // For now, return None; the EventProxy can be extended to capture it.
        None
    }

    /// Get a reference to the inner Term for direct grid access.
    pub fn term(&self) -> &Term<EventProxy> {
        &self.term
    }

    /// Scroll the display up by the given number of lines.
    pub fn scroll_up(&mut self, lines: i32) {
        self.term
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(lines));
    }

    /// Scroll the display down by the given number of lines.
    pub fn scroll_down(&mut self, lines: i32) {
        self.term
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(-lines));
    }

    /// Scroll up by one page.
    pub fn scroll_page_up(&mut self) {
        let page = self.term.screen_lines() as i32;
        self.scroll_up(page);
    }

    /// Scroll down by one page.
    pub fn scroll_page_down(&mut self) {
        let page = self.term.screen_lines() as i32;
        self.scroll_down(page);
    }

    /// Snap back to the bottom of the scrollback.
    pub fn scroll_to_bottom(&mut self) {
        self.term
            .scroll_display(alacritty_terminal::grid::Scroll::Bottom);
    }

    /// Return the absolute row of the cursor measured from the top of the
    /// scrollback + screen combined buffer.
    ///
    /// `abs_row = history_size + cursor_viewport_line`, where
    /// `cursor_viewport_line` is 0-indexed from the top of the current
    /// viewport (independent of scroll position).
    pub fn abs_cursor_row(&self) -> i32 {
        let history = (self.term.total_lines() - self.term.screen_lines()) as i32;
        let cursor_line = self.term.grid().cursor.point.line.0;
        history + cursor_line
    }

    /// Return the current display offset (number of lines scrolled above the
    /// bottom of the scrollback buffer).
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Number of scrollback history lines currently held.
    pub fn history_size(&self) -> usize {
        self.term
            .total_lines()
            .saturating_sub(self.term.screen_lines())
    }

    /// Height of the visible viewport in rows.
    pub fn viewport_rows(&self) -> usize {
        self.term.screen_lines()
    }

    /// Capture text from an absolute row range as a plain-text string.
    ///
    /// `abs_start`/`abs_end` use the same coordinate system as
    /// `CommandBlock::abs_row`: `abs_row = history_size + viewport_line`.
    /// Rows that have been evicted from the scrollback buffer are silently
    /// skipped.
    pub fn capture_text(&self, abs_start: i32, abs_end: i32) -> String {
        let grid = self.term.grid();
        let history = self.history_size() as i32;
        let cols = self.term.columns();
        let screen_lines = self.term.screen_lines() as i32;
        // Valid grid line range: -history .. screen_lines
        let min_line = -history;
        let max_line = screen_lines;
        let mut result = String::new();
        for abs_row in abs_start..abs_end {
            let viewport_line = abs_row - history;
            if viewport_line < min_line || viewport_line >= max_line {
                continue;
            }
            let line = alacritty_terminal::index::Line(viewport_line);
            let mut line_text = String::with_capacity(cols);
            for col_idx in 0..cols {
                let col = alacritty_terminal::index::Column(col_idx);
                let cell = &grid[line][col];
                let c = cell.c;
                if c.is_control() || c == '\0' {
                    line_text.push(' ');
                } else {
                    line_text.push(c);
                }
            }
            let trimmed = line_text.trim_end();
            if !trimmed.is_empty() || !result.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(trimmed);
            }
        }
        result
    }
}
