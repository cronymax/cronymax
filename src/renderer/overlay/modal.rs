use super::modal_panel::ModalPanel;

use std::sync::{Arc, Mutex};

use raw_window_handle::HasWindowHandle;
use winit::window::Window;

use crate::config::AppConfig;
use crate::renderer::overlay::{Overlay, Renderer};
use crate::ui::UiAction;
use crate::ui::types::TooltipRequest;
use crate::{renderer::egui_pass::ScreenDescriptor, webview::BrowserRenderResult};

/// Tier 2: A child window that floats above native webviews.
///
/// Used for browser overlay view and the Settings panel.
/// Each `Modal` owns one `ModalPanel` + one [`ChildSurface`].
pub struct Modal {
    pub overlay: Overlay<ModalPanel>,
}

impl std::ops::Deref for Modal {
    type Target = Overlay<ModalPanel>;
    fn deref(&self) -> &Self::Target {
        &self.overlay
    }
}
impl std::ops::DerefMut for Modal {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.overlay
    }
}

impl Modal {
    #[cfg(target_os = "macos")]
    pub fn new(
        gpu: &crate::renderer::GpuContext,
        panel: ModalPanel,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, String> {
        let wh = panel
            .window_handle()
            .map_err(|e| format!("Failed to get overlay panel window handle: {e}"))?
            .as_raw();
        let overlay = Overlay::new(panel, gpu, wh, width, height, scale)?;
        Ok(Self { overlay })
    }

    #[cfg(target_os = "windows")]
    pub fn new(
        gpu: &crate::renderer::GpuContext,
        panel: ModalPanel,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, String> {
        let wh = panel
            .window_handle()
            .map_err(|e| format!("Failed to get overlay panel window handle: {e}"))?
            .as_raw();
        let overlay = Overlay::new(panel, gpu, wh, width, height, scale)?;
        Ok(Self { overlay })
    }

    pub fn set_frame(&self, parent: &Window, lx: f32, ly: f32, lw: f32, lh: f32, scale: f32) {
        self.panel.set_frame_logical(parent, lx, ly, lw, lh, scale);
    }

    pub fn event_buffer(&self) -> &Arc<Mutex<Vec<egui::Event>>> {
        &self.panel.event_buffer
    }

    /// Run an egui frame on this overlay and present the result.
    pub fn render<F>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        app_config: &AppConfig,
        ui_fn: F,
    ) -> Result<Vec<UiAction>, String>
    where
        F: FnOnce(&egui::Context) -> Vec<UiAction>,
    {
        let surface_texture = self
            .surface
            .get_current_texture()
            .map_err(|e| format!("Failed to get overlay surface texture: {e}"))?;

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("overlay_egui_encoder"),
        });

        let buffered_events: Vec<egui::Event> = self
            .panel
            .event_buffer
            .lock()
            .map(|mut buf| buf.drain(..).collect())
            .unwrap_or_default();

        let visuals = app_config.resolve_egui_visuals();

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
            events: buffered_events,
            ..Default::default()
        };

        let mut actions = Vec::new();

        let mut ui_fn = Some(ui_fn);
        let full_output = self.egui.ctx.run(raw_input, |ctx| {
            ctx.set_visuals(visuals.clone());
            if let Some(f) = ui_fn.take() {
                actions = f(ctx);
            }
        });

        // Handle clipboard output from standalone egui context.
        for cmd in &full_output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = cmd {
                crate::terminal::input::copy_to_clipboard(text);
            }
        }
        #[allow(deprecated)]
        if !full_output.platform_output.copied_text.is_empty() {
            crate::terminal::input::copy_to_clipboard(&full_output.platform_output.copied_text);
        }

        let primitives = self
            .egui
            .ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        let screen_desc = ScreenDescriptor {
            width_px: self.surface_config.width,
            height_px: self.surface_config.height,
            pixels_per_point: self.scale,
        };

        // Clear to transparent so rounded corners are see-through.
        {
            let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay_clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
        }

        self.egui
            .render(crate::renderer::egui_pass::EguiRenderArgs {
                device,
                queue,
                encoder: &mut encoder,
                color_target: &view,
                primitives: &primitives,
                textures_delta: &full_output.textures_delta,
                screen_descriptor: screen_desc,
            });

        queue.submit(std::iter::once(encoder.finish()));
        surface_texture.present();

        Ok(actions)
    }

    /// Render browser overlay view (address bar + navigation buttons).
    ///
    /// `url` and `editing` are mutable references to the address bar state
    /// owned by the caller (typically `WebviewTab.address_bar`).
    pub fn render_browser_view(
        &mut self,
        url: &mut String,
        editing: &mut bool,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        panel_origin: [f32; 2],
        app_config: &AppConfig,
    ) -> Result<BrowserRenderResult, String> {
        use crate::ui::i18n::t;
        use crate::ui::icons::Icon;

        let mut url_local = std::mem::take(url);
        let mut is_editing = *editing;
        let mut tooltip: Option<TooltipRequest> = None;

        let styles = &app_config.styles;
        let colors = app_config.resolve_colors();
        let ab_fg = colors.text_title;
        let ab_em = colors.primary;
        let icon_size = styles.typography.title5;
        let margin = styles.spacing.medium;
        let corner_r = styles.radii.md;
        let browser_h = styles.address_bar_height();
        let small_sp = styles.spacing.small;

        let actions = self.render(device, queue, app_config, |ctx| {
            let mut actions: Vec<UiAction> = Vec::new();

            let mut check_hover = |resp: &egui::Response, text: &str| {
                if resp.hovered() {
                    let center = resp.rect.center();
                    let bottom = resp.rect.max.y;
                    tooltip = Some(TooltipRequest {
                        screen_x: panel_origin[0] + center.x,
                        screen_y: panel_origin[1] + bottom + 4.0,
                        text: text.to_string(),
                    });
                }
            };

            let render_icon = |ui: &mut egui::Ui, icon: Icon| -> egui::Response {
                crate::ui::icons::icon_button(
                    ui,
                    crate::ui::icons::IconButtonCfg {
                        icon,
                        tooltip: "",
                        base_color: ab_fg,
                        hover_color: ab_em,
                        pixel_size: icon_size,
                        margin: small_sp,
                    },
                )
            };

            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(colors.bg_float)
                        .corner_radius(egui::CornerRadius::same(corner_r as u8))
                        .inner_margin(egui::Margin::same(0)),
                )
                .show(ctx, |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), browser_h),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin::same(margin as i8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal_centered(|ui| {
                                        let back = render_icon(ui, Icon::ArrowLeft);
                                        check_hover(&back, t("browser.back"));
                                        if back.clicked() {
                                            actions.push(UiAction::WebviewBack(0));
                                        }

                                        let fwd = render_icon(ui, Icon::ArrowRight);
                                        check_hover(&fwd, t("browser.forward"));
                                        if fwd.clicked() {
                                            actions.push(UiAction::WebviewForward(0));
                                        }

                                        let refresh = render_icon(ui, Icon::Refresh);
                                        check_hover(&refresh, t("browser.refresh"));
                                        if refresh.clicked() {
                                            actions.push(UiAction::WebviewRefresh(0));
                                        }

                                        let url_resp = ui.add(
                                            egui::TextEdit::singleline(&mut url_local)
                                                .desired_width(ui.available_width() - 170.0)
                                                .font(egui::TextStyle::Small)
                                                .clip_text(true)
                                                .min_size(egui::vec2(0.0, 22.0))
                                                .vertical_align(egui::Align::Center),
                                        );
                                        is_editing = url_resp.has_focus();
                                        if url_resp.lost_focus()
                                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                        {
                                            let nav_url = if url_local.contains("://") {
                                                url_local.clone()
                                            } else if url_local.contains('.') {
                                                format!("https://{url_local}")
                                            } else {
                                                format!(
                                                    "https://www.google.com/search?q={}",
                                                    url_local
                                                )
                                            };
                                            actions.push(UiAction::NavigateWebview(nav_url, 0));
                                        }

                                        ui.add_space(margin);

                                        let split_h = render_icon(ui, Icon::SplitHorizontal);
                                        check_hover(&split_h, t("browser.split_horizontal"));
                                        if split_h.clicked() {
                                            actions.push(UiAction::DockWebviewRight);
                                        }

                                        let split_v = render_icon(ui, Icon::SplitVertical);
                                        check_hover(&split_v, t("browser.split_vertical"));
                                        if split_v.clicked() {
                                            actions.push(UiAction::DockWebviewDown);
                                        }

                                        let tab = render_icon(ui, Icon::OpenInProduct);
                                        check_hover(&tab, t("browser.open_as_tab"));
                                        if tab.clicked() {
                                            actions.push(UiAction::WebviewToTab(0));
                                        }

                                        let ext = render_icon(ui, Icon::Globe);
                                        check_hover(&ext, t("browser.open_system"));
                                        if ext.clicked() {
                                            actions.push(UiAction::OpenInSystemBrowser);
                                        }

                                        let close = render_icon(ui, Icon::Close);
                                        check_hover(&close, t("browser.close"));
                                        if close.clicked() {
                                            actions.push(UiAction::CloseWebview(0));
                                        }
                                    });
                                });
                        },
                    );
                });

            actions
        })?;

        *url = url_local;
        *editing = is_editing;

        Ok(BrowserRenderResult {
            actions,
            tooltip,
            browser_height: browser_h,
        })
    }
}

impl Renderer for Modal {
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32, scale: f32) {
        self.overlay.resize(device, width, height, scale);
    }

    fn is_visible(&self) -> bool {
        self.panel.is_visible()
    }

    fn set_visible(&mut self, visible: bool) {
        self.panel.set_visible(visible);
    }
}
