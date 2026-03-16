use crate::{
    terminal::SessionId,
    ui::{CommandEntry, prompt::PromptState, widget::Widget},
};

/// Floating command-suggestion dropdown rendered above the input line.
pub(super) struct CommandPickerWidget<'a> {
    pub state: &'a mut PromptState,
    pub filtered_items: &'a [(usize, &'a CommandEntry)],
    pub sid: SessionId,
}

impl Widget<egui::Ui> for CommandPickerWidget<'_> {
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <egui::Ui as crate::ui::widget::Painter>::Ref<'a>,
        #[allow(unused)] mut ctx: crate::ui::widget::Context<'a>,
    ) {
        let styles = ctx.styles;

        if self.state.cmd_suggestion_idx >= self.filtered_items.len() {
            self.state.cmd_suggestion_idx = 0;
        }

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .max_height(
                (self.filtered_items.len() as f32 * styles.typography.line_height).min(150.0),
            )
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.with_layout(
                    egui::Layout::top_down(egui::Align::LEFT).with_cross_justify(true),
                    |ui| {
                        for (display_idx, (_orig_idx, cmd)) in
                            self.filtered_items.iter().enumerate()
                        {
                            ctx.bind::<egui::Ui>(ui).add(CommandPickerRow {
                                state: self.state,
                                sid: self.sid,
                                display_idx,
                                cmd,
                            });
                        }
                    },
                );
            });
    }
}

struct CommandPickerRow<'a> {
    state: &'a mut PromptState,
    sid: SessionId,
    display_idx: usize,
    cmd: &'a CommandEntry,
}

impl Widget<egui::Ui> for CommandPickerRow<'_> {
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <egui::Ui as crate::ui::widget::Painter>::Ref<'a>,
        #[allow(unused)] mut ctx: crate::ui::widget::Context<'a>,
    ) {
        let is_selected = self.display_idx == self.state.cmd_suggestion_idx;
        let Self {
            cmd,
            display_idx,
            state: editor,
            ..
        } = self;

        let styles = ctx.styles;
        let colors = &ctx.colors;

        let item_id = egui::Id::new("cmd_suggestion_item").with(display_idx);
        let hover_t = ui
            .ctx()
            .animate_bool_with_time(item_id.with("hover"), is_selected, 0.15);

        let inner_response = egui::Frame::new()
            .fill(colors.fill_active.gamma_multiply(hover_t))
            .corner_radius(styles.radii.sm - 1.0)
            .inner_margin(egui::Margin::symmetric(
                styles.spacing.medium as i8,
                styles.spacing.small as i8,
            ))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let text_col = if is_selected {
                        colors.text_title
                    } else {
                        colors.text_caption
                    };
                    let lbl = ui.add(
                        egui::Label::new(egui::RichText::new(&cmd.label).color(text_col).small())
                            .sense(egui::Sense::click()),
                    );
                    if let Some(sc) = &cmd.shortcut {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new(sc).small().color(colors.text_caption));
                        });
                    }
                    if lbl.clicked() {
                        if cmd.needs_arg {
                            editor.text = format!(":{} ", cmd.action);
                        } else {
                            let cmd_text = format!(":{}", cmd.action);
                            editor.text.clear();
                            editor.cmd_suggestion_idx = 0;
                            ctx.dirties.commands.push((self.sid, cmd_text));
                        }
                    }
                });
            });
        if is_selected {
            inner_response.response.scroll_to_me(None);
        }
    }
}
