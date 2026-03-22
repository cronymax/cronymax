//! Glyph atlas management via glyphon.

use glyphon::{Cache, FontSystem, Metrics, Resolution, SwashCache, TextAtlas, TextRenderer};
use wgpu::{Device, MultisampleState, Queue, TextureFormat};

use crate::renderer::{quad::QuadRenderer, viewport::Viewport};

/// Cell dimensions computed from font metrics.
#[derive(Debug, Clone, Copy)]
pub struct CellSize {
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    /// Calculate grid dimensions (columns, rows) from this viewport and cell size.
    pub fn grid_dimensions(&self, cell: &CellSize) -> (u16, u16) {
        let cols = (self.width / cell.width).floor().max(1.0) as u16;
        let rows = (self.height / cell.height).floor().max(1.0) as u16;
        (cols, rows)
    }
}

/// Business-free terminal frame data ready for GPU rendering.
///
/// Built by the UI layer (which reads session/config state), then handed
/// to [`TerminalRenderer::render_prepared`] inside the GPU submit closure.
pub struct TerminalOutput {
    pub quads: Vec<crate::renderer::quad::Quad>,
    pub text_color: glyphon::Color,
    pub text_buffers: Vec<(glyphon::Buffer, Viewport)>,
}

/// Manages the glyphon text rendering pipeline: font system, atlas, and renderer.
pub struct TerminalRenderer {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub atlas: TextAtlas,
    pub text_renderer: TextRenderer,
    pub viewport: glyphon::Viewport,
    pub cell_size: CellSize,
    pub quad_renderer: QuadRenderer,
    /// Reusable scratch buffer to reduce allocations in the text render loop.
    pub(crate) text_scratch: String,
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
        let viewport = glyphon::Viewport::new(device, &cache);
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
            text_scratch: String::with_capacity(80 * 24 + 24),
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
            width: cell_width,
            height: line_height,
        }
    }

    /// Update the viewport resolution (call on resize).
    pub fn update_viewport(&mut self, queue: &Queue, width: u32, height: u32) {
        self.viewport.update(queue, Resolution { width, height });
    }

    /// GPU-submit a [`PreparedTerminalFrame`] (quads + text).
    ///
    /// This is the **business-free** rendering half — it only touches wgpu
    /// resources and doesn't need session/config data.
    pub fn render_prepared(
        &mut self,
        frame: &TerminalOutput,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        gpu: &super::GpuContext,
        sw: f32,
        sh: f32,
    ) {
        if frame.quads.is_empty() && frame.text_buffers.is_empty() {
            return;
        }

        // ── Quad pass (cursor, scrollbar, link underline) ────────────
        self.quad_renderer.prepare(&gpu.queue, &frame.quads, sw, sh);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("quad-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            self.quad_renderer.render(&mut pass);
        }

        // ── Text pass (all terminal panes at once) ───────────────────
        self.update_viewport(&gpu.queue, sw as u32, sh as u32);

        let text_areas: Vec<_> = frame
            .text_buffers
            .iter()
            .map(|(buf, v)| {
                super::text::terminal_text_area(
                    buf,
                    v.x,
                    v.y,
                    v.width as i32,
                    v.height as i32,
                    frame.text_color,
                )
            })
            .collect();

        self.text_renderer
            .prepare(
                &gpu.device,
                &gpu.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .expect("Failed to prepare text rendering");

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("Failed to render text");
        }
    }
}
