//! SVG icon system — VSCode-style icons rendered as egui textures.
//!
//! Each icon is stored as an inline SVG via `egui::include_image!`.
//! Hover tinting is achieved by rendering the image with `egui::Image::tint()`
//! using the base or hover colour depending on pointer state.
//!
//! Icons sourced from the VSCode Codicons icon font
//! (<https://github.com/microsoft/vscode-codicons>).

/// All available icon identifiers.
#[allow(unused)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    // Browser address bar
    ArrowLeft,
    ArrowRight,
    Refresh,
    SplitHorizontal,
    SplitVertical,
    Close,
    OpenInProduct,
    Pin,
    Pinned,
    // Titlebar
    ChromeClose,
    ChromeMinimize,
    ChromeMaximize,
    LayoutSidebarLeft,
    LayoutSidebarRight,
    LayoutPanel,
    // General
    Add,
    Search,
    Filter,
    Terminal,
    Globe,
    ChatSparkle,
    SettingsGear,
    Feishu,
}

impl Icon {
    /// Return the raw SVG source for the given icon (VSCode Codicon style, 16×16 viewBox).
    /// Stroke/fill is set to `currentColor` placeholder that we replace at render time.
    pub fn to_image_source(&self) -> egui::ImageSource<'static> {
        match self {
            Icon::ArrowLeft => egui::include_image!("icons/arrow-left.svg"),
            Icon::ArrowRight => egui::include_image!("icons/arrow-right.svg"),
            Icon::Refresh => egui::include_image!("icons/refresh.svg"),
            Icon::SplitHorizontal => egui::include_image!("icons/split-horizontal.svg"),
            Icon::SplitVertical => egui::include_image!("icons/split-vertical.svg"),
            Icon::Close => egui::include_image!("icons/close.svg"),
            Icon::OpenInProduct => egui::include_image!("icons/open-in-product.svg"),
            Icon::Pin => egui::include_image!("icons/pin.svg"),
            Icon::Pinned => egui::include_image!("icons/pinned.svg"),
            Icon::ChromeClose => egui::include_image!("icons/chrome-close.svg"),
            Icon::ChromeMinimize => egui::include_image!("icons/chrome-minimize.svg"),
            Icon::ChromeMaximize => egui::include_image!("icons/chrome-maximize.svg"),
            Icon::LayoutSidebarLeft => egui::include_image!("icons/layout-sidebar-left.svg"),
            Icon::LayoutSidebarRight => egui::include_image!("icons/layout-sidebar-right.svg"),
            Icon::LayoutPanel => egui::include_image!("icons/layout-panel.svg"),
            Icon::Add => egui::include_image!("icons/add.svg"),
            Icon::Search => egui::include_image!("icons/search.svg"),
            Icon::Filter => egui::include_image!("icons/filter.svg"),
            Icon::Terminal => egui::include_image!("icons/terminal.svg"),
            Icon::Globe => egui::include_image!("icons/globe.svg"),
            Icon::ChatSparkle => egui::include_image!("icons/chat-sparkle.svg"),
            Icon::SettingsGear => egui::include_image!("icons/settings-gear.svg"),
            Icon::Feishu => egui::include_image!("icons/feishu.svg"),
        }
    }
}

/// Configuration for an SVG icon button.
pub struct IconButtonCfg<'a> {
    pub icon: Icon,
    pub tooltip: &'a str,
    pub base_color: egui::Color32,
    pub hover_color: egui::Color32,
    pub pixel_size: f32,
    pub margin: f32,
}

/// Draw an SVG icon button with hover-tint support.
///
/// Allocates a clickable rectangle, detects hover, and paints the icon with
/// the appropriate tint colour.  Uses `egui::Image` so the SVG is decoded
/// and cached by egui's built-in image pipeline — no manual rasterisation.
pub fn icon_button(ui: &mut egui::Ui, cfg: IconButtonCfg<'_>) -> egui::Response {
    icon_button_at(ui, None, cfg)
}

/// Draw an SVG icon button at a specific rect, without allocating layout space.
///
/// Use this when the button must be painted inside an already-allocated area
/// (e.g. a tab close button inside the tab rect).
pub fn icon_button_at(
    ui: &mut egui::Ui,
    rect: impl Into<Option<egui::Rect>>,
    cfg: IconButtonCfg<'_>,
) -> egui::Response {
    let btn_size = egui::vec2(cfg.pixel_size + cfg.margin, cfg.pixel_size + cfg.margin);
    let (rect, response) = if let Some(rect) = rect.into() {
        let id = ui.id().with(("icon_btn_at", cfg.icon));
        (rect, ui.interact(rect, id, egui::Sense::click()))
    } else {
        ui.allocate_exact_size(btn_size, egui::Sense::click())
    };

    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();

        // Draw rounded hover background over the full button rect
        if hovered {
            ui.painter().rect_filled(
                rect,
                cfg.margin,
                egui::Color32::from_rgba_unmultiplied(128, 128, 128, 40),
            );
        }

        let icon_size = egui::vec2(cfg.pixel_size, cfg.pixel_size);
        let img_rect = egui::Rect::from_center_size(rect.center(), icon_size);
        let image = egui::Image::new(cfg.icon.to_image_source())
            .tint(if hovered {
                cfg.hover_color
            } else {
                cfg.base_color
            })
            .fit_to_exact_size(icon_size);
        image.paint_at(ui, img_rect);
    }

    if !cfg.tooltip.is_empty() {
        response.clone().on_hover_text(cfg.tooltip);
    }

    response
}
pub struct IconButtonStatusCfg<'a> {
    pub icon: Icon,
    pub tooltip: &'a str,
    pub pixel_size: f32,
    pub stroke: Option<egui::Stroke>,
    pub corner_radius: egui::CornerRadius,
}

/// Draw an SVG icon button with a border/status stroke.
///
/// Renders the icon with its **original SVG colours** (white tint passthrough)
/// and optionally draws a rounded border stroke around the button.
pub fn icon_button_with_status(ui: &mut egui::Ui, cfg: IconButtonStatusCfg<'_>) -> egui::Response {
    let btn_size = egui::vec2(cfg.pixel_size + 4.0, cfg.pixel_size + 4.0);
    let icon_size = egui::vec2(cfg.pixel_size, cfg.pixel_size);

    let (rect, response) = ui.allocate_exact_size(btn_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        if let Some(stroke) = cfg.stroke {
            ui.painter()
                .rect_stroke(rect, cfg.corner_radius, stroke, egui::StrokeKind::Outside);
        }

        let img_rect = egui::Rect::from_center_size(rect.center(), icon_size);
        let image = egui::Image::new(cfg.icon.to_image_source())
            .tint(egui::Color32::WHITE)
            .fit_to_exact_size(icon_size);
        image.paint_at(ui, img_rect);
    }

    if !cfg.tooltip.is_empty() {
        response.clone().on_hover_text(cfg.tooltip);
    }

    response
}
