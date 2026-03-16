//! User event dispatch - routes AppEvent variants to sub-handlers

pub(super) mod channel;
pub(super) mod llm;
pub(super) mod misc;
pub(super) mod onboard;

use super::*;

pub(super) fn handle_user_event(app: &mut App, _event_loop: &ActiveEventLoop, event: AppEvent) {
    let state = match app.state.as_mut() {
        Some(s) => s,
        None => return,
    };

    // Route to categorized sub-handlers.
    // Each sub-handler matches its own variants with _ => {} fallthrough.
    match event {
        AppEvent::LlmToken { .. }
        | AppEvent::LlmDone { .. }
        | AppEvent::LlmError { .. }
        | AppEvent::CompactionNeeded { .. }
        | AppEvent::ToolResult { .. }
        | AppEvent::SkillUiAction { .. } => llm::handle_llm_event(state, event, _event_loop),

        AppEvent::ScheduledTaskStarted { .. }
        | AppEvent::ScheduledTaskCompleted { .. }
        | AppEvent::ScheduledTaskFire { .. }
        | AppEvent::ModelsLoaded { .. }
        | AppEvent::InjectScript { .. }
        | AppEvent::TerminalExec { .. }
        | AppEvent::ReadTerminalScreen { .. }
        | AppEvent::CopilotDeviceCode { .. }
        | AppEvent::CopilotLoginComplete { .. }
        | AppEvent::CopilotLoginFailed { .. } => misc::handle_misc_event(state, event, _event_loop),

        AppEvent::ChannelMessageReceived { .. }
        | AppEvent::ChannelSendReply { .. }
        | AppEvent::ChannelStatusChanged { .. }
        | AppEvent::ChannelTestResult { .. }
        | AppEvent::ChannelBotCheckResult { .. }
        | AppEvent::ChannelTestResultById { .. }
        | AppEvent::ChannelBotCheckResultById { .. }
        | AppEvent::ReloadSkills
        | AppEvent::SkillSearchResults { .. }
        | AppEvent::SkillSearchError { .. }
        | AppEvent::FindFiles { .. }
        | AppEvent::StarChatBlock { .. }
        | AppEvent::UnstarChatBlock { .. }
        | AppEvent::ReferenceTerminalOutput { .. }
        | AppEvent::ReferenceBlockContent { .. }
        | AppEvent::AddContext { .. }
        | AppEvent::RemoveContext { .. }
        | AppEvent::CompactContext { .. }
        | AppEvent::RenameTab { .. } => channel::handle_channel_event(state, event, _event_loop),

        AppEvent::OnboardingStart { .. }
        | AppEvent::OnboardingAdvanceStep { .. }
        | AppEvent::OnboardingStoreCredentials { .. }
        | AppEvent::OnboardingTestConnection { .. }
        | AppEvent::OnboardingFinalize { .. }
        | AppEvent::OnboardingBrowserAutomationFinished { .. }
        | AppEvent::SchedulerCreate { .. }
        | AppEvent::SchedulerList { .. }
        | AppEvent::SchedulerGet { .. }
        | AppEvent::SchedulerDelete { .. }
        | AppEvent::SchedulerToggle { .. }
        | AppEvent::SchedulerUpdate { .. } => {
            onboard::handle_onboard_event(state, event, _event_loop)
        }
        // Ollama management events — inline info messages and model refresh.
        AppEvent::OllamaInfoMessage { session_id, text } => {
            if let Some(sid) = session_id {
                push_info_block(state, sid, &text);
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::OllamaPullProgress { session_id, text } => {
            if let Some(sid) = session_id {
                update_info_block(state, sid, &text);
            }
            state.scheduler.mark_dirty();
        }
        AppEvent::OllamaPullComplete { model: _ } => {
            // Trigger model list refresh.
            if let Some(ref client) = state.llm_client {
                client.fetch_available_models(state.proxy.clone(), &state.runtime);
            }
            state.scheduler.mark_dirty();
        }
        // Render loop events — signal the scheduler.
        AppEvent::PtyDataReady { .. } | AppEvent::RequestRepaint => {
            crate::renderer::scheduler::RenderSchedule::mark_dirty(&mut state.scheduler);
        }
        AppEvent::RequestRepaintAfter { delay } => {
            crate::renderer::scheduler::RenderSchedule::schedule_repaint_after(
                &mut state.scheduler,
                delay,
            );
        }
    }
}
