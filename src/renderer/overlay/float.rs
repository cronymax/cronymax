use super::float_panel::FloatPanel;

use raw_window_handle::HasWindowHandle;
use winit::window::Window;

use crate::{
    config::AppConfig,
    renderer::{
        egui_pass::ScreenDescriptor,
        overlay::{Overlay, Renderer},
    },
};

/// Tier 3: Non-interactive child window for tooltips.
pub struct Float {
    pub overlay: Overlay<FloatPanel>,
    ctx_initialized: bool,
    pending_textures_delta: Option<egui::TexturesDelta>,
}

impl std::ops::Deref for Float {
    type Target = Overlay<FloatPanel>;
    fn deref(&self) -> &Self::Target {
        &self.overlay
    }
}
impl std::ops::DerefMut for Float {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.overlay
    }
}

impl Float {
    #[cfg(target_os = "macos")]
    pub fn new(
        gpu: &crate::renderer::GpuContext,
        panel: FloatPanel,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, String> {
        let wh = panel
            .window_handle()
            .map_err(|e| format!("Failed to get float panel window handle: {e}"))?
            .as_raw();
        let overlay = Overlay::new(panel, gpu, wh, width, height, scale)?;
        Ok(Self {
            overlay,
            ctx_initialized: false,
            pending_textures_delta: None,
        })
    }

    #[cfg(target_os = "windows")]
    pub fn new(
        gpu: &crate::renderer::GpuContext,
        panel: FloatPanel,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, String> {
        let wh = panel
            .window_handle()
            .map_err(|e| format!("Failed to get float panel window handle: {e}"))?
            .as_raw();
        let overlay = Overlay::new(panel, gpu, wh, width, height, scale)?;
        Ok(Self {
            overlay,
            ctx_initialized: false,
            pending_textures_delta: None,
        })
    }

    pub fn set_frame(&self, parent: &Window, sx: f32, sy: f32, w: f32, h: f32, scale: f32) {
        self.panel.set_frame(parent, sx, sy, w, h, scale);
    }

    pub fn ensure_above_overlays(&self) {
        self.panel.ensure_above_overlays();
    }

    /// Measure tooltip text size without rendering.
    /// Returns `(width, height)` in logical points.
    pub fn measure_tooltip(&mut self, app_config: &AppConfig, text: &str) -> (f32, f32) {
        let styles = &app_config.styles;
        let colors = app_config.resolve_colors();
        let visuals = styles.build_egui_visuals(&colors);
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(400.0, 200.0),
            )),
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(self.scale),
                    ..Default::default()
                },
            ))
            .collect(),
            ..Default::default()
        };

        let text_owned = text.to_string();
        let fg = colors.text_title;
        let font_size = styles.typography.body0;

        let full_output = self.egui.ctx.run(raw_input, |ctx| {
            ctx.set_visuals(visuals.clone());
            egui::Area::new(egui::Id::new("float_measure"))
                .fixed_pos(egui::pos2(0.0, 0.0))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .corner_radius(egui::CornerRadius::same(styles.radii.sm as u8))
                        .inner_margin(egui::Margin::symmetric(
                            styles.spacing.medium as i8,
                            styles.spacing.small as i8,
                        ))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&text_owned).color(fg).size(font_size));
                        });
                });
        });

        self.ctx_initialized = true;
        self.pending_textures_delta = Some(full_output.textures_delta);

        let used = self.egui.ctx.used_rect();
        let w = used.width().max(30.0) + 2.0;
        let h = used.height().max(16.0) + 2.0;
        (w, h)
    }

    /// Render a tooltip on the float panel surface.
    pub fn render_tooltip(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        text: &str,
        app_config: &AppConfig,
    ) -> Result<(), String> {
        let surface_texture = self
            .overlay
            .surface
            .get_current_texture()
            .map_err(|e| format!("Failed to get float surface texture: {e}"))?;

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("float_tooltip_encoder"),
        });

        let styles = &app_config.styles;
        let colors = app_config.resolve_colors();
        let visuals = styles.build_egui_visuals(&colors);
        let fg = colors.text_title;
        let font_size = styles.typography.body0;
        let text_owned = text.to_string();

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(self.width, self.height),
            )),
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(self.scale),
                    ..Default::default()
                },
            ))
            .collect(),
            ..Default::default()
        };

        let full_output = self.egui.ctx.run(raw_input, |ctx| {
            ctx.set_visuals(visuals.clone());
            egui::Area::new(egui::Id::new("float_tooltip"))
                .fixed_pos(egui::pos2(0.0, 0.0))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .corner_radius(egui::CornerRadius::same(styles.radii.sm as u8))
                        .inner_margin(egui::Margin::symmetric(
                            styles.spacing.medium as i8,
                            styles.spacing.small as i8,
                        ))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&text_owned).color(fg).size(font_size));
                        });
                });
        });

        let mut textures_delta = full_output.textures_delta;
        if let Some(pending) = self.pending_textures_delta.take() {
            for set in pending.set {
                textures_delta.set.push(set);
            }
            for free in pending.free {
                textures_delta.free.push(free);
            }
        }

        let primitives = self
            .overlay
            .egui
            .ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        let screen_desc = ScreenDescriptor {
            width_px: self.surface_config.width,
            height_px: self.surface_config.height,
            pixels_per_point: self.scale,
        };

        self.egui
            .render(crate::renderer::egui_pass::EguiRenderArgs {
                device,
                queue,
                encoder: &mut encoder,
                color_target: &view,
                primitives: &primitives,
                textures_delta: &textures_delta,
                screen_descriptor: screen_desc,
            });

        queue.submit(std::iter::once(encoder.finish()));
        surface_texture.present();

        Ok(())
    }
}

impl Renderer for Float {
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32, scale: f32) {
        self.overlay.resize(device, width, height, scale);
    }

    fn is_visible(&self) -> bool {
        self.panel.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.panel.set_visible(visible);
    }
}
