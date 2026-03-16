//! Scheduled task command handlers extracted from cmd_settings.rs

use crate::app::*;

pub(super) fn handle_scheduled_task_action(state: &mut AppState, action: UiAction) {
    match action {
        UiAction::CreateScheduledTask => {
            let ui_st = &state.scheduler_ui_state;
            let task = crate::ai::scheduler::ScheduledTask {
                id: uuid::Uuid::new_v4().to_string(),
                name: ui_st.edit_name.clone(),
                cron: ui_st.edit_cron.clone(),
                action_type: ui_st.edit_action_type.clone(),
                action_value: ui_st.edit_action_value.clone(),
                agent_name: ui_st.edit_agent_name.clone(),
                profile_id: "default".to_string(),
                enabled: ui_st.edit_enabled,
                run_once: false,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            let name = task.name.clone();
            match state.task_store.create(task) {
                Ok(()) => {
                    log::info!("Created scheduled task '{}'", name);
                    state.scheduler_ui_state.creating_new = false;
                    state.scheduler_ui_state.reset_editor();
                }
                Err(e) => log::error!("Failed to create scheduled task: {}", e),
            }
        }
        UiAction::SaveScheduledTask(tid) => {
            let ui_st = &state.scheduler_ui_state;
            let task = crate::ai::scheduler::ScheduledTask {
                id: tid.clone(),
                name: ui_st.edit_name.clone(),
                cron: ui_st.edit_cron.clone(),
                action_type: ui_st.edit_action_type.clone(),
                action_value: ui_st.edit_action_value.clone(),
                agent_name: ui_st.edit_agent_name.clone(),
                profile_id: "default".to_string(),
                enabled: ui_st.edit_enabled,
                run_once: false,
                created_at: state
                    .task_store
                    .get(&tid)
                    .map(|t| t.created_at.clone())
                    .unwrap_or_default(),
            };
            if let Err(e) = state.task_store.update(&tid, task) {
                log::error!("Failed to save scheduled task '{}': {}", tid, e);
            } else {
                state.scheduler_ui_state.selected_task_id = None;
                state.scheduler_ui_state.fields_loaded_for = None;
            }
        }
        UiAction::DeleteScheduledTask(tid) => {
            if let Err(e) = state.task_store.delete(&tid) {
                log::error!("Failed to delete scheduled task '{}': {}", tid, e);
            }
        }
        UiAction::ToggleScheduledTask(tid) => match state.task_store.toggle_enabled(&tid) {
            Ok(new_state) => {
                log::info!(
                    "Scheduled task '{}' is now {}",
                    tid,
                    if new_state { "enabled" } else { "disabled" }
                );
            }
            Err(e) => log::error!("Failed to toggle scheduled task '{}': {}", tid, e),
        },
        _ => {}
    }
}
