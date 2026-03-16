//! Configurable theme system — Arc / Apple-inspired colour presets.
//!
//! `Tokens` (private) prebakes dark / light palettes. `apply_system_theme()`
//! derives both `Colors` (app-level semantics) and `egui::Visuals` (widget paint)
//! from the active `Tokens`.  Widget code reads `styles.colors.*` for app colours
//! and `styles.visuals.*` (via `Deref`) for standard egui widget styling.
pub mod colors;
pub mod visuals;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, smart_default::SmartDefault)]
#[serde(default)]
pub struct Styles {
    pub typography: Typography,
    pub spacing: Spacing,
    pub radii: Radii,
    pub sizes: Sizes,
    pub shadows: Shadows,
    colors: Option<colors::Colors>,
    visuals: Option<egui::Visuals>,
}

impl Styles {
    /// Access the optional user-configured colours override.
    pub(crate) fn colors_override(&self) -> Option<&colors::Colors> {
        self.colors.as_ref()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, smart_default::SmartDefault)]
#[serde(default)]
pub struct Sizes {
    #[default(0.0)]
    pub none: f32,
    #[default(1.0)]
    pub border: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, smart_default::SmartDefault)]
#[serde(default)]
pub struct Radii {
    #[default(0.0)]
    pub none: f32,
    #[default(4.0)]
    pub xs: f32,
    #[default(8.0)]
    pub sm: f32,
    #[default(12.0)]
    pub md: f32,
    #[default(12.0)]
    pub lg: f32,
    #[default(32.0)]
    pub xl: f32,
    #[default(999.0)]
    pub rounded: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, smart_default::SmartDefault)]
#[serde(default)]
pub struct Shadows {
    #[default(12.0)]
    pub small: f32,
    #[default(16.0)]
    pub medium: f32,
    #[default(24.0)]
    pub large: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, smart_default::SmartDefault)]
#[serde(default)]
pub struct Spacing {
    // Spacing tokens
    #[default(0.0)]
    pub none: f32,
    #[default(4.0)]
    pub small: f32,
    #[default(8.0)]
    pub medium: f32,
    #[default(16.0)]
    pub large: f32,
}

// Typography (font sizes in logical pixels)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, smart_default::SmartDefault)]
#[serde(default)]
pub struct Typography {
    #[default(30.0)]
    pub title0: f32,
    #[default(24.0)]
    pub title1: f32,
    #[default(20.0)]
    pub title2: f32,
    #[default(18.0)]
    pub title3: f32,
    #[default(16.0)]
    pub title4: f32,
    #[default(16.0)]
    pub title5: f32,
    #[default(14.0)]
    pub headline: f32,
    #[default(14.0)]
    pub body0: f32,
    #[default(12.0)]
    pub body2: f32,
    #[default(12.0)]
    pub caption0: f32,
    #[default(12.0)]
    pub caption1: f32,

    // CSS line-height for 14px body text is typically 1.6–1.5× ≈ 22px.
    // Aligns native spacing with web `px` conventions.
    #[default(22.0)]
    pub line_height: f32,
}

impl Styles {
    /// Build a complete `egui::Style` with proper spacing from design tokens.
    pub fn build_egui_style(&self, colors: &colors::Colors) -> egui::Style {
        egui::Style {
            visuals: self.build_egui_visuals(colors),
            spacing: egui::Spacing {
                item_spacing: egui::vec2(self.spacing.medium, self.spacing.small),
                button_padding: egui::vec2(self.spacing.medium, self.spacing.small),
                // combo_height: (self.typography.line_height + self.spacing.small * 2.0) * 5.0,
                ..Default::default()
            },
            ..egui::Style::default()
        }
    }
}
