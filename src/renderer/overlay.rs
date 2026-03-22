use raw_window_handle::HasWindowHandle;

use crate::renderer::atlas::TerminalOutput;
use crate::renderer::egui::{EguiRenderArgs, ScreenDescriptor};

use super::egui::EguiRenderer;
use super::panel::Panel;

/// Bundles mutable references the closure receives from [`Overlay::render`].
pub struct OverlayCtx<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub view: &'a wgpu::TextureView,
    pub egui: &'a mut EguiRenderer,
    pub surface_config: &'a wgpu::SurfaceConfiguration,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

// ===========================================================================
// Overlay — unified child window with GPU surface + egui context
// ===========================================================================

/// A child window model integrating a platform child window ([`ChildPanel`]),
/// a wgpu surface, and a standalone egui context for independent rendering.
///
/// Replaces the previous `Overlay<Panel>` + `ui::Overlay<P>` two-layer
/// design with a single non-generic struct.
///
/// Container widgets `Modal` (focusable, for browser overlays and settings)
/// and `Float` (non-interactive, for tooltips) are built on top of this.
pub struct Overlay {
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub egui: EguiRenderer,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub panel: Panel,
}

impl Overlay {
    /// Create a new `Overlay` from a [`ChildPanel`].
    ///
    /// Obtains the raw window handle from the panel, creates a wgpu surface,
    /// and initialises a standalone egui context.
    pub fn new(
        panel: Panel,
        gpu: &super::GpuContext,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, String> {
        let wh = panel
            .window_handle()
            .map_err(|e| format!("Failed to get window handle: {e}"))?
            .as_raw();

        Self::create_surface(panel, gpu, wh, width, height, scale)
    }

    #[cfg(target_os = "macos")]
    fn create_surface(
        panel: Panel,
        gpu: &super::GpuContext,
        window_handle: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, String> {
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: raw_window_handle::RawDisplayHandle::AppKit(
                raw_window_handle::AppKitDisplayHandle::new(),
            ),
            raw_window_handle: window_handle,
        };

        let surface = unsafe {
            gpu.instance
                .create_surface_unsafe(target)
                .map_err(|e| format!("Failed to create child wgpu surface: {e}"))?
        };

        let surface_caps = surface.get_capabilities(&gpu.adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps
                .alpha_modes
                .iter()
                .copied()
                .find(|m| *m == wgpu::CompositeAlphaMode::PostMultiplied)
                .or_else(|| {
                    surface_caps
                        .alpha_modes
                        .iter()
                        .copied()
                        .find(|m| *m == wgpu::CompositeAlphaMode::PreMultiplied)
                })
                .unwrap_or(surface_caps.alpha_modes[0]),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&gpu.device, &surface_config);

        let ctx = egui::Context::default();
        EguiRenderer::install_cjk_font_standalone(&ctx);
        let egui = EguiRenderer::new_standalone(&gpu.device, surface_format, ctx);

        Ok(Self {
            surface,
            surface_config,
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
            egui,
            width: width as f32 / scale,
            height: height as f32 / scale,
            scale,
            panel,
        })
    }

    #[cfg(target_os = "windows")]
    fn create_surface(
        panel: Panel,
        gpu: &super::GpuContext,
        window_handle: raw_window_handle::RawWindowHandle,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, String> {
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: raw_window_handle::RawDisplayHandle::Windows(
                raw_window_handle::WindowsDisplayHandle::new(),
            ),
            raw_window_handle: window_handle,
        };

        let surface = unsafe {
            gpu.instance
                .create_surface_unsafe(target)
                .map_err(|e| format!("Failed to create child wgpu surface: {e}"))?
        };

        let surface_caps = surface.get_capabilities(&gpu.adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&gpu.device, &surface_config);

        let ctx = egui::Context::default();
        EguiRenderer::install_cjk_font_standalone(&ctx);
        let egui = EguiRenderer::new_standalone(&gpu.device, surface_format, ctx);

        Ok(Self {
            surface,
            surface_config,
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
            egui,
            width: width as f32 / scale,
            height: height as f32 / scale,
            scale,
            panel,
        })
    }
}

impl super::Renderer for Overlay {
    type Context<'a> = OverlayCtx<'a>;

    fn resize(&mut self, width: u32, height: u32, scale: f32) {
        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.width = width as f32 / scale;
            self.height = height as f32 / scale;
            self.scale = scale;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    fn is_visible(&self) -> bool {
        self.panel.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.panel.set_visible(visible);
    }

    /// Run the full GPU rendering pipeline for one overlay frame.
    ///
    /// 1. Acquires the surface texture and creates a command encoder.
    /// 2. Calls `f` with an [`OverlayCtx`] — the closure drives the render
    ///    passes (clear, egui, etc.).
    /// 3. Submits the encoder and presents the surface.
    fn submit<U, F>(&mut self, f: F) -> Result<U, wgpu::SurfaceError>
    where
        F: FnOnce(Self::Context<'_>) -> U,
    {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("overlay-command-encoder"),
            });
        let result = f(OverlayCtx {
            device: &self.device,
            queue: &self.queue,
            encoder: &mut encoder,
            view: &view,
            egui: &mut self.egui,
            surface_config: &self.surface_config,
            width: self.width,
            height: self.height,
            scale: self.scale,
        });
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(result)
    }

    fn run_ui<F>(&mut self, raw_input: egui::RawInput, run_ui: F) -> egui::FullOutput
    where
        F: FnMut(&egui::Context),
    {
        self.egui.ctx.run(raw_input, run_ui)
    }

    fn present(
        &mut self,
        egui::FullOutput {
            pixels_per_point,
            shapes,
            mut textures_delta,
            platform_output,
            viewport_output: _,
        }: egui::FullOutput,
        clear_color: Option<wgpu::Color>,
        extra_textures: Option<egui::TexturesDelta>,
        _terminal_output: Option<TerminalOutput>,
    ) -> Result<egui::PlatformOutput, wgpu::SurfaceError> {
        let primitives = self.egui.ctx.tessellate(shapes, pixels_per_point);

        // Merge extra textures (e.g. Float's pending measurement delta).
        if let Some(extra) = extra_textures {
            textures_delta.set.extend(extra.set);
            textures_delta.free.extend(extra.free);
        }

        self.submit(|ctx| {
            // 1. Optional clear pass.
            if let Some(color) = clear_color {
                let _pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("overlay_clear_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: ctx.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(color),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });
            }

            // 2. Egui GPU render pass.
            ctx.egui.render_mut(EguiRenderArgs {
                device: ctx.device,
                queue: ctx.queue,
                encoder: ctx.encoder,
                color_target: ctx.view,
                primitives: &primitives,
                textures_delta: &textures_delta,
                screen_descriptor: ScreenDescriptor {
                    width_px: ctx.surface_config.width,
                    height_px: ctx.surface_config.height,
                    pixels_per_point: ctx.scale,
                },
            });
        })?;

        Ok(platform_output)
    }
}
