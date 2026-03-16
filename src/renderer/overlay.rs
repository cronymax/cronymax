//! Three-tier renderer container architecture: Base -> Overlay -> Float.
//!
//! ## Z-index hierarchy
//!
//! ```text
//! +-----------------------------------------------------+
//! |  Float  (tooltips, popovers)                 |  z = 3
//! +-----------------------------------------------------+
//! |  Modal  (browser overlays, settings)       |  z = 2
//! +-----------------------------------------------------+
//! |  BaseRenderer  (main window, terminals, egui)        |  z = 1
//! +-----------------------------------------------------+
//! ```
//!
//! Each tier wraps a platform window + GPU surface + egui context:
//!
//! - **[`Modal`]**: A child window (NSPanel / owned popup) that floats
//!   above native webviews.
//! - **[`Float`]**: A non-interactive child window for tooltips that floats
//!   above everything. Click-through, ignores mouse events, highest z-order.
pub mod float;
pub mod float_panel;
pub mod modal;
pub mod modal_panel;

use crate::renderer::egui_pass::EguiIntegration;

pub use float::Float;
pub use float_panel::{FloatPanel, FloatPanelState};
pub use modal::Modal;
pub use modal_panel::ModalPanel;

// ── macOS coordinate helpers ────────────────────────────────────────────────

// --- Shared trait ---

/// Common interface for renderer tiers that own a wgpu surface + egui context.
pub trait Renderer {
    /// Resize the GPU surface.
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32, scale: f32);
    /// Whether the renderer is currently visible.
    fn is_visible(&self) -> bool;
    /// Show or hide the renderer.
    fn set_visible(&mut self, visible: bool);
}

// ===========================================================================
// Overlay<Panel> -- shared GPU surface + egui context for child windows
// ===========================================================================

/// Shared GPU-side state owned by both [`Modal`] and [`Float`].
///
/// Encapsulates the wgpu surface, surface configuration, standalone egui context,
/// and logical dimensions. Created once per child window.
pub struct Overlay<Panel> {
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub egui: EguiIntegration,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub panel: Panel,
}

impl<Panel> Overlay<Panel> {
    #[cfg(target_os = "macos")]
    pub fn new(
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
        EguiIntegration::install_cjk_font_standalone(&ctx);
        let egui = EguiIntegration::new_standalone(&gpu.device, surface_format, ctx);

        Ok(Self {
            surface,
            surface_config,
            egui,
            width: width as f32 / scale,
            height: height as f32 / scale,
            scale,
            panel,
        })
    }

    #[cfg(target_os = "windows")]
    pub fn new(
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
        EguiIntegration::install_cjk_font_standalone(&ctx);
        let egui = EguiIntegration::new_standalone(&gpu.device, surface_format, ctx);

        Ok(Self {
            surface,
            surface_config,
            egui,
            width: width as f32 / scale,
            height: height as f32 / scale,
            scale,
            panel,
        })
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32, scale: f32) {
        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.width = width as f32 / scale;
            self.height = height as f32 / scale;
            self.scale = scale;
            self.surface.configure(device, &self.surface_config);
        }
    }
}
