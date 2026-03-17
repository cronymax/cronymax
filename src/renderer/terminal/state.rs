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

    /// Search all terminal content (scrollback + screen) for case-insensitive
    /// occurrences of `query`. Returns a list of `(grid_line, start_col)`
    /// pairs in top-to-bottom order, where `grid_line` uses alacritty's
    /// coordinate system (negative = scrollback, 0.. = screen).
    pub fn search_text(&self, query: &str) -> Vec<(i32, usize)> {
        if query.is_empty() {
            return Vec::new();
        }
        let grid = self.term.grid();
        let history = self.history_size() as i32;
        let cols = self.term.columns();
        let screen_lines = self.term.screen_lines() as i32;
        let query_lower = query.to_lowercase();
        let mut matches = Vec::new();

        for vp_line in -history..screen_lines {
            let line = alacritty_terminal::index::Line(vp_line);
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
            let line_lower = line_text.to_lowercase();
            let mut start = 0;
            while let Some(pos) = line_lower[start..].find(&query_lower) {
                matches.push((vp_line, start + pos));
                start += pos + 1;
            }
        }
        matches
    }

    /// Scroll the viewport so that the given grid line is visible.
    /// `grid_line` uses the same coordinate system as `search_text` results.
    pub fn scroll_to_line(&mut self, grid_line: i32) {
        let screen_lines = self.term.screen_lines() as i32;
        // Compute how far from the bottom this line is.
        // display_offset = distance from bottom: 0 = showing the very bottom.
        // The bottom-most visible line is at grid_line = screen_lines - 1 - display_offset..
        // We want the target line roughly centered in the viewport.
        let half_page = screen_lines / 2;
        // Target: make grid_line appear at ~center. The last grid line is
        // (screen_lines - 1). display_offset = (screen_lines - 1) - top_visible_line.
        let target_top = grid_line - half_page;
        let new_offset = (screen_lines - 1) - target_top;
        let max_offset = self.history_size() as i32;
        let clamped = new_offset.clamp(0, max_offset) as usize;
        // Use Scroll::Bottom then Delta to set exact offset.
        self.term
            .scroll_display(alacritty_terminal::grid::Scroll::Bottom);
        if clamped > 0 {
            self.term
                .scroll_display(alacritty_terminal::grid::Scroll::Delta(clamped as i32));
        }
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
