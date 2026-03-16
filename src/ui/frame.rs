use std::collections::HashMap;

use crate::config::AppConfig;
use crate::renderer::terminal::SessionId;
use crate::ui;
use crate::ui::block::Block;
use crate::ui::prompt::PromptState;
use crate::ui::styles::Styles;
use crate::ui::tiles;
use crate::ui::widget::Fragment;

use crate::ui::widget::Widget;

/// Root widget that renders the complete UI frame.
///
/// Encapsulates the body of the `egui.run` closure — builds the tiles panel,
/// draws all widgets via [`ui::draw_all`], and renders the settings overlay.
pub struct FrameWidget<'a> {
    // ── Main UI (tiles panel) ───────────────────────────────────────────
    pub tile_tree: &'a mut egui_tiles::Tree<tiles::Pane>,
    pub prompt_editors: &'a mut HashMap<SessionId, PromptState>,
    pub blocks: HashMap<SessionId, Block>,
    pub session_chats: &'a mut HashMap<SessionId, crate::ui::chat::SessionChat>,
    pub live_outputs: HashMap<SessionId, String>,
    pub channel_messages: &'a HashMap<String, Vec<crate::channels::ChannelDisplayMessage>>,
    pub pane_widgets: &'a mut tiles::PaneWidgetStore,
    // ── Settings overlay ────────────────────────────────────────────────
    pub settings_in_child: bool,
    pub settings_state: &'a mut crate::ui::settings::SettingsState,
    pub theme: &'a Styles,
    pub profile_manager: &'a std::sync::Mutex<crate::profile::ProfileManager>,
    pub providers_ui_state: &'a mut crate::ui::settings::providers::ProvidersSettingsState,
    pub config: &'a AppConfig,
    pub general_ui_state: &'a mut crate::ui::settings::general::GeneralSettingsState,
    pub channels_ui_state: &'a mut crate::ui::settings::channels::ChannelsSettingsState,
    pub onboarding_wizard_state:
        Option<&'a mut crate::ui::settings::onboarding::OnboardingWizardState>,
    pub agent_registry: &'a mut crate::ai::agent::AgentRegistry,
    pub agents_ui_state: &'a mut crate::ui::settings::agents::AgentsSettingsState,
    pub profiles_ui_state: &'a mut crate::ui::settings::profiles::ProfilesSettingsState,
    pub task_store: &'a mut crate::ai::scheduler::ScheduledTaskStore,
    pub scheduler_ui_state: &'a mut crate::ui::settings::scheduler::SchedulerSettingsState,
    pub scheduler_history: &'a [crate::ai::scheduler::ExecutionRecord],
    pub skills_panel_state: &'a mut crate::ui::skills_panel::SkillsPanelState,
}

impl Widget for FrameWidget<'_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Context>) {
        ui::draw_all(
            ui::tiles::TilesPanel {
                tile_tree: self.tile_tree,
                prompt_editors: self.prompt_editors,
                blocks: std::mem::take(&mut self.blocks),
                session_chats: self.session_chats,
                live_outputs: std::mem::take(&mut self.live_outputs),
                channel_messages: self.channel_messages,
                channel_connection_state: f.ui_state.channel_connection_state,
                pane_widgets: self.pane_widgets,
            },
            &mut f,
        );

        // Settings overlay (foreground layer, above everything else).
        if !self.settings_in_child {
            let mut pm_guard = self.profile_manager.lock().unwrap();
            if !self.providers_ui_state.loaded {
                self.providers_ui_state.load_from_config(self.config);
            }
            let actions = self.settings_state.draw(
                f.ctx(),
                self.theme,
                &f.colors,
                crate::ui::settings::SettingsDrawCtx {
                    general_ui_state: Some(&mut *self.general_ui_state),
                    channels_ui_state: Some(&mut *self.channels_ui_state),
                    onboarding_wizard_state: self.onboarding_wizard_state.as_deref_mut(),
                    agent_registry: Some(&mut *self.agent_registry),
                    agents_ui_state: Some(&mut *self.agents_ui_state),
                    profile_manager: Some(&mut *pm_guard),
                    profiles_ui_state: Some(&mut *self.profiles_ui_state),
                    providers_ui_state: Some(&mut *self.providers_ui_state),
                    task_store: Some(&mut *self.task_store),
                    scheduler_ui_state: Some(&mut *self.scheduler_ui_state),
                    scheduler_history: self.scheduler_history,
                    skills_panel_state: Some(&mut *self.skills_panel_state),
                },
            );
            f.dirties.actions.extend(actions);
        }
    }
}
