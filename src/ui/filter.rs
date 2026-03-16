//! Filter / find-in-terminal bar widget.

use super::actions::UiAction;
use super::i18n::t;
use super::widget::{Fragment, Widget};

/// Filter bar panel widget — find-in-terminal search bar.
pub struct FilterBarWidget;

impl Widget for FilterBarWidget {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Context>) {
        let ctx = f.ctx();
        let styles = f.styles;
        let ui_state = f.ui_state;
        if !ui_state.filter.open {
            return;
        }

        egui::TopBottomPanel::top("filter_bar")
            .exact_height(styles.address_bar_height())
            .frame(egui::Frame::new().inner_margin(egui::Margin::same(styles.spacing.medium as i8)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut ui_state.filter.text)
                            .hint_text(t("filter.placeholder"))
                            .desired_width(styles.typography.line_height * 10.0),
                    );

                    // Auto-focus
                    if ui_state.filter.text.is_empty() {
                        response.request_focus();
                    }

                    // Match count
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

                    // Prev / Next
                    if ui.small_button("▲").clicked() {
                        f.dirties.actions.push(UiAction::FilterPrev);
                    }
                    if ui.small_button("▼").clicked() {
                        f.dirties.actions.push(UiAction::FilterNext);
                    }

                    // Close
                    if ui.small_button("✕").clicked() {
                        f.dirties.actions.push(UiAction::FilterClose);
                    }

                    // Enter to search
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        f.dirties
                            .actions
                            .push(UiAction::FilterSearch(ui_state.filter.text.clone()));
                        response.request_focus();
                    }

                    // Escape to close
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        f.dirties.actions.push(UiAction::FilterClose);
                    }
                });
            });
    }
}
