//! UI action dispatch — routes `UiAction` variants to the appropriate handler.
//!
//! Each sub-module adds `impl Ui` methods; the main dispatcher here also lives
//! on `Ui` so callers do: `let (ui, mut ctx) = state.split_ui(); ui.handle_ui_action(&mut ctx, action, el);`

mod channel;
mod onboard;
mod settings;
mod split;
mod webview;

use winit::event_loop::ActiveEventLoop;

use crate::renderer::scheduler::RenderSchedule;
use crate::ui::{Ui, UiAction, actions::KeyAction, model::AppCtx, tiles};

impl Ui {
    pub(crate) fn handle_ui_action(
        &mut self,
        ctx: &mut AppCtx<'_>,
        action: UiAction,
        #[allow(unused)] event_loop: &ActiveEventLoop,
    ) {
        match action {
            UiAction::SwitchTab(..)
            | UiAction::CloseTab(..)
            | UiAction::NewChat
            | UiAction::NewTerminal
            | UiAction::NewTerminalWithShell(..)
            | UiAction::OpenHistory
            | UiAction::OpenHistorySession(..)
            | UiAction::OpenBrowserOverlay(..)
            | UiAction::ExecuteCommand(..)
            | UiAction::NavigateWebview(..)
            | UiAction::SwitchWebview(..)
            | UiAction::ActivateWebviewPane(..)
            | UiAction::CloseWebview(..)
            | UiAction::WebviewBack(..)
            | UiAction::WebviewForward(..)
            | UiAction::WebviewRefresh(..)
            | UiAction::FilterSearch(..)
            | UiAction::FilterClose
            | UiAction::FilterNext
            | UiAction::FilterPrev
            | UiAction::DockTab { .. }
            | UiAction::DockWebview
            | UiAction::DockWebviewLeft
            | UiAction::DockWebviewRight
            | UiAction::DockWebviewDown
            | UiAction::WebviewToTab(..)
            | UiAction::OpenInSystemBrowser
            | UiAction::SplitLeft
            | UiAction::SplitRight
            | UiAction::SplitDown => self.handle_ui_action_webview(ctx, action, event_loop),

            UiAction::PinTab(..)
            | UiAction::UnpinTab(..)
            | UiAction::StartWindowDrag
            | UiAction::CloseWindow
            | UiAction::Minimize
            | UiAction::ToggleMaximize
            | UiAction::PopOutOverlay
            | UiAction::BringOverlayToFront(..)
            | UiAction::OpenSettings
            | UiAction::CloseSettings
            | UiAction::OpenOverlay
            | UiAction::RelaunchApp
            | UiAction::NewWindowWithProfile(..)
            | UiAction::SwitchModel { .. }
            | UiAction::OpenWebviewTab(..)
            | UiAction::CloseWebviewTab(..)
            | UiAction::InstallAgent
            | UiAction::UninstallAgent(..)
            | UiAction::ToggleAgent(..)
            | UiAction::CreateProfile
            | UiAction::SaveProfile(..)
            | UiAction::DuplicateProfile(..)
            | UiAction::DeleteProfile(..)
            | UiAction::SetActiveProfile(..)
            | UiAction::SaveProviders
            | UiAction::CreateScheduledTask
            | UiAction::SaveScheduledTask(..)
            | UiAction::DeleteScheduledTask(..)
            | UiAction::ToggleScheduledTask(..) => {
                self.handle_ui_action_settings(ctx, action, event_loop)
            }

            UiAction::ToggleLarkChannel(..)
            | UiAction::StartOnboarding
            | UiAction::TestChannelConnection
            | UiAction::EnableClawMode
            | UiAction::DisableClawMode
            | UiAction::SaveChannelConfig
            | UiAction::SaveChannelConfigById { .. }
            | UiAction::ToggleLarkChannelById { .. }
            | UiAction::TestChannelConnectionById { .. }
            | UiAction::AddLarkChannel
            | UiAction::RemoveLarkChannel { .. } => {
                self.handle_ui_action_channel(ctx, action, event_loop)
            }

            UiAction::OnboardingWizardStepChanged { .. }
            | UiAction::OnboardingWizardComplete { .. }
            | UiAction::OnboardingAutomateLarkSetup { .. }
            | UiAction::ToggleStarred { .. }
            | UiAction::CloseChannel(..)
            | UiAction::OpenChannelTab { .. }
            | UiAction::InstallSkill(..)
            | UiAction::UninstallSkill(..)
            | UiAction::ToggleSkill { .. }
            | UiAction::SearchSkills(..)
            | UiAction::ReloadSkills
            | UiAction::ToggleSkillsPanel => self.handle_ui_action_onboard(ctx, action, event_loop),
        }
    }

    pub(crate) fn handle_colon_command(
        &mut self,
        ctx: &mut AppCtx<'_>,
        cmd: &str,
        #[allow(unused)] event_loop: &ActiveEventLoop,
    ) {
        let cmd = cmd.trim();
        // Strip leading `:` if present (overlay inserts `:action` format).
        let cmd = cmd.strip_prefix(':').unwrap_or(cmd);

        if let Some(url) = cmd.strip_prefix("webview ") {
            let url = url.trim();
            if url.is_empty() {
                log::warn!(":webview command requires a URL");
                return;
            }
            crate::app::open_browser(self, ctx, url, event_loop);
        } else if cmd == "close" || cmd == "q" {
            if !self.browser_tabs.is_empty() {
                crate::app::close_active_browser(self, ctx);
            } else if let Some(sid) = tiles::active_terminal_session(&self.tile_tree) {
                ctx.sessions.remove(&sid);
                tiles::remove_terminal_pane(&mut self.tile_tree, sid);
                ctx.scheduler.mark_dirty();
            }
        } else if cmd == "newtab" {
            crate::app::handle_action(self, ctx, KeyAction::NewChat);
        } else if cmd == "closetab" {
            crate::app::handle_action(self, ctx, KeyAction::CloseTab);
        } else if cmd == "new_terminal_tab" {
            crate::app::handle_action(self, ctx, KeyAction::NewTerminal);
        } else if cmd == "split_horizontal" {
            crate::app::handle_action(self, ctx, KeyAction::SplitHorizontal);
        } else if cmd == "split_vertical" {
            crate::app::handle_action(self, ctx, KeyAction::SplitVertical);
        } else if cmd == "filter" {
            ctx.ui_state.filter.toggle();
            ctx.scheduler.mark_dirty();
        } else if cmd == "copilot-login" {
            if let Some(client) = ctx.llm_client.as_ref() {
                if client.copilot_authenticated() {
                    log::info!("Copilot: already authenticated");
                } else {
                    client.start_copilot_login(ctx.proxy.clone(), ctx.runtime);
                }
            } else {
                log::warn!("Copilot: LLM client not initialized");
            }
        } else {
            log::warn!("Unknown command: :{}", cmd);
        }
    }
}
