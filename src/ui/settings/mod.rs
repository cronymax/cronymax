//! Settings overlay (§3.2) — sidebar-navigable panel for Profiles, Agents, Tasks.
#![allow(dead_code)]

pub mod agents;
pub mod channels;
mod channels_draw;
pub mod general;
/// Same content as [`SettingsState::draw`] but rendered directly into the child
/// window's egui context without the `Area` + `Foreground` wrapper — the
/// child window IS the settings panel.
mod modal;
pub mod onboarding;
pub mod profiles;
mod profiles_form;
pub mod providers;
mod providers_draw;
pub mod scheduler;

pub use modal::SettingsModal;

use crate::ui::actions::UiAction;
use crate::ui::i18n;
use crate::ui::icons::{Icon, IconButtonCfg, icon_button};
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;
use crate::ui::widget::Widget;

// ─── Types ───────────────────────────────────────────────────────────────────

/// Active section in the Settings sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    General,
    Channels,
    LLMProviders,
    Profiles,
    AgentsAndSkills,
    ScheduledTasks,
}

impl SettingsSection {
    /// Display label for the section.
    pub fn label(&self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Profiles => "Profiles",
            Self::Channels => "Channels",
            Self::LLMProviders => "LLM Providers",
            Self::AgentsAndSkills => "Agents & Skills",
            Self::ScheduledTasks => "Scheduled Tasks",
        }
    }

    /// All sections in display order.
    pub fn all() -> &'static [Self] {
        &[
            Self::General,
            Self::Profiles,
            Self::Channels,
            Self::LLMProviders,
            Self::AgentsAndSkills,
            Self::ScheduledTasks,
        ]
    }
}

/// Persistent state for the Settings overlay.
#[derive(Debug, Clone)]
pub struct SettingsState {
    /// Whether the Settings overlay is currently visible.
    pub open: bool,
    /// The currently active sidebar section.
    pub active_section: SettingsSection,
    /// Whether the user has unsaved changes.
    pub dirty: bool,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            active_section: SettingsSection::General,
            dirty: false,
        }
    }
}
