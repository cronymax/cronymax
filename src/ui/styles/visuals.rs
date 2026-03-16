use super::{Styles, colors::Colors};

/// Replace the alpha channel of a [`Color32`], keeping its RGB.
#[allow(dead_code)]
fn alpha(c: egui::Color32, a: u8) -> egui::Color32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Shorthand for [`Color32::from_rgba_unmultiplied`].
#[allow(dead_code)]
fn rgba(r: u8, g: u8, b: u8, a: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

impl Styles {
    pub fn titlebar_height(&self) -> f32 {
        self.typography.title3 * 1.5 + self.spacing.medium * 2.0
    }

    pub fn address_bar_height(&self) -> f32 {
        self.typography.line_height + self.spacing.medium * 2.0
    }

    pub fn suggestion_row_height(&self) -> f32 {
        self.typography.body0 + self.spacing.medium + self.spacing.small
    }

    pub fn tab_bar_height(&self) -> f32 {
        self.typography.line_height + self.spacing.small * 2.0
    }

    pub fn browser_view_tab_width(&self) -> f32 {
        self.typography.line_height * 10.0
    }

    pub fn addr_btn_width(&self) -> f32 {
        self.typography.line_height + self.spacing.medium + self.spacing.small
    }

    pub fn addr_btn_total(&self) -> f32 {
        self.addr_btn_width() * 3.0 + self.spacing.small / 2.0
    }

    pub fn browser_view_tab_entry_height(&self) -> f32 {
        self.typography.line_height * 2.0
    }
}

impl Styles {
    /// Build `egui::Visuals` from the Lark/Feishu design tokens.
    ///
    /// Mode-aware: produces correct fills, backgrounds, and text for both
    /// dark and light themes.
    pub fn build_egui_visuals(&self, colors: &Colors) -> egui::Visuals {
        if let Some(ref vis) = self.visuals {
            return vis.clone();
        }

        let Self {
            radii,
            sizes,
            shadows,
            ..
        } = self;
        let egui_base = if colors.darked {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let rounding = egui::CornerRadius::same(radii.sm as u8);
        let expansion = 1.0;

        egui::Visuals {
            dark_mode: colors.darked,
            override_text_color: Some(colors.text_title),
            window_fill: colors.bg_body,
            panel_fill: colors.bg_base,
            window_corner_radius: egui::CornerRadius::same(radii.lg as u8),
            extreme_bg_color: colors.bg_base,
            faint_bg_color: colors.bg_mask,

            widgets: egui::style::Widgets {
                noninteractive: egui::style::WidgetVisuals {
                    weak_bg_fill: egui::Color32::TRANSPARENT,
                    bg_fill: egui::Color32::TRANSPARENT,
                    bg_stroke: egui::Stroke::new(sizes.border, colors.divider),
                    fg_stroke: egui::Stroke::new(sizes.border, colors.text_title),
                    corner_radius: rounding,
                    expansion,
                },
                inactive: egui::style::WidgetVisuals {
                    weak_bg_fill: colors.fill_tag,
                    bg_fill: colors.fill_tag,
                    bg_stroke: egui::Stroke::NONE,
                    fg_stroke: egui::Stroke::new(sizes.border, colors.text_title),
                    corner_radius: rounding,
                    expansion,
                },
                hovered: egui::style::WidgetVisuals {
                    weak_bg_fill: colors.fill_hover,
                    bg_fill: colors.fill_hover,
                    bg_stroke: egui::Stroke::NONE,
                    fg_stroke: egui::Stroke::new(sizes.border, colors.text_title),
                    corner_radius: rounding,
                    expansion,
                },
                active: egui::style::WidgetVisuals {
                    weak_bg_fill: colors.fill_active,
                    bg_fill: colors.fill_active,
                    bg_stroke: egui::Stroke::NONE,
                    fg_stroke: egui::Stroke::new(sizes.border, colors.text_title),
                    corner_radius: rounding,
                    expansion,
                },
                open: egui::style::WidgetVisuals {
                    weak_bg_fill: colors.fill_selected,
                    bg_fill: colors.fill_selected,
                    bg_stroke: egui::Stroke::NONE,
                    fg_stroke: egui::Stroke::new(sizes.border, colors.text_title),
                    corner_radius: rounding,
                    expansion,
                },
            },

            window_shadow: egui::Shadow {
                offset: [0, 4],
                blur: shadows.large as u8,
                spread: 2,
                color: if colors.darked {
                    egui::Color32::from_black_alpha(89)
                } else {
                    egui::Color32::from_black_alpha(31)
                },
            },
            popup_shadow: egui::Shadow {
                offset: [0, 2],
                blur: shadows.medium as u8,
                spread: 1,
                color: if colors.darked {
                    egui::Color32::from_black_alpha(77)
                } else {
                    egui::Color32::from_black_alpha(20)
                },
            },

            selection: egui::style::Selection {
                bg_fill: colors.fill_selected,
                stroke: egui::Stroke::new(sizes.border, colors.text_title),
            },

            text_cursor: egui::style::TextCursorStyle {
                stroke: egui::Stroke::new(2.0, colors.text_title),
                blink: true,
                ..egui_base.text_cursor
            },

            ..egui_base
        }
    }
}
