use super::actions::UiAction;
use super::widget::{Fragment, Widget};

/// Relaunch-confirmation dialog for sandbox rule changes.
pub struct RelaunchDialog;

impl Widget for RelaunchDialog {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Context>) {
        let ctx = f.ctx();
        let colors = &*f.colors;
        let (ui_state, styles, dirties) = (&mut *f.ui_state, f.styles, &mut *f.dirties);

        // T031: fade animation for appear/dismiss
        let appear_t = ctx.animate_bool_with_time(
            egui::Id::new("relaunch_dialog_appear"),
            ui_state.show_profile_relaunch_dialog,
            0.2,
        );
        if appear_t <= 0.0 {
            return;
        }

        let heading_color = colors.text_title.gamma_multiply(appear_t);
        let body_color = colors.text_caption.gamma_multiply(appear_t);
        let heading_size = styles.typography.title3;
        let body_size = styles.typography.body2;
        let accent = colors.primary.gamma_multiply(appear_t);

        egui::Area::new(egui::Id::new("profile_relaunch_dialog"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Frame::new()
                    .corner_radius(egui::CornerRadius::same(styles.radii.md as u8))
                    .inner_margin(egui::Margin::same(styles.spacing.large as i8))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Sandbox rules changed")
                                .color(heading_color)
                                .size(heading_size),
                        );
                        ui.add_space(styles.spacing.small);
                        ui.label(
                            egui::RichText::new(
                                "The new profile has different sandbox rules.\n\
                             Relaunch to apply OS-level enforcement.",
                            )
                            .color(body_color)
                            .size(body_size),
                        );
                        ui.add_space(styles.spacing.medium);
                        ui.horizontal(|ui| {
                            if ui
                                .add(egui::Button::new("Later").fill(egui::Color32::TRANSPARENT))
                                .clicked()
                            {
                                ui_state.show_profile_relaunch_dialog = false;
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Relaunch Now").color(accent),
                                    )
                                    .fill(egui::Color32::TRANSPARENT),
                                )
                                .clicked()
                            {
                                ui_state.show_profile_relaunch_dialog = false;
                                dirties.actions.push(UiAction::RelaunchApp);
                            }
                        });
                    });
            });
    }
}
