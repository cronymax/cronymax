/// App-level semantic colours — rebuilt by `apply_system_theme()`.
///
/// Status colours (`primary` … `danger`) are user-facing brand colours.
/// Text / surface / section colours are derived from the active dark/light palette.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Colors {
    pub darked: bool,

    // ── Brand ──
    pub primary: egui::Color32,
    pub secondary: egui::Color32,

    // ── Functions ──
    pub info: egui::Color32,
    pub success: egui::Color32,
    pub warning: egui::Color32,
    pub danger: egui::Color32,

    // Lines
    pub border: egui::Color32,
    pub divider: egui::Color32,

    // Fillings
    pub fill_active: egui::Color32,
    pub fill_disabled: egui::Color32,
    pub fill_focus: egui::Color32,
    pub fill_hover: egui::Color32,
    pub fill_pressed: egui::Color32,
    pub fill_selected: egui::Color32,
    pub fill_tag: egui::Color32,

    // Backgrounds
    pub bg_base: egui::Color32,
    pub bg_body: egui::Color32,
    pub bg_float: egui::Color32,
    pub bg_mask: egui::Color32,

    // Texts
    pub text_title: egui::Color32,
    pub text_caption: egui::Color32,
    pub text_disabled: egui::Color32,
    pub text_placeholder: egui::Color32,
}

impl Default for Colors {
    fn default() -> Self {
        if crate::renderer::platform::is_dark_mode() {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

fn rgb(r: u8, g: u8, b: u8) -> egui::Color32 {
    egui::Color32::from_rgb(r, g, b)
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(r, g, b, (a * 255.0) as _)
}

impl Colors {
    pub fn dark() -> Colors {
        Colors {
            darked: true,
            // ── Brand ──
            primary: rgb(45, 212, 191),
            secondary: rgb(20, 86, 240),

            // ── Functions ──
            info: rgb(76, 136, 255),
            success: rgb(81, 186, 67),
            warning: rgb(243, 135, 27),
            danger: rgb(240, 91, 86),

            border: rgba(235, 235, 235, 0.15),
            divider: rgba(207, 207, 207, 0.15),

            bg_body: rgb(26, 26, 26),
            bg_base: rgb(10, 10, 10),
            bg_float: rgb(41, 41, 41),
            bg_mask: rgba(0, 0, 0, 0.6),

            // fill_active: rgba(76, 136, 255, 0.2),
            fill_active: rgba(45, 212, 191, 0.2),
            fill_disabled: rgb(95, 95, 95),
            fill_focus: rgba(235, 235, 235, 0.12),
            fill_hover: rgba(235, 235, 235, 0.08),
            fill_pressed: rgba(235, 235, 235, 0.12),
            // fill_selected: rgba(76, 136, 255, 0.15),
            fill_selected: rgba(45, 212, 191, 0.15),
            fill_tag: rgba(235, 235, 235, 0.1),

            text_title: rgb(235, 235, 235),
            text_caption: rgb(166, 166, 166),
            text_disabled: rgb(95, 95, 95),
            text_placeholder: rgb(117, 117, 117),
        }
    }
    pub fn light() -> Colors {
        Colors {
            darked: false,
            // ── Brand ──
            primary: rgb(13, 148, 136),
            secondary: rgb(20, 86, 240),

            // ── Functions ──
            info: rgb(20, 86, 240),
            success: rgb(50, 166, 69),
            warning: rgb(237, 109, 12),
            danger: rgb(245, 74, 69),

            // Lines
            border: rgb(222, 224, 227),
            divider: rgba(31, 35, 41, 0.15),

            // Backgrounds
            bg_base: rgb(242, 243, 245),
            bg_body: rgb(255, 255, 255),
            bg_float: rgb(255, 255, 255),
            bg_mask: rgba(0, 0, 0, 0.55),

            // Fillings
            // fill_active: rgba(20, 86, 240, 0.15),
            fill_active: rgba(13, 148, 136, 0.15),
            fill_disabled: rgb(187, 191, 196),
            fill_focus: rgba(31, 35, 41, 0.12),
            fill_hover: rgba(31, 35, 41, 0.08),
            fill_pressed: rgba(31, 35, 41, 0.12),
            // fill_selected: rgba(20, 86, 240, 0.1),
            fill_selected: rgba(13, 148, 136, 0.1),
            fill_tag: rgba(31, 35, 41, 0.1),

            // Texts
            text_title: rgb(31, 35, 41),
            text_caption: rgb(100, 106, 115),
            text_disabled: rgb(187, 191, 196),
            text_placeholder: rgb(143, 149, 158),
        }
    }
}
