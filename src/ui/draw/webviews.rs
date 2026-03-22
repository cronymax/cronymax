//! Webview positioning during redraw — repositions overlay browsers and
//! re-stacks z-order.

use crate::renderer::viewport::Viewport;
use crate::ui::Ui;
use crate::ui::ViewMut;
use crate::ui::model::AppCtx;
use crate::ui::types::BrowserViewMode;

impl Ui {
    /// Reposition overlay webviews and re-stack z-order during redraw.
    pub(crate) fn reposition_overlays(&mut self, ctx: &mut AppCtx<'_>, scale: f32) {
        // ── Reposition overlay webviews using the actual egui rect ──
        if let Some([px, py, pw, ph]) = ctx.ui_state.overlay_panel_rect {
            let bw = self.styles.sizes.border;
            let address_bar_h = self.styles.address_bar_height();

            let panel_lx = px + bw;
            let panel_ly = py + bw;
            let panel_lw = pw - 2.0 * bw;
            let panel_lh = ph - 2.0 * bw;

            let phys_w = (panel_lw * scale).round();
            let panel_phys_h = (panel_lh * scale).round();
            let browser_phys_h = (address_bar_h * scale).round();
            let wv_phys_h = panel_phys_h - browser_phys_h;

            let content_phys = Viewport {
                x: (panel_lx * scale).round(),
                y: ((panel_ly + address_bar_h) * scale).round(),
                width: phys_w,
                height: wv_phys_h,
            };

            for wt in &mut self.browser_tabs {
                if wt.mode == BrowserViewMode::Overlay && wt.browser.view.visible {
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    let has_overlay_panel = wt.overlay.is_some();
                    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                    let has_overlay_panel = false;

                    if has_overlay_panel {
                        wt.browser.view.set_viewport(Viewport::new(
                            0.0,
                            browser_phys_h,
                            phys_w,
                            wv_phys_h,
                        ));
                        #[cfg(any(target_os = "macos", target_os = "windows"))]
                        if let Some(overlay) = &mut wt.overlay {
                            overlay.panel.set_frame_logical(
                                &self.frame.window,
                                crate::renderer::panel::LogicalRect {
                                    x: panel_lx,
                                    y: panel_ly,
                                    w: panel_lw,
                                    h: panel_lh,
                                    scale,
                                },
                            );
                            overlay.resize(phys_w.max(1.0) as _, panel_phys_h.max(1.0) as _, scale);
                        }
                        wt.overlay_origin = (panel_lx, panel_ly);
                    } else {
                        wt.browser.view.set_viewport(content_phys);
                    }
                }
            }
        }

        // On Linux, hide docked webviews when an overlay is visible.
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let has_overlay = self
                .browser_tabs
                .iter()
                .any(|wt| wt.mode == BrowserViewMode::Overlay && wt.browser.view.visible);
            if has_overlay {
                for wt in &mut self.browser_tabs {
                    if wt.mode == BrowserViewMode::Docked && wt.browser.view.visible {
                        wt.browser.view.set_visible(false);
                    }
                }
            }
        }

        // ── Re-stack overlay/independent child windows by z-order ──
        {
            let z_stack: Vec<crate::ui::browser::BrowserId> =
                self.browser_manager.overlay_stack().to_vec();
            for wid in &z_stack {
                if let Some(wt) = self
                    .browser_tabs
                    .iter_mut()
                    .find(|wt| wt.browser.id == *wid && wt.browser.view.visible)
                {
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    if let Some(overlay) = &mut wt.overlay {
                        overlay.panel.set_visible(true);
                    }
                }
            }

            #[cfg(any(target_os = "macos", target_os = "windows"))]
            if let Some(ref fr) = self.float_renderer {
                fr.ensure_above_overlays();
            }
        }
    }
}
