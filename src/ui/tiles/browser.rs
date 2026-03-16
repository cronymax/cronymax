//! Browser view pane widget — `BrowserViewPane` struct + rendering methods.

use super::*;
use crate::ui::widget::Widget;

/// Stateful widget for rendering a browser view pane (address bar + content rect).
///
/// Persists across frames in `PaneWidgetStore::browser`.
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

/// Temporary view adapting `BrowserViewPane` to `Widget<egui::Ui>`.
pub struct BrowserPaneView<'a> {
    pub widget: &'a mut BrowserViewPane,
}

impl Widget<egui::Ui> for BrowserPaneView<'_> {
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <egui::Ui as crate::ui::widget::Painter>::Ref<'a>,
        #[allow(unused)] mut ctx: crate::ui::widget::Context<'a>,
    ) {
        let styles = ctx.styles;

        let full_rect = ui.available_rect_before_wrap();
        let bar_h = styles.address_bar_height();
        let bar_rect =
            egui::Rect::from_min_size(full_rect.min, egui::vec2(full_rect.width(), bar_h));
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(full_rect.min.x, full_rect.min.y + bar_h),
            full_rect.max,
        );

        let webview_id = self.widget.webview_id;

        ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(bar_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::same(styles.spacing.medium as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_centered(|ui| {
                            ctx.bind::<egui::Ui>(ui)
                                .add(crate::ui::browser::AddressBarWidget {
                                    close_webview_id: webview_id,
                                    editing: &mut self.widget.editing,
                                    tooltip: &mut self.widget.tooltip,
                                    url: &mut self.widget.url,
                                });
                        });
                    });
            },
        );
        let _response = ui.allocate_rect(content_rect, egui::Sense::click_and_drag());

        ctx.dirties.tile_rects.push(TileRect::BrowserView {
            webview_id,
            rect: content_rect,
        });
        ctx.dirties.tile_rects.push(TileRect::BrowserView {
            webview_id,
            rect: content_rect,
        });
    }
}
