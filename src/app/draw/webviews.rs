//! Webview positioning during redraw, extracted from draw.rs

use crate::app::*;

/// Reposition overlay webviews and re-stack z-order during redraw.
pub(super) fn reposition_overlays(state: &mut AppState, scale: f32) {
    // ── Reposition overlay webviews using the actual egui rect ──
    // The child panel covers the FULL overlay content inside
    // borders (address bar + webview area).  An Modal surface
    // renders the address bar on the panel's Metal layer;
    // the WKWebView sits below it as a subview.
    if let Some([px, py, pw, ph]) = state.ui_state.overlay_panel_rect {
        let bw = state.styles.sizes.border;
        let address_bar_h = state.styles.address_bar_height();

        // Panel = content inside the egui border strokes.
        let panel_lx = px + bw;
        let panel_ly = py + bw;
        let panel_lw = pw - 2.0 * bw;
        let panel_lh = ph - 2.0 * bw;

        let phys_w = (panel_lw * scale).round() as u32;
        let panel_phys_h = (panel_lh * scale).round() as u32;
        let browser_phys_h = (address_bar_h * scale).round() as u32;
        let wv_phys_h = panel_phys_h.saturating_sub(browser_phys_h);

        // Content-level fallback bounds (for non-panel webviews).
        let content_phys = Bounds {
            x: (panel_lx * scale).round() as u32,
            y: ((panel_ly + address_bar_h) * scale).round() as u32,
            width: phys_w,
            height: wv_phys_h,
        };

        for wt in &mut state.webview_tabs {
            if wt.mode == BrowserViewMode::Overlay && wt.manager.visible {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                let has_overlay_panel = wt.manager.overlay.is_some();
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                let has_overlay_panel = false;

                if has_overlay_panel {
                    // Set webview bounds first (avoids double &mut borrow).
                    wt.manager
                        .set_bounds(Bounds::new(0, browser_phys_h, phys_w, wv_phys_h));
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    if let Some(overlay) = &mut wt.manager.overlay {
                        overlay.set_frame(
                            &state.window,
                            panel_lx,
                            panel_ly,
                            panel_lw,
                            panel_lh,
                            scale,
                        );
                        // Resize the overlay surface to match.
                        use crate::renderer::overlay::Renderer;
                        overlay.resize(
                            &state.gpu.device,
                            phys_w.max(1),
                            panel_phys_h.max(1),
                            scale,
                        );
                    }
                } else {
                    // Fallback: child-of-window webview (no overlay panel).
                    wt.manager.set_bounds(content_phys);
                }
            }
        }
    }

    // On Linux (no child panel support), hide docked webviews
    // when an overlay is visible so the main-surface egui address
    // bar fallback is exposed.  macOS and Windows use child panels
    // that float above docked views, so docked views stay visible.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let has_overlay = state
            .webview_tabs
            .iter()
            .any(|wt| wt.mode == BrowserViewMode::Overlay && wt.manager.visible);
        if has_overlay {
            for wt in &mut state.webview_tabs {
                if wt.mode == BrowserViewMode::Docked && wt.manager.visible {
                    wt.manager.set_visible(false);
                }
            }
        }
    }

    // ── Re-stack overlay/independent child windows by z-order ──
    // Iterate the overlay_z_stack (bottom→top) and re-order the
    // platform child windows so the topmost overlay is in front.
    {
        let z_stack: Vec<WebviewId> = state.webview_manager.overlay_stack().to_vec();
        for wid in &z_stack {
            if let Some(wt) = state
                .webview_tabs
                .iter()
                .find(|wt| wt.id == *wid && wt.manager.visible)
            {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                if let Some(overlay) = &wt.manager.overlay {
                    overlay.panel.set_visible(true); // orderFront restacks
                }
            }
        }

        // Float must stay above all overlay ModalPanels.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(ref fr) = state.float_renderer {
            fr.ensure_above_overlays();
        }
    }
}
