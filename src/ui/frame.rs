use std::collections::HashMap;

use crate::config::AppConfig;
use crate::renderer::terminal::SessionId;
use crate::ui;
use crate::ui::blocks::BlockGrid;
use crate::ui::filter::FilterBarWidget;
use crate::ui::styles::Styles;
use crate::ui::tiles;
use crate::ui::titlebar::TitlebarWidget;
use crate::ui::widget::{Fragment, Widget};

/// Root widget that renders the complete UI frame.
///
/// Encapsulates the body of the `egui.run` closure — builds the tiles panel,
/// draws all widgets via [`ui::draw_all`], and renders the settings overlay.
pub struct FrameWidget<'a> {
    // ── Main UI (tiles panel) ───────────────────────────────────────────
    pub tile_tree: &'a mut egui_tiles::Tree<tiles::Pane>,
    pub blocks: HashMap<SessionId, BlockGrid>,
    pub session_chats: &'a mut HashMap<SessionId, crate::ui::chat::SessionChat>,
    pub live_outputs: HashMap<SessionId, String>,
    pub channel_messages: &'a HashMap<String, Vec<crate::channels::ChannelDisplayMessage>>,
    // ── Settings overlay ────────────────────────────────────────────────
    pub settings_in_child: bool,
    pub theme: &'a Styles,
    pub profile_manager: &'a std::sync::Mutex<crate::profile::ProfileManager>,
    pub config: &'a AppConfig,
    pub agent_registry: &'a mut crate::ai::agent::AgentRegistry,
    pub task_store: &'a mut crate::ai::scheduler::ScheduledTaskStore,
    pub scheduler_history: &'a [crate::ai::scheduler::ExecutionRecord],
}

impl Widget for FrameWidget<'_> {
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, egui::Context>) {
        let ctx = f.ctx();
        // ── Window frame: rounded corners ─────────────────────────────────
        ctx.layer_painter(egui::LayerId::background()).rect_filled(
            ctx.screen_rect(),
            egui::CornerRadius::from(f.styles.spacing.large),
            f.colors.bg_body,
        );

        // child widgets
        {
            f.add(TitlebarWidget);

            f.add(FilterBarWidget);

            f.add(ui::tiles::TilesPanel {
                tile_tree: self.tile_tree,
                blocks: std::mem::take(&mut self.blocks),
                session_chats: self.session_chats,
                live_outputs: std::mem::take(&mut self.live_outputs),
                channel_messages: self.channel_messages,
                channel_connection_state: f.ui_state.channel_connection_state,
            });
        }

        // ── Window border stroke (on top of everything) ─────────────────────
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("window_border"),
        ))
        .rect_stroke(
            ctx.screen_rect(),
            egui::CornerRadius::from(f.styles.spacing.large),
            egui::Stroke::new(f.styles.sizes.border, f.colors.border),
            egui::StrokeKind::Inside,
        );

        // The docked tooltip is still communicated via the tile behavior's field.
        // TilesPanel doesn't surface it through WidgetResponse (it's a separate concern
        // routed to FloatPanel), so we extract it from the tiles behavior state that
        // was stored in ui_state during TilesPanel::show().
        f.dirties.mount_tooltip(f.ui_state.docked_tooltip.take());
        //     // Settings overlay (foreground layer, above everything else).
        //     if !self.settings_in_child {
        //         let mut pm_guard = self.profile_manager.lock().unwrap();
        //         if !self.providers_ui_state.loaded {
        //             self.providers_ui_state.load_from_config(self.config);
        //         }
        //         let actions = self.settings_state.draw(
        //             f.ctx(),
        //             self.theme,
        //             &f.colors,
        //             crate::ui::settings::SettingsDrawCtx {
        //                 general_ui_state: Some(&mut *self.general_ui_state),
        //                 channels_ui_state: Some(&mut *self.channels_ui_state),
        //                 onboarding_wizard_state: self.onboarding_wizard_state.as_deref_mut(),
        //                 agent_registry: Some(&mut *self.agent_registry),
        //                 agents_ui_state: Some(&mut *self.agents_ui_state),
        //                 profile_manager: Some(&mut *pm_guard),
        //                 profiles_ui_state: Some(&mut *self.profiles_ui_state),
        //                 providers_ui_state: Some(&mut *self.providers_ui_state),
        //                 task_store: Some(&mut *self.task_store),
        //                 scheduler_ui_state: Some(&mut *self.scheduler_ui_state),
        //                 scheduler_history: self.scheduler_history,
        //                 skills_panel_state: Some(&mut *self.skills_panel_state),
        //             },
        //         );
        //         f.dirties.actions.extend(actions);
        //     }
    }
}
