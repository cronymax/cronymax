use std::sync::{Arc, Mutex};

use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::renderer::overlay::Overlay;
use crate::renderer::panel::{LogicalRect, Panel, PanelAttrs};
use crate::ui::{View, ViewMut};

/// Tier 2: A focusable child window that floats above native webviews.
///
/// Used for browser overlay view and the Settings panel.
/// Wraps an [`OverlayWindow`] — the unified child window model.
/// Creates its own [`ChildPanel`] with modal configuration (focusable,
/// shadow, event monitoring).
pub struct Modal {
    pub ow: Overlay,
}

impl std::ops::Deref for Modal {
    type Target = Overlay;
    fn deref(&self) -> &Self::Target {
        &self.ow
    }
}
impl std::ops::DerefMut for Modal {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ow
    }
}

impl Modal {
    pub fn new(
        parent: &Window,
        event_loop: Option<&ActiveEventLoop>,
        gpu: &crate::renderer::GpuContext,
        rect: LogicalRect,
    ) -> Result<Self, String> {
        let attrs = PanelAttrs {
            shadow: true,
            focusable: true,
            click_through: false,
            level_offset: 0,
            initially_visible: true,
            corner_radius: 8.0,
            opaque: false,
        };
        let mut panel = Panel::new(parent, event_loop, rect, attrs)?;
        panel.install_event_monitor(rect.h);
        let width = (rect.w * rect.scale).round() as u32;
        let height = (rect.h * rect.scale).round() as u32;
        let ow = Overlay::new(panel, gpu, width.max(1), height.max(1), rect.scale)?;
        Ok(Self { ow })
    }

    pub fn set_frame(&mut self, parent: &Window, rect: LogicalRect) {
        self.panel.set_frame_logical(parent, rect);
    }

    pub fn event_buffer(&self) -> &Arc<Mutex<Vec<egui::Event>>> {
        &self.panel.event_buffer
    }
}

impl View for Modal {
    type Renderer = Overlay;
    fn as_renderer(&self) -> &Self::Renderer {
        &self.ow
    }
}

impl ViewMut for Modal {
    fn as_mut_renderer(&mut self) -> &mut Self::Renderer {
        &mut self.ow
    }
    fn prepare_raw_input(&mut self) -> egui::RawInput {
        // Drain buffered events before the mutable borrow in render_prepared().
        let events: Vec<egui::Event> = self
            .ow
            .panel
            .event_buffer
            .lock()
            .map(|mut buf| buf.drain(..).collect())
            .unwrap_or_default();

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
            events,
            ..Default::default()
        }
    }

    fn manipulate_full_output(&mut self, full_output: egui::FullOutput) -> egui::FullOutput {
        let output = &full_output.platform_output;
        // Handle clipboard output from standalone egui context.
        for cmd in &output.commands {
            if let egui::OutputCommand::CopyText(text) = cmd {
                crate::renderer::terminal::input::copy_to_clipboard(text);
            }
        }
        #[allow(deprecated)]
        if !output.copied_text.is_empty() {
            crate::renderer::terminal::input::copy_to_clipboard(&output.copied_text);
        }

        full_output
    }
}
