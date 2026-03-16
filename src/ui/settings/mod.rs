//! Settings overlay (§3.2) — sidebar-navigable panel for Profiles, Agents, Tasks.
#![allow(dead_code)]

pub mod agents;
pub mod channels;
mod channels_draw;
pub mod general;
pub mod onboarding;
pub mod profiles;
mod profiles_form;
pub mod providers;
mod providers_draw;
pub mod scheduler;
/// Same content as [`SettingsState::draw`] but rendered directly into the child
/// window's egui context without the `Area` + `Foreground` wrapper — the
/// child window IS the settings panel.
mod settings_content;

use crate::ui::actions::UiAction;
use crate::ui::i18n;
use crate::ui::icons::{Icon, IconButtonCfg, icon_button};
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;
use crate::ui::widget::{Fragment, Widget};

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

// ─── Drawing ─────────────────────────────────────────────────────────────────

/// Bundled section-specific states for the settings draw pipeline.
///
/// Replaces the 12-parameter explosion that was threaded through every
/// `draw_settings*` function.
pub struct SettingsDrawCtx<'a> {
    pub general_ui_state: Option<&'a mut general::GeneralSettingsState>,
    pub channels_ui_state: Option<&'a mut channels::ChannelsSettingsState>,
    pub onboarding_wizard_state: Option<&'a mut onboarding::OnboardingWizardState>,
    pub agent_registry: Option<&'a mut crate::ai::agent::AgentRegistry>,
    pub agents_ui_state: Option<&'a mut agents::AgentsSettingsState>,
    pub profile_manager: Option<&'a mut crate::profile::ProfileManager>,
    pub profiles_ui_state: Option<&'a mut profiles::ProfilesSettingsState>,
    pub providers_ui_state: Option<&'a mut providers::ProvidersSettingsState>,
    pub task_store: Option<&'a mut crate::ai::scheduler::ScheduledTaskStore>,
    pub scheduler_ui_state: Option<&'a mut scheduler::SchedulerSettingsState>,
    pub scheduler_history: &'a [crate::ai::scheduler::ExecutionRecord],
    pub skills_panel_state: Option<&'a mut crate::ui::skills_panel::SkillsPanelState>,
}

/// Draw the Settings overlay if open. Returns UiActions emitted by Settings UI.
impl SettingsState {
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        styles: &Styles,
        colors: &Colors,
        sctx: SettingsDrawCtx<'_>,
    ) -> Vec<UiAction> {
        if !self.open {
            return Vec::new();
        }

        let mut actions = Vec::new();
        // Panel sized to ~80% of screen, centered.
        let screen = ctx.screen_rect();
        let margin = styles.spacing.large * 3.0;
        let panel_rect = screen.shrink(margin);

        egui::Area::new(egui::Id::new("settings_overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(panel_rect.min)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_float)
                    .stroke(egui::Stroke::new(styles.sizes.border, colors.border))
                    .corner_radius(egui::CornerRadius::same(styles.radii.md as _))
                    .inner_margin(egui::Margin::same(0))
                    .show(ui, |ui| {
                        ui.set_min_size(panel_rect.size());
                        ui.set_max_size(panel_rect.size());

                        self.draw_inner(ui, styles, colors, sctx, &mut actions);
                    });
            });

        // Escape to close.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.open = false;
            actions.push(UiAction::CloseSettings);
        }

        actions
    }
} // impl SettingsState (draw)

/// Settings overlay panel widget — wraps `draw_settings` with all sub-states.
pub struct SettingsOverlay<'a> {
    pub state: &'a mut SettingsState,
    pub general_ui_state: Option<&'a mut general::GeneralSettingsState>,
    pub channels_ui_state: Option<&'a mut channels::ChannelsSettingsState>,
    pub onboarding_wizard_state: Option<&'a mut onboarding::OnboardingWizardState>,
    pub agent_registry: Option<&'a mut crate::ai::agent::AgentRegistry>,
    pub agents_ui_state: Option<&'a mut agents::AgentsSettingsState>,
    pub profile_manager: Option<&'a mut crate::profile::ProfileManager>,
    pub profiles_ui_state: Option<&'a mut profiles::ProfilesSettingsState>,
    pub providers_ui_state: Option<&'a mut providers::ProvidersSettingsState>,
    pub task_store: Option<&'a mut crate::ai::scheduler::ScheduledTaskStore>,
    pub scheduler_ui_state: Option<&'a mut scheduler::SchedulerSettingsState>,
    pub scheduler_history: &'a [crate::ai::scheduler::ExecutionRecord],
    pub skills_panel_state: Option<&'a mut crate::ui::skills_panel::SkillsPanelState>,
}

impl Widget for SettingsOverlay<'_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Context>) {
        let c = SettingsDrawCtx {
            general_ui_state: self.general_ui_state.as_deref_mut(),
            channels_ui_state: self.channels_ui_state.as_deref_mut(),
            onboarding_wizard_state: self.onboarding_wizard_state.as_deref_mut(),
            agent_registry: self.agent_registry.as_deref_mut(),
            agents_ui_state: self.agents_ui_state.as_deref_mut(),
            profile_manager: self.profile_manager.as_deref_mut(),
            profiles_ui_state: self.profiles_ui_state.as_deref_mut(),
            providers_ui_state: self.providers_ui_state.as_deref_mut(),
            task_store: self.task_store.as_deref_mut(),
            scheduler_ui_state: self.scheduler_ui_state.as_deref_mut(),
            scheduler_history: self.scheduler_history,
            skills_panel_state: self.skills_panel_state.as_deref_mut(),
        };
        let ctx = f.ctx();
        let colors = &*f.colors;
        let actions = self.state.draw(ctx, f.styles, colors, c);
        f.dirties.actions.extend(actions);
    }
}
