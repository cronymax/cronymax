//! Channel pane widget — `ChannelPane` struct + rendering methods.

use crate::ui::styles::colors::Colors;
use crate::ui::widget::{Fragment, Widget};

use super::FrameState;
use super::overlays::format_timestamp;

/// Stateful widget for rendering a channel conversation pane.
///
/// Persists across frames in `PaneWidgetStore::channel`.
pub struct ChannelPane {
    pub channel_id: String,
    pub channel_name: String,
}

impl ChannelPane {
    pub fn new(channel_id: String, channel_name: String) -> Self {
        Self {
            channel_id,
            channel_name,
        }
    }

    /// Render the channel header bar with connection status dot.
    fn render_header(
        &self,
        ui: &mut egui::Ui,
        full_rect: egui::Rect,
        styles: &crate::ui::styles::Styles,
        colors: &Colors,
        state: &FrameState<'_>,
    ) -> f32 {
        let conn_state = state.channel_connection_state;
        let header_h = styles.typography.title3 + styles.spacing.medium * 2.0;
        let header_rect =
            egui::Rect::from_min_size(full_rect.min, egui::vec2(full_rect.width(), header_h));
        ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(header_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.add_space(styles.spacing.medium);

                let (dot_color, status_label) = match conn_state {
                    crate::channel::ConnectionState::Connected => (colors.success, "Connected"),
                    crate::channel::ConnectionState::Connecting => {
                        (colors.warning, "Connecting...")
                    }
                    crate::channel::ConnectionState::Reconnecting => {
                        (colors.warning, "Reconnecting...")
                    }
                    crate::channel::ConnectionState::Error => (colors.danger, "Error"),
                    crate::channel::ConnectionState::Disconnected => {
                        (colors.warning, "Disconnected")
                    }
                };
                let dot_center = egui::pos2(ui.cursor().min.x + 5.0, header_rect.center().y);
                ui.painter()
                    .circle_filled(dot_center, styles.radii.md, dot_color);
                ui.add_space(styles.spacing.large);

                ui.colored_label(
                    colors.primary,
                    egui::RichText::new(format!("# {}", self.channel_name))
                        .size(styles.typography.body0)
                        .strong(),
                );
                ui.colored_label(
                    colors.text_caption,
                    egui::RichText::new(format!(" — {status_label}")).size(styles.typography.body2),
                );
            },
        );

        // Separator line
        let sep_y = full_rect.min.y + header_h;
        ui.painter().line_segment(
            [
                egui::pos2(full_rect.min.x, sep_y),
                egui::pos2(full_rect.max.x, sep_y),
            ],
            egui::Stroke::new(styles.sizes.border, colors.border),
        );

        header_h
    }

    /// Render a single channel message as a chat-style bubble.
    fn render_message(
        &self,
        ui: &mut egui::Ui,
        msg: &crate::channel::ChannelDisplayMessage,
        avail_w: f32,
        styles: &crate::ui::styles::Styles,
        colors: &Colors,
    ) {
        let padding = styles.spacing.medium;
        let bubble_max_w = (avail_w * 0.75).max(100.0);

        ui.add_space(styles.spacing.medium);

        if msg.is_outgoing {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                ui.add_space(padding);
                let bubble_color = colors.primary;
                egui::Frame::new()
                    .fill(bubble_color)
                    .corner_radius(egui::CornerRadius {
                        nw: styles.spacing.large as _,
                        ne: styles.spacing.small as _,
                        sw: styles.spacing.large as _,
                        se: styles.spacing.large as _,
                    })
                    .inner_margin(egui::Margin::symmetric(
                        styles.spacing.large as _,
                        styles.spacing.medium as i8,
                    ))
                    .show(ui, |ui| {
                        ui.set_max_width(bubble_max_w);
                        ui.colored_label(
                            colors.text_disabled,
                            egui::RichText::new("Bot").size(styles.typography.body2),
                        );
                        ui.colored_label(
                            colors.text_title,
                            egui::RichText::new(&msg.content).size(styles.typography.body0),
                        );
                        let time_str = format_timestamp(msg.timestamp);
                        ui.colored_label(
                            colors.text_disabled,
                            egui::RichText::new(time_str).size(styles.typography.body2),
                        );
                    });
            });
        } else {
            ui.horizontal(|ui| {
                ui.add_space(padding);
                let bubble_color = colors.border;
                egui::Frame::new()
                    .fill(bubble_color)
                    .corner_radius(egui::CornerRadius {
                        nw: styles.spacing.small as _,
                        ne: styles.spacing.large as _,
                        sw: styles.spacing.large as _,
                        se: styles.spacing.large as _,
                    })
                    .inner_margin(egui::Margin::symmetric(
                        styles.spacing.large as _,
                        styles.spacing.medium as i8,
                    ))
                    .show(ui, |ui| {
                        ui.set_max_width(bubble_max_w);
                        ui.colored_label(
                            colors.primary,
                            egui::RichText::new(&msg.sender)
                                .size(styles.typography.body2)
                                .strong(),
                        );
                        ui.colored_label(
                            colors.text_title,
                            egui::RichText::new(&msg.content).size(styles.typography.body2),
                        );
                        let time_str = format_timestamp(msg.timestamp);
                        ui.colored_label(
                            colors.text_disabled,
                            egui::RichText::new(time_str).size(styles.typography.body2),
                        );
                    });
            });
        }

        ui.add_space(styles.spacing.small);
    }
}

/// Temporary view adapting `ChannelPane` to `Widget<egui::Ui>`.
pub struct ChannelPaneView<'w, 'f> {
    pub widget: &'w mut ChannelPane,
    pub state: &'w mut FrameState<'f>,
}

impl Widget<egui::Ui> for ChannelPaneView<'_, '_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Ui>) {
        let ui = &mut *f.painter;
        let styles = f.styles;
        let colors = &*f.colors;
        let full_rect = ui.available_rect_before_wrap();

        ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(full_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
            |ui| {
                ui.set_clip_rect(full_rect);

                let header_h = self
                    .widget
                    .render_header(ui, full_rect, styles, colors, self.state);

                // Message area (scrollable)
                let content_rect = egui::Rect::from_min_max(
                    egui::pos2(full_rect.min.x, full_rect.min.y + header_h + 1.0),
                    full_rect.max,
                );
                let content_h = content_rect.height();
                let messages = self.state.channel_messages.get(&self.widget.channel_id);

                ui.allocate_new_ui(
                    egui::UiBuilder::new()
                        .max_rect(content_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    |ui| {
                        ui.set_clip_rect(content_rect);
                        let has_msgs = messages.is_some_and(|m| !m.is_empty());
                        egui::ScrollArea::vertical()
                            .id_salt(egui::Id::new("channel_scroll").with(&self.widget.channel_id))
                            .max_height(content_h)
                            .stick_to_bottom(has_msgs)
                            .show(ui, |ui| {
                                let avail_w = content_rect.width() - styles.spacing.medium * 2.0;
                                ui.set_width(avail_w);
                                ui.add_space(styles.spacing.small);
                                if has_msgs {
                                    for msg in messages.unwrap() {
                                        self.widget
                                            .render_message(ui, msg, avail_w, styles, colors);
                                    }
                                } else {
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(styles.spacing.large * 3.0);
                                        ui.colored_label(
                                            colors.text_disabled,
                                            egui::RichText::new("No messages yet")
                                                .size(styles.typography.body0),
                                        );
                                        ui.add_space(styles.spacing.small);
                                        ui.colored_label(
                                            colors.text_disabled,
                                            egui::RichText::new(
                                                "Messages will appear here as they arrive.",
                                            )
                                            .size(styles.typography.body2),
                                        );
                                    });
                                }

                                ui.add_space(styles.spacing.medium);
                            });
                    },
                );
            },
        );
    }
}
