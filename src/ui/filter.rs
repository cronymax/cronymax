//! Filter / find-in-terminal bar widget.

use super::actions::UiAction;
use super::i18n::t;
use super::widget::{Fragment, Widget};

/// Filter bar panel widget — find-in-terminal search bar.
///
/// Rendered as a floating overlay pinned to the top-right of the content area
/// (below the tab bar), rather than a full-width panel.
pub struct FilterBarWidget;

impl Widget for FilterBarWidget {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Context>) {
        let ctx = f.ctx();
        let styles = f.styles;
        let ui_state = f.ui_state;
        if !ui_state.filter.open {
            return;
        }

        let screen = ctx.screen_rect();
        let bar_width = 340.0_f32.min(screen.width() - 40.0);
        let bar_x = screen.max.x - bar_width - styles.spacing.medium;
        // Position below the titlebar panel (which includes our custom chrome).
        // The egui_tiles tab bar is rendered inside the CentralPanel immediately
        // below the titlebar, so adding tab_bar_height puts us just under the tabs.
        let bar_y = styles.titlebar_height() + styles.tab_bar_height() + styles.spacing.small;

        let area = egui::Area::new(egui::Id::new("filter_bar_overlay"))
            .fixed_pos(egui::Pos2::new(bar_x, bar_y))
            .order(egui::Order::Foreground)
            .interactable(true);

        area.show(ctx, |ui| {
            let frame = egui::Frame::new()
                .fill(f.colors.bg_float)
                .corner_radius(styles.radii.md)
                .inner_margin(egui::Margin::symmetric(
                    styles.spacing.medium as i8,
                    styles.spacing.small as i8,
                ))
                .stroke(egui::Stroke::new(1.0, f.colors.border))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 2],
                    blur: 8,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(40),
                });

            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut ui_state.filter.text)
                            .hint_text(t("filter.placeholder"))
                            .desired_width(bar_width - 120.0),
                    );

                    // Auto-focus when opened.
                    if ui_state.filter.text.is_empty() {
                        response.request_focus();
                    }

                    // Match count.
                    let match_label = if ui_state.filter.match_count > 0 {
                        format!(
                            "{}/{}",
                            ui_state.filter.current_match + 1,
                            ui_state.filter.match_count
                        )
                    } else if ui_state.filter.text.is_empty() {
                        String::new()
                    } else {
                        t("filter.no_matches").into()
                    };
                    ui.weak(&match_label);

                    // Prev / Next.
                    if ui.small_button("▲").clicked() {
                        f.dirties.actions.push(UiAction::FilterPrev);
                    }
                    if ui.small_button("▼").clicked() {
                        f.dirties.actions.push(UiAction::FilterNext);
                    }

                    // Close button.
                    if ui.small_button("✕").clicked() {
                        f.dirties.actions.push(UiAction::FilterClose);
                    }

                    // Enter to search.
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        f.dirties
                            .actions
                            .push(UiAction::FilterSearch(ui_state.filter.text.clone()));
                        response.request_focus();
                    }

                    // Escape to close.
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        f.dirties.actions.push(UiAction::FilterClose);
                    }
                });
            });
        });
    }
}
