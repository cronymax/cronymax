use std::rc::Rc;

use crate::{
    config::AppConfig,
    renderer::Renderer,
    renderer::atlas::TerminalOutput,
    ui::{
        UiState,
        widget::{Dirties, Fragment},
    },
};
// --- Shared trait ---

/// Common interface for renderer tiers that own a wgpu surface + egui context.
pub trait View {
    type Renderer: Renderer;

    fn as_renderer(&self) -> &Self::Renderer;

    fn is_visible(&self) -> bool {
        self.as_renderer().is_visible()
    }
}

pub trait ViewMut: View {
    fn as_mut_renderer(&mut self) -> &mut Self::Renderer;

    /// Build the [`PreparedFrame`] for this rendering cycle.
    fn prepare_raw_input(&mut self) -> ::egui::RawInput;

    fn manipulate_full_output(&mut self, full_output: ::egui::FullOutput) -> ::egui::FullOutput {
        full_output
    }

    #[allow(unused)]
    fn handle_platform_output(&mut self, platform_output: egui::PlatformOutput) {}

    fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.as_mut_renderer().resize(width, height, scale);
    }

    fn set_visible(&mut self, visible: bool) {
        self.as_mut_renderer().set_visible(visible);
    }

    /// Run the egui CPU pass only: prepare input → run_ui → manipulate output.
    ///
    /// Returns the `FullOutput` for the caller to inspect (e.g. read back
    /// layout rects) before handing it to [`present_frame`].
    fn run_ui<F>(&mut self, f: F) -> ::egui::FullOutput
    where
        F: FnMut(&egui::Context),
    {
        let raw_input = self.prepare_raw_input();
        let full_output = self.as_mut_renderer().run_ui(raw_input, f);
        self.manipulate_full_output(full_output)
    }

    /// GPU submit: present the composed frame and handle platform output.
    fn present(
        &mut self,
        full_output: ::egui::FullOutput,
        terminal_output: Option<TerminalOutput>,
    ) -> Result<(), wgpu::SurfaceError> {
        let platform_output = self.as_mut_renderer().present(
            full_output,
            Some(wgpu::Color::TRANSPARENT),
            None,
            terminal_output,
        )?;
        self.handle_platform_output(platform_output);
        Ok(())
    }

    /// Convenience: compose + present in one call (no terminal output).
    fn run<F>(&mut self, f: F) -> Result<(), wgpu::SurfaceError>
    where
        F: FnMut(&egui::Context),
    {
        let full_output = self.run_ui(f);
        self.present(full_output, None)
    }

    /// Run an egui frame on this overlay and present the result.
    fn render<F>(
        &mut self,
        app_config: &AppConfig,
        ui_state: &mut UiState,
        mut f: F,
    ) -> Result<Dirties, wgpu::SurfaceError>
    where
        F: FnMut(Fragment),
    {
        let mut dirties = Dirties::default();
        self.run(|c| {
            egui_extras::install_image_loaders(c);
            let colors = app_config.resolve_colors();
            let style = app_config.styles.build_egui_style(&colors);
            c.set_style(style);

            f(Fragment::new(
                c,
                Rc::new(colors),
                &app_config.styles,
                ui_state,
                &mut dirties,
            ));
        })?;
        Ok(dirties)
    }
}
