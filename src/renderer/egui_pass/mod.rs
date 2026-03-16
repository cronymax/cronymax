//! Custom egui → wgpu 28 renderer.
//!
//! Replaces egui-wgpu (which requires wgpu 24) with a direct wgpu 28 render
//! pipeline for egui's tessellated primitives.

use std::collections::HashMap;

/// Integration layer: egui context + winit event adaptor + custom wgpu pipeline.
pub struct EguiIntegration {
    pub ctx: egui::Context,
    state: Option<egui_winit::State>,
    renderer: EguiRenderer,
}

impl EguiIntegration {
    /// Create a new integration for the given window and GPU context.
    pub fn new(
        window: &winit::window::Window,
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let ctx = egui::Context::default();

        // ── Load system CJK font for Chinese/Japanese/Korean support ──
        Self::install_cjk_font(&ctx);

        let viewport_id = ctx.viewport_id();
        let state = egui_winit::State::new(ctx.clone(), viewport_id, window, None, None, None);
        let renderer = EguiRenderer::new(device, surface_format);
        Self {
            ctx,
            state: Some(state),
            renderer,
        }
    }

    /// Create a standalone integration without a winit window.
    ///
    /// Used for child windows (NSPanel / owned popup) that don't have a
    /// winit `Window` but still need egui rendering.  Events must be
    /// fed manually via `egui::RawInput`.
    pub fn new_standalone(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        ctx: egui::Context,
    ) -> Self {
        let renderer = EguiRenderer::new(device, surface_format);
        Self {
            ctx,
            state: None,
            renderer,
        }
    }

    /// Load a CJK font into a standalone egui context (no window required).
    pub fn install_cjk_font_standalone(ctx: &egui::Context) {
        Self::install_cjk_font(ctx);
    }

    /// Try to load a system CJK font and install it as a fallback for both
    /// Proportional and Monospace families. This enables Chinese, Japanese,
    /// and Korean text rendering in egui (TextEdit, Labels, CommonMark, etc.).
    fn install_cjk_font(ctx: &egui::Context) {
        let cjk_paths: &[&str] = &[
            // macOS
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            // Linux — Noto Sans CJK
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            // Linux — WenQuanYi
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            // Linux — Droid Sans Fallback
            "/usr/share/fonts/truetype/droid/DroidSansFallback.ttf",
            // Windows
            "C:\\Windows\\Fonts\\msyh.ttc",   // Microsoft YaHei
            "C:\\Windows\\Fonts\\simsun.ttc", // SimSun
        ];

        for path in cjk_paths {
            if let Ok(data) = std::fs::read(path) {
                log::info!("Loaded CJK font: {}", path);
                let mut fonts = egui::FontDefinitions::default();
                fonts.font_data.insert(
                    "cjk_fallback".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_owned(data)),
                );
                // Append CJK font as the LAST fallback for both families.
                if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                    family.push("cjk_fallback".to_owned());
                }
                if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                    family.push("cjk_fallback".to_owned());
                }
                ctx.set_fonts(fonts);
                return;
            }
        }
        log::warn!("No system CJK font found — Chinese/Japanese/Korean text may not render");
    }

    /// Forward a winit event to egui.
    ///
    /// Returns `true` when egui consumed the event **or** needs a repaint
    /// (e.g. pointer-move that updates hover state).  The caller should
    /// call `window.request_redraw()` when this returns `true`.
    pub fn on_window_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) -> bool {
        if let Some(state) = &mut self.state {
            let response = state.on_window_event(window, event);
            response.consumed || response.repaint
        } else {
            false
        }
    }

    /// Begin an egui frame, run the UI closure, then end the frame and return
    /// paint jobs + textures delta ready for `render()`.
    /// Returns (primitives, textures_delta, open_url) where open_url
    /// is set when the user clicks a hyperlink in egui (e.g. in markdown).
    pub fn run(
        &mut self,
        window: &winit::window::Window,
        mut run_ui: impl FnMut(&egui::Context),
    ) -> (
        Vec<egui::ClippedPrimitive>,
        egui::TexturesDelta,
        Option<String>,
    ) {
        let mut raw_input = self
            .state
            .as_mut()
            .expect("run() requires winit State — use ctx.run() for standalone")
            .take_egui_input(window);

        // Strip Tab key events from raw input BEFORE ctx.run().
        // egui processes Tab for focus-cycling in begin_pass() which runs before
        // any widget code.  Removing Tab here prevents focus from moving away
        // from the prompt TextEdit so path-completion / suggestion-select works.
        let tab_pressed = raw_input.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    ..
                }
            )
        });
        if tab_pressed {
            raw_input.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Tab,
                        ..
                    }
                )
            });
        }

        let full_output = self.ctx.run(raw_input, |ctx| {
            // Store tab_pressed as per-frame temp data so widgets can read it.
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new("__global_tab_pressed"), tab_pressed);
            });
            run_ui(ctx);
        });
        // Extract open_url from both the new commands API and the deprecated field
        // so we can redirect link clicks to the built-in browser instead of the system one.
        let mut platform_output = full_output.platform_output;

        // Check the new `commands` vector first (egui 0.31+ uses OutputCommand::OpenUrl).
        let mut open_url: Option<String> = None;
        platform_output.commands.retain(|cmd| {
            if let egui::OutputCommand::OpenUrl(ou) = cmd {
                open_url = Some(ou.url.clone());
                false // remove so egui-winit doesn't also open system browser
            } else {
                true
            }
        });
        // Fallback: also check the deprecated field.
        if open_url.is_none() {
            #[allow(deprecated)]
            if let Some(ou) = platform_output.open_url.take() {
                open_url = Some(ou.url);
            }
        }

        if let Some(state) = &mut self.state {
            state.handle_platform_output(window, platform_output);
        }
        let primitives = self
            .ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        (primitives, full_output.textures_delta, open_url)
    }

    /// Upload texture changes, then render egui primitives into the given render pass.
    pub fn render(&mut self, args: EguiRenderArgs<'_>) {
        // Upload / update textures first
        for (id, delta) in &args.textures_delta.set {
            self.renderer
                .update_texture(args.device, args.queue, *id, delta);
        }

        self.renderer.render(
            args.device,
            args.queue,
            args.encoder,
            args.color_target,
            args.primitives,
            &args.screen_descriptor,
        );

        // Free released textures
        for id in &args.textures_delta.free {
            self.renderer.free_texture(*id);
        }
    }

    /// Whether egui wants keyboard focus (i.e. an input field is active).
    pub fn wants_keyboard_input(&self) -> bool {
        self.ctx.wants_keyboard_input()
    }

    /// Whether egui wants pointer events.
    pub fn wants_pointer_input(&self) -> bool {
        self.ctx.wants_pointer_input()
    }
}

// ─── Egui Render Args ────────────────────────────────────────────────────────

/// Bundles the arguments for [`EguiIntegration::render`].
pub struct EguiRenderArgs<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub color_target: &'a wgpu::TextureView,
    pub primitives: &'a [egui::ClippedPrimitive],
    pub textures_delta: &'a egui::TexturesDelta,
    pub screen_descriptor: ScreenDescriptor,
}

// ─── Screen Descriptor ───────────────────────────────────────────────────────

/// Physical screen dimensions for computing clip rects and NDC transform.
#[derive(Clone, Copy)]
pub struct ScreenDescriptor {
    pub width_px: u32,
    pub height_px: u32,
    pub pixels_per_point: f32,
}

mod renderer;
use renderer::EguiRenderer;
