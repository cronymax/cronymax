//! SVG icon system — VSCode-style icons rendered as egui textures.
//!
//! Each icon is stored as an inline SVG string.  On first use, the SVG is
//! rasterised to an `egui::ColorImage` and loaded into the texture manager.
//! Subsequent frames use the cached texture handle.
//!
//! Icons sourced from the VSCode Codicons icon font
//! (<https://github.com/microsoft/vscode-codicons>).

use std::collections::HashMap;
use std::sync::Mutex;

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

/// Return the raw SVG source for the given icon (VSCode Codicon style, 16×16 viewBox).
/// Stroke/fill is set to `currentColor` placeholder that we replace at render time.
fn svg_source(icon: Icon) -> &'static str {
    match icon {
        Icon::ArrowLeft => include_str!("icons/arrow-left.svg"),
        Icon::ArrowRight => include_str!("icons/arrow-right.svg"),
        Icon::Refresh => include_str!("icons/refresh.svg"),
        Icon::SplitHorizontal => include_str!("icons/split-horizontal.svg"),
        Icon::SplitVertical => include_str!("icons/split-vertical.svg"),
        Icon::Close => include_str!("icons/close.svg"),
        Icon::OpenInProduct => include_str!("icons/open-in-product.svg"),
        Icon::Pin => include_str!("icons/pin.svg"),
        Icon::Pinned => include_str!("icons/pinned.svg"),
        Icon::ChromeClose => include_str!("icons/chrome-close.svg"),
        Icon::ChromeMinimize => include_str!("icons/chrome-minimize.svg"),
        Icon::ChromeMaximize => include_str!("icons/chrome-maximize.svg"),
        Icon::LayoutSidebarLeft => include_str!("icons/layout-sidebar-left.svg"),
        Icon::LayoutSidebarRight => include_str!("icons/layout-sidebar-right.svg"),
        Icon::LayoutPanel => include_str!("icons/layout-panel.svg"),
        Icon::Add => include_str!("icons/add.svg"),
        Icon::Search => include_str!("icons/search.svg"),
        Icon::Filter => include_str!("icons/filter.svg"),
        Icon::Terminal => include_str!("icons/terminal.svg"),
        Icon::Globe => include_str!("icons/globe.svg"),
        Icon::ChatSparkle => include_str!("icons/chat-sparkle.svg"),
        Icon::SettingsGear => include_str!("icons/settings-gear.svg"),
        Icon::Feishu => include_str!("icons/feishu.svg"),
    }
}

/// Parse an SVG string and rasterise it to an RGBA image at the given size,
/// replacing `currentColor` with the specified colour.
fn rasterize_svg(svg_src: &str, color: egui::Color32, size: u32) -> egui::ColorImage {
    // Replace `currentColor` and `fill="currentColor"` with the actual hex colour.
    let hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let svg_str = svg_src.replace("currentColor", &hex);

    // Parse the SVG and render to a tiny pixmap.
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(&svg_str, &opt).expect("Invalid SVG icon");

    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("Failed to create pixmap");

    let sx = size as f32 / tree.size().width();
    let sy = size as f32 / tree.size().height();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );

    let pixels: Vec<egui::Color32> = pixmap
        .data()
        .chunks_exact(4)
        .map(|p| {
            // Pre-multiplied alpha from resvg → straight alpha for egui.
            let a = p[3];
            if a == 0 {
                egui::Color32::TRANSPARENT
            } else {
                let r = ((p[0] as u16 * 255) / a as u16).min(255) as u8;
                let g = ((p[1] as u16 * 255) / a as u16).min(255) as u8;
                let b = ((p[2] as u16 * 255) / a as u16).min(255) as u8;
                egui::Color32::from_rgba_unmultiplied(r, g, b, a)
            }
        })
        .collect();

    egui::ColorImage {
        size: [size as usize, size as usize],
        pixels,
    }
}

/// Cache key: (icon, color, size).
type CacheKey = (Icon, [u8; 4], u32);

/// Per-context icon texture cache.
///
/// Wraps an `Arc<Mutex<HashMap>>` so that cloning (required by egui's
/// `get_temp`/`insert_temp`) is cheap and shares the underlying map.
#[derive(Clone)]
struct IconTextureCache(std::sync::Arc<Mutex<HashMap<CacheKey, egui::TextureHandle>>>);

/// Get (or create) a texture handle for the given icon + colour + pixel size.
///
/// Textures are cached **per egui context** (via `ctx.data_mut`) so that
/// multiple contexts (main window, child window, float panel) each have
/// their own set of texture handles.  A global cache would return handles
/// created in one context that are invalid in another.
pub fn icon_texture(
    ctx: &egui::Context,
    icon: Icon,
    color: egui::Color32,
    size: u32,
) -> egui::TextureHandle {
    let key: CacheKey = (icon, [color.r(), color.g(), color.b(), color.a()], size);
    let cache_id = egui::Id::new("icon_texture_cache");

    // Step 1: Check per-context cache.
    let cache_arc = ctx.data_mut(|d| {
        let cache = d.get_temp::<IconTextureCache>(cache_id).unwrap_or_else(|| {
            let c = IconTextureCache(std::sync::Arc::new(Mutex::new(HashMap::new())));
            d.insert_temp(cache_id, c.clone());
            c
        });
        cache.0.clone()
    });

    {
        let guard = cache_arc.lock().unwrap();
        if let Some(handle) = guard.get(&key) {
            return handle.clone();
        }
    }

    // Step 2: Cache miss — rasterise and load into THIS context.
    let image = rasterize_svg(svg_source(icon), color, size);
    let handle = ctx.load_texture(
        format!(
            "{:?}_{:02x}{:02x}{:02x}_{}",
            icon,
            color.r(),
            color.g(),
            color.b(),
            size
        ),
        image,
        egui::TextureOptions::LINEAR,
    );
    cache_arc.lock().unwrap().insert(key, handle.clone());
    handle
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

/// Draw an SVG icon button.  Returns the egui Response.
///
/// `pixel_size` is the logical icon size (will be scaled by the context's
/// pixels_per_point automatically).
pub fn icon_button(ui: &mut egui::Ui, cfg: IconButtonCfg<'_>) -> egui::Response {
    let btn_size = egui::vec2(cfg.pixel_size + cfg.margin, cfg.pixel_size + cfg.margin);
    let (rect, response) = ui.allocate_exact_size(btn_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let color = if response.hovered() {
            // Draw a subtle rounded background on hover.
            ui.painter().rect_filled(
                rect,
                cfg.margin,
                egui::Color32::from_rgba_unmultiplied(128, 128, 128, 40),
            );
            cfg.hover_color
        } else {
            cfg.base_color
        };
        let tex_size = (cfg.pixel_size * ui.ctx().pixels_per_point()).round() as u32;
        let tex_size = tex_size.max(16); // minimum 16px for quality
        let texture = icon_texture(ui.ctx(), cfg.icon, color, tex_size);
        let img_rect =
            egui::Rect::from_center_size(rect.center(), egui::vec2(cfg.pixel_size, cfg.pixel_size));
        ui.painter().image(
            texture.id(),
            img_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }

    if !cfg.tooltip.is_empty() {
        response.clone().on_hover_text(cfg.tooltip);
    }

    response
}

/// Configuration for an SVG icon button with a status indicator.
pub struct IconButtonStatusCfg<'a> {
    pub icon: Icon,
    pub tooltip: &'a str,
    pub pixel_size: f32,
    pub stroke: Option<egui::Stroke>,
    pub corner_radius: egui::CornerRadius,
}

/// Draw a colorful SVG icon button with a background status indicator dot.
///
/// Unlike `icon_button` which tints the icon via `currentColor`, this renders
/// the icon with its **original SVG colors** (using `Color32::WHITE` as the
/// tint passthrough) and draws a small colored circle behind the icon to
/// indicate connection status.
///
/// - `status_color`: `Some(color)` to draw a background status circle, `None` for no indicator.
/// - `pixel_size`: logical icon size (scaled by pixels_per_point automatically).
pub fn icon_button_with_status(ui: &mut egui::Ui, cfg: IconButtonStatusCfg<'_>) -> egui::Response {
    let btn_size = egui::vec2(cfg.pixel_size + 4.0, cfg.pixel_size + 4.0);
    let (rect, response) = ui.allocate_exact_size(btn_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        // Draw status background circle if provided.
        if let Some(stroke) = cfg.stroke {
            ui.painter()
                .rect_stroke(rect, cfg.corner_radius, stroke, egui::StrokeKind::Outside);
        }

        // Render icon with original SVG colors (use WHITE as a neutral tint
        // so `currentColor` replacements don't affect multi-color SVGs).
        let tex_size = (cfg.pixel_size * ui.ctx().pixels_per_point()).round() as u32;
        let tex_size = tex_size.max(16);
        let texture = icon_texture(ui.ctx(), cfg.icon, egui::Color32::WHITE, tex_size);
        let img_rect =
            egui::Rect::from_center_size(rect.center(), egui::vec2(cfg.pixel_size, cfg.pixel_size));
        ui.painter().image(
            texture.id(),
            img_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }

    if !cfg.tooltip.is_empty() {
        response.clone().on_hover_text(cfg.tooltip);
    }

    response
}
