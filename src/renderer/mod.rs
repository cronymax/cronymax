pub mod atlas;
pub mod bridge;
pub mod cursor;
pub mod egui;
pub mod platform;
pub mod quad;
pub mod scheduler;
pub mod terminal;
pub mod text;
pub mod viewport;
pub mod webview;

// windowing
pub mod frame;
pub mod overlay;
pub mod panel;

use std::sync::Arc;
use winit::window::Window;

use crate::renderer::atlas::TerminalOutput;

// ── Renderer trait ──────────────────────────────────────────────────────────

pub trait Renderer {
    type Context<'a>;

    /// Resize the GPU surface.
    fn resize(&mut self, width: u32, height: u32, scale: f32);
    /// Whether the renderer is currently visible.
    fn is_visible(&self) -> bool;
    /// Show or hide the renderer.
    fn set_visible(&mut self, visible: bool);

    /// Low-level GPU submit: acquire surface texture, run `f` with the
    /// render context, then present.
    fn submit<U, F>(&mut self, f: F) -> Result<U, wgpu::SurfaceError>
    where
        F: FnOnce(Self::Context<'_>) -> U;

    /// Run egui's CPU pass only (no GPU work).
    ///
    /// Calls `egui::Context::run(raw_input, run_ui)` and returns the
    /// full output.  The caller can then inspect / transform the output
    /// via [`EguiLifecycle::manipulate_full_output`] before handing it to
    /// [`present_egui`](Renderer::present_egui).
    fn run_ui<F>(&mut self, raw_input: ::egui::RawInput, run_ui: F) -> ::egui::FullOutput
    where
        F: FnMut(&::egui::Context);

    /// GPU submit: optional clear pass + tessellate + egui render + present.
    ///
    /// Consumes the `FullOutput` produced by [`run_ui`](Renderer::run_ui)
    /// (possibly transformed by [`EguiLifecycle::manipulate_full_output`]).
    fn present(
        &mut self,
        full_output: ::egui::FullOutput,
        clear_color: Option<wgpu::Color>,
        extra_textures: Option<::egui::TexturesDelta>,
        terminal_output: Option<TerminalOutput>,
    ) -> Result<::egui::PlatformOutput, wgpu::SurfaceError>;
}

/// wgpu device, surface, and pipeline setup.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
}

impl GpuContext {
    /// Create a new GPU context for the given window.
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: crate::renderer::platform::preferred_backend(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find a suitable GPU adapter");

        log::info!("GPU adapter: {:?}", adapter.get_info().name);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cronymax-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .expect("Failed to create GPU device");

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        log::info!(
            "Surface format: {:?}, alpha_modes: {:?}",
            surface_format,
            surface_caps.alpha_modes
        );

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            // Main window surface is always opaque — rounded corners are
            // handled by CALayer masking, not surface alpha.  Only overlay
            // child-window surfaces need PostMultiplied.
            alpha_mode: surface_caps
                .alpha_modes
                .iter()
                .copied()
                .find(|m| *m == wgpu::CompositeAlphaMode::Opaque)
                .or_else(|| {
                    surface_caps
                        .alpha_modes
                        .iter()
                        .copied()
                        .find(|m| *m == wgpu::CompositeAlphaMode::PostMultiplied)
                })
                .unwrap_or(surface_caps.alpha_modes[0]),
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            surface_config,
        }
    }

    /// Reconfigure the surface after a window resize.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    /// Get the surface texture format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }
}
