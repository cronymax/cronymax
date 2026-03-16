//! UI action command dispatch

use super::*;

pub(super) fn handle_ui_action(
    state: &mut AppState,
    action: UiAction,
    #[allow(unused)] event_loop: &ActiveEventLoop,
) {
    match action {
        UiAction::SwitchTab(..)
        | UiAction::CloseTab(..)
        | UiAction::NewChat
        | UiAction::NewTerminal
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
        | UiAction::SplitDown => {
            super::cmd::webview::handle_ui_action_webview(state, action, event_loop)
        }

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
            super::cmd::settings::handle_ui_action_settings(state, action, event_loop)
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
            super::cmd::channel::handle_ui_action_channel(state, action, event_loop)
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
        | UiAction::ToggleSkillsPanel => {
            super::cmd::onboard::handle_ui_action_onboard(state, action, event_loop)
        }
    }
}

pub(super) fn handle_colon_command(
    state: &mut AppState,
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
        open_webview(state, url, event_loop);
    } else if cmd == "close" || cmd == "q" {
        if !state.webview_tabs.is_empty() {
            close_active_webview(state);
        } else if let Some(sid) = tiles::active_terminal_session(&state.tile_tree) {
            state.sessions.remove(&sid);
            tiles::remove_terminal_pane(&mut state.tile_tree, sid);
            state.scheduler.mark_dirty();
        }
    } else if cmd == "newtab" {
        handle_action(state, Action::NewChat);
    } else if cmd == "closetab" {
        handle_action(state, Action::CloseTab);
    } else if cmd == "new_terminal_tab" {
        handle_action(state, Action::NewTerminal);
    } else if cmd == "split_horizontal" {
        handle_action(state, Action::SplitHorizontal);
    } else if cmd == "split_vertical" {
        handle_action(state, Action::SplitVertical);
    } else if cmd == "filter" {
        state.ui_state.filter.toggle();
        state.scheduler.mark_dirty();
    } else if cmd == "copilot-login" {
        if let Some(ref client) = state.llm_client {
            if client.copilot_authenticated() {
                log::info!("Copilot: already authenticated");
            } else {
                client.start_copilot_login(state.proxy.clone(), &state.runtime);
            }
        } else {
            log::warn!("Copilot: LLM client not initialized");
        }
    } else if let Some(args) = cmd.strip_prefix("ollama") {
        let args = args.strip_prefix(' ').unwrap_or(args);
        super::cmd::ollama::handle_ollama_command(state, args);
    } else if let Some(args) = cmd
        .strip_prefix("credentials")
        .or_else(|| cmd.strip_prefix("creds"))
    {
        let args = args.strip_prefix(' ').unwrap_or(args);
        super::cmd::credentials::handle_credentials_command(state, args);
    } else {
        log::warn!("Unknown command: :{}", cmd);
    }
}
