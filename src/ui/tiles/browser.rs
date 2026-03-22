//! Browser view pane widget — `BrowserViewPane` struct + rendering methods.

use super::*;
use crate::ui::widget::Widget;

/// Stateful widget for rendering a browser view pane (address bar + content rect).
///
/// Persists across frames in `PaneWidgetStore::browser`.
#[derive(Debug)]
pub struct BrowserViewPane {
    pub webview_id: u32,
    /// Current URL — synced from/to `Pane::BrowserView.url` by the Behavior dispatcher.
    pub url: String,
    /// Whether the address bar text field is currently being edited.
    pub editing: bool,
    /// Tooltip request from address bar button hovers.
    pub tooltip: Option<TooltipRequest>,
}

impl BrowserViewPane {
    pub fn new(webview_id: u32) -> Self {
        Self {
            webview_id,
            url: String::new(),
            editing: false,
            tooltip: None,
        }
    }
}

impl Widget<egui::Ui> for BrowserViewPane {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let styles = &f.styles;

        let full_rect = f.available_rect_before_wrap();
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(
                full_rect.min.x,
                full_rect.min.y + styles.address_bar_height(),
            ),
            full_rect.max,
        );

        let webview_id = self.webview_id;

        f.add(crate::ui::browser::BrowserView {
            webview_id,
            url: &mut self.url,
            editing: &mut self.editing,
            docked: true,
        });
        let _response = f.allocate_rect(content_rect, egui::Sense::click_and_drag());

        f.dirties.tile_rects.push(TileRect::BrowserView {
            webview_id,
            rect: content_rect,
        });
    }
}
