//! `Frame` — main application window model.
//!
//! Bundles `Arc<Window>` + `GpuContext` + `EguiIntegration` into a single
//! coherent model.  Business-free renderer-level foundation type.

use std::sync::Arc;

use winit::window::Window;

use super::GpuContext;
use super::atlas::{TerminalOutput, TerminalRenderer};
use super::egui::EguiRenderer;

/// The main application window integrating winit, wgpu, and egui.
///
/// This is the root window that hosts the tile tree, terminal panes,
/// and the primary wgpu rendering surface.
pub struct Frame {
    pub window: Arc<Window>,
    pub gpu: GpuContext,
    pub egui: EguiRenderer,
    pub state: egui_winit::State,
    /// Terminal text/quad renderer — co-located with the GPU context it depends on.
    pub terminal: TerminalRenderer,
}

impl Frame {
    /// Create a new `Frame` from its constituent parts.
    pub fn new(
        window: Arc<Window>,
        gpu: GpuContext,
        egui: EguiRenderer,
        terminal: TerminalRenderer,
    ) -> Self {
        let viewport_id = egui.ctx.viewport_id();
        let state =
            egui_winit::State::new(egui.ctx.clone(), viewport_id, &window, None, None, None);
        Self {
            window,
            gpu,
            egui,
            state,
            terminal,
        }
    }

    /// The DPI scale factor of the window.
    pub fn scale(&self) -> f32 {
        self.window.scale_factor() as f32
    }

    /// Current inner size in physical pixels.
    pub fn inner_size(&self) -> winit::dpi::PhysicalSize<u32> {
        self.window.inner_size()
    }

    /// Resize the GPU surface to match the new physical size.
    pub fn resize_surface(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
    }

    /// See also [`egui_winit::State::take_egui_input`](egui_winit::State::take_egui_input)
    pub fn take_egui_input(&mut self) -> egui::RawInput {
        self.state.take_egui_input(&self.window)
    }

    /// Forward a winit event to egui.
    ///
    /// Returns `true` when egui consumed the event **or** needs a repaint
    /// (e.g. pointer-move that updates hover state).  The caller should
    /// call `window.request_redraw()` when this returns `true`.
    ///
    /// See also [`egui_winit::State::on_window_event`](egui_winit::State::on_window_event)
    pub fn on_window_event(&mut self, event: &winit::event::WindowEvent) -> bool {
        let response = self.state.on_window_event(&self.window, event);
        response.consumed || response.repaint
    }

    /// Call with the output given by egui.
    ///
    /// This will, if needed:
    /// - update the cursor
    /// - copy text to the clipboard
    /// - open any clicked urls
    /// - update the IME
    ///
    /// See also [`egui_winit::State::handle_platform_output`](egui_winit::State::handle_platform_output)
    pub fn handle_platform_output(&mut self, platform_output: egui::PlatformOutput) {
        self.state
            .handle_platform_output(&self.window, platform_output);
    }
}

/// Bundles mutable references the closure receives from [`Frame::submit`].
pub struct FrameCtx<'a> {
    pub gpu: &'a GpuContext,
    pub view: &'a wgpu::TextureView,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub egui: &'a mut EguiRenderer,
    pub terminal: &'a mut TerminalRenderer,
}

impl super::Renderer for Frame {
    type Context<'a> = FrameCtx<'a>;

    fn is_visible(&self) -> bool {
        self.window.is_visible().is_some_and(|v| v)
    }

    fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.window
            .set_max_inner_size(Some(winit::dpi::PhysicalSize::new(
                (width as f32 / scale) as u32,
                (height as f32 / scale) as u32,
            )));
    }
    fn set_visible(&mut self, visible: bool) {
        self.window.set_visible(visible);
    }

    /// Run the full GPU rendering pipeline for one frame.
    ///
    /// 1. Acquires the surface texture view and creates a command encoder.
    /// 2. Calls `f` with a [`FrameCtx`] — the closure drives the render
    ///    passes (clear, egui, terminal, etc.).
    /// 3. Submits the encoder and presents the surface.
    fn submit<U, F>(&mut self, f: F) -> Result<U, wgpu::SurfaceError>
    where
        F: FnOnce(Self::Context<'_>) -> U,
    {
        let output = self.gpu.surface.get_current_texture()?;
        let view = output.texture.create_view(&Default::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame-command-encoder"),
            });
        let result = f(FrameCtx {
            encoder: &mut encoder,
            view: &view,
            gpu: &self.gpu,
            egui: &mut self.egui,
            terminal: &mut self.terminal,
        });
        self.gpu.queue.submit(std::iter::once(encoder.finish()));
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
        terminal_output: Option<TerminalOutput>,
    ) -> Result<egui::PlatformOutput, wgpu::SurfaceError> {
        let win_size = self.window.inner_size();
        let scale = self.window.scale_factor() as f32;

        let primitives = self.egui.ctx.tessellate(shapes, pixels_per_point);

        // Merge extra textures.
        if let Some(extra) = extra_textures {
            textures_delta.set.extend(extra.set);
            textures_delta.free.extend(extra.free);
        }

        let sw = win_size.width as f32;
        let sh = win_size.height as f32;

        self.submit(|ctx| {
            use crate::renderer::egui::{EguiRenderArgs, ScreenDescriptor};

            // 1. Optional clear pass.
            if let Some(color) = clear_color {
                let _pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("clear-pass"),
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
                device: &ctx.gpu.device,
                queue: &ctx.gpu.queue,
                encoder: ctx.encoder,
                color_target: ctx.view,
                primitives: &primitives,
                textures_delta: &textures_delta,
                screen_descriptor: ScreenDescriptor {
                    width_px: win_size.width,
                    height_px: win_size.height,
                    pixels_per_point: scale,
                },
            });

            // 3. Terminal render pass (staged via ViewMut::stage_terminal).
            if let Some(ref prepared) = terminal_output {
                ctx.terminal
                    .render_prepared(prepared, ctx.encoder, ctx.view, ctx.gpu, sw, sh);
            }
        })?;

        Ok(platform_output)
    }
}
