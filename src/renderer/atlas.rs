//! Glyph atlas management via glyphon.

use glyphon::{
    Cache, FontSystem, Metrics, Resolution, SwashCache, TextAtlas, TextRenderer, Viewport,
};
use wgpu::{Device, MultisampleState, Queue, TextureFormat};

use crate::renderer::quad::QuadRenderer;

/// Cell dimensions computed from font metrics.
#[derive(Debug, Clone, Copy)]
pub struct CellSize {
    pub width: f32,
    pub height: f32,
}

/// Manages the glyphon text rendering pipeline: font system, atlas, and renderer.
pub struct TerminalRenderer {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub atlas: TextAtlas,
    pub text_renderer: TextRenderer,
    pub viewport: Viewport,
    pub cell_size: CellSize,
    pub quad_renderer: QuadRenderer,
}

impl TerminalRenderer {
    /// Create a new terminal renderer with the given font configuration.
    pub fn new(
        device: &Device,
        queue: &Queue,
        format: TextureFormat,
        font_family: &str,
        font_size: f32,
        line_height_mult: f32,
    ) -> Self {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let text_renderer =
            TextRenderer::new(&mut atlas, device, MultisampleState::default(), None);

        // Create quad renderer for UI backgrounds.
        let quad_renderer = QuadRenderer::new(device, format);

        // Compute cell size from font metrics.
        let cell_size =
            Self::compute_cell_size(&mut font_system, font_family, font_size, line_height_mult);

        log::info!(
            "TerminalRenderer initialized: cell={}x{}, font='{}'@{}pt",
            cell_size.width,
            cell_size.height,
            font_family,
            font_size
        );

        Self {
            font_system,
            swash_cache,
            atlas,
            text_renderer,
            viewport,
            cell_size,
            quad_renderer,
        }
    }

    /// Compute monospace cell dimensions using cosmic-text metrics.
    fn compute_cell_size(
        font_system: &mut FontSystem,
        family: &str,
        size: f32,
        line_height_mult: f32,
    ) -> CellSize {
        use glyphon::{Attrs, Buffer, Family, Shaping};

        let metrics = Metrics::new(size, size * line_height_mult);
        let mut buffer = Buffer::new(font_system, metrics);

        let family_val = if family == "monospace" || family.is_empty() {
            Family::Monospace
        } else {
            Family::Name(family)
        };

        // Measure a single 'M' glyph to determine cell size.
        buffer.set_text(
            font_system,
            "M",
            &Attrs::new().family(family_val),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);

        let line_height = metrics.line_height;
        let mut cell_width = size * 0.6; // fallback

        if let Some(run) = buffer.layout_runs().next()
            && let Some(glyph) = run.glyphs.iter().next()
        {
            cell_width = glyph.w;
        }

        CellSize {
            width: cell_width.ceil(),
            height: line_height.ceil(),
        }
    }

    /// Update the viewport resolution (call on resize).
    pub fn update_viewport(&mut self, queue: &Queue, width: u32, height: u32) {
        self.viewport.update(queue, Resolution { width, height });
    }
}
