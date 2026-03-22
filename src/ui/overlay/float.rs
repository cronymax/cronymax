use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::{
    config::AppConfig,
    renderer::{
        overlay::Overlay,
        panel::{LogicalRect, Panel, PanelAttrs},
        Renderer,
    },
    ui::{TooltipRequest, View, ViewMut},
};

// ── FloatPanelState ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FloatPanelState {
    pub tooltip: Option<TooltipRequest>,
}

impl FloatPanelState {
    pub fn clear(&mut self) {
        self.tooltip = None;
    }
}
/// Tier 3: Non-interactive child window for tooltips.
pub struct Float {
    pub ow: Overlay,
    ctx_initialized: bool,
    pending_textures_delta: Option<egui::TexturesDelta>,
    /// Opaque clear color for the wgpu surface, set before each render.
    clear_color: wgpu::Color,
}

impl std::ops::Deref for Float {
    type Target = Overlay;
    fn deref(&self) -> &Self::Target {
        &self.ow
    }
}
impl std::ops::DerefMut for Float {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ow
    }
}

impl Float {
    pub fn new(
        parent: &Window,
        event_loop: Option<&ActiveEventLoop>,
        gpu: &crate::renderer::GpuContext,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, String> {
        let rect = LogicalRect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            scale,
        };
        let attrs = PanelAttrs {
            shadow: true,
            focusable: false,
            click_through: true,
            level_offset: 1,
            initially_visible: false,
            corner_radius: 8.0,
            opaque: false,
        };
        let panel = Panel::new(parent, event_loop, rect, attrs)?;
        let mut ow = Overlay::new(panel, gpu, width, height, scale)?;

        // Force the surface alpha_mode to Opaque so wgpu tells Metal the
        // CAMetalLayer is opaque.  This prevents the compositor from
        // alpha-blending the surface, eliminating the washed-out gray look.
        ow.surface_config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        ow.surface.configure(&ow.device, &ow.surface_config);

        // Re-apply layer masking after wgpu has created the CAMetalLayer.
        #[cfg(target_os = "macos")]
        ow.panel.configure_layer(8.0, false);

        Ok(Self {
            ow,
            ctx_initialized: false,
            pending_textures_delta: None,
            clear_color: wgpu::Color::BLACK,
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

        let mut tooltip_rect = egui::Rect::NOTHING;
        let full_output = self.egui.ctx.run(raw_input, |ctx| {
            ctx.set_visuals(visuals.clone());
            let resp = egui::Area::new(egui::Id::new("float_measure"))
                .fixed_pos(egui::pos2(0.0, 0.0))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(
                            styles.spacing.medium as i8,
                            styles.spacing.small as i8,
                        ))
                        .show(ui, |ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            ui.label(
                                egui::RichText::new(&text_owned)
                                    .color(colors.text_title)
                                    .size(styles.typography.caption1),
                            );
                        });
                });
            tooltip_rect = resp.response.rect;
        });

        self.ctx_initialized = true;
        self.pending_textures_delta = Some(full_output.textures_delta);

        let w = tooltip_rect.width().max(30.0) + 2.0;
        let h = tooltip_rect.height().max(16.0) + 2.0;
        (w, h)
    }

    /// Render a tooltip on the float panel surface.
    pub fn render_tooltip(
        &mut self,
        text: &str,
        app_config: &AppConfig,
        ui_state: &mut crate::ui::UiState,
    ) -> Result<crate::ui::widget::Dirties, wgpu::SurfaceError> {
        // Set opaque clear color from theme so the entire surface is opaque,
        // eliminating alpha compositing artifacts (ghosting).
        let bg = app_config.resolve_colors().bg_float;
        self.clear_color = wgpu::Color {
            r: bg.r() as f64 / 255.0,
            g: bg.g() as f64 / 255.0,
            b: bg.b() as f64 / 255.0,
            a: 1.0,
        };
        let text_owned = text.to_string();
        self.render(app_config, ui_state, |f| {
            let styles = f.styles;
            egui::Area::new(egui::Id::new("float_tooltip"))
                .fixed_pos(egui::pos2(0.0, 0.0))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .show(f.painter, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(
                            styles.spacing.medium as i8,
                            styles.spacing.small as i8,
                        ))
                        .show(ui, |ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                            ui.label(
                                egui::RichText::new(&text_owned)
                                    .color(f.colors.text_title)
                                    .size(styles.typography.caption1),
                            );
                        });
                });
        })
    }
}

impl View for Float {
    type Renderer = Overlay;
    fn as_renderer(&self) -> &Self::Renderer {
        &self.ow
    }
}

impl ViewMut for Float {
    fn as_mut_renderer(&mut self) -> &mut Self::Renderer {
        &mut self.ow
    }

    /// Override resize to re-apply CAMetalLayer corner masking after
    /// `surface.configure()` (which wgpu calls inside `Overlay::resize`).
    fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.as_mut_renderer().resize(width, height, scale);
        #[cfg(target_os = "macos")]
        self.ow.panel.configure_layer(8.0, false);
    }

    fn prepare_raw_input(&mut self) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(self.ow.width, self.ow.height),
            )),
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(self.ow.scale),
                    ..Default::default()
                },
            ))
            .collect(),
            ..Default::default()
        }
    }
    fn manipulate_full_output(&mut self, mut full_output: egui::FullOutput) -> egui::FullOutput {
        // Merge extra textures.
        if let Some(extra) = self.pending_textures_delta.take() {
            full_output.textures_delta.set.extend(extra.set);
            full_output.textures_delta.free.extend(extra.free);
        }
        full_output
    }
    fn present(
        &mut self,
        full_output: ::egui::FullOutput,
        terminal_output: Option<crate::renderer::atlas::TerminalOutput>,
    ) -> Result<(), wgpu::SurfaceError> {
        // Use our opaque clear color instead of TRANSPARENT to prevent
        // alpha-compositing ghosting on the Float tooltip surface.
        let clear = self.clear_color;
        let platform_output = self.as_mut_renderer().present(
            full_output,
            Some(clear),
            None,
            terminal_output,
        )?;
        self.handle_platform_output(platform_output);
        Ok(())
    }
}
