// Markdown rendering — egui_commonmark wrappers for streaming markdown.

use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

fn render_with_link_tooltips(ui: &mut egui::Ui, cache: &mut CommonMarkCache, content: &str) {
    let prior = ui.style().url_in_tooltip;
    ui.style_mut().url_in_tooltip = true;
    CommonMarkViewer::new().show(ui, cache, content);
    ui.style_mut().url_in_tooltip = prior;
}

/// Render a completed message as markdown.
pub fn render_message(ui: &mut egui::Ui, content: &str, cache: &mut CommonMarkCache) {
    render_with_link_tooltips(ui, cache, content);
}

/// Render a streaming buffer (in-progress response).
/// The cache should be cleared on each new token for correct incremental rendering.
pub fn render_streaming(ui: &mut egui::Ui, buffer: &str, cache: &mut CommonMarkCache) {
    if !buffer.is_empty() {
        render_with_link_tooltips(ui, cache, buffer);
    }
}
