//! Informational message block — italic, muted, no prompt marker or star.

use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

/// Info block widget — renders a plain informational message.
pub struct InfoBlock;

impl InfoBlock {
    pub fn render(
        ui: &mut egui::Ui,
        cell_id: u32,
        text: &str,
        styles: &Styles,
        _colors: &Colors,
    ) -> egui::Response {
        ui.set_min_width(ui.available_width());
        let resp_id = ui.id().with("info_block").with(cell_id);
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .italics()
                    .size(styles.typography.body0),
            )
            .wrap()
            .selectable(true),
        );
        ui.interact(ui.min_rect(), resp_id, egui::Sense::hover())
    }
}
