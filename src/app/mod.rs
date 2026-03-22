use std::collections::HashMap;
use std::sync::Arc;

pub(crate) mod browser;
mod chat;
pub(crate) mod commands;
pub mod daemon;
mod draw;
mod events;
mod keybindings;
mod lifecycle;
mod render;
pub mod session_persist;
mod state;
mod util;
mod window;

use browser::*;
use chat::*;
use keybindings::*;
use state::*;
use util::*;

// Re-export AppState for ui-layer modules that need mutable access.
pub(crate) use state::AppState;

// Re-export functions called from ui/dispatch.
pub(crate) use browser::{close_active_browser, open_browser, switch_browser_tab};
pub(crate) use keybindings::{handle_action, new_terminal_with_shell, open_history_tab, open_history_session};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::ai::stream::AppEvent;
use crate::channels::Channel;
use crate::config::AppConfig;
use crate::renderer::GpuContext;
use crate::renderer::atlas::TerminalRenderer;

use crate::renderer::scheduler::RenderSchedule;
use crate::renderer::terminal::input;
use crate::renderer::terminal::{SessionId, TerminalSession};
use crate::renderer::webview::Webview;
use crate::renderer::webview::bridge::WebviewToRust;
use crate::ui::block::BlockMode;
use crate::ui::i18n::{t, t_fmt};
use crate::ui::prompt::{CommandBlock, PromptState};
use crate::ui::{self, tiles};
use crate::ui::{AddressBarState, BrowserViewMode, TabInfo, UiAction, UiState};

use crate::ai::stream::PendingResultMap;

/// Handler that bridges winit events to our application.
pub(crate) struct App {
    pub(crate) config: AppConfig,
    pub(crate) state: Option<AppState>,
    /// Tokio runtime shared with AppState after initialization.
    pub(crate) runtime: Arc<tokio::runtime::Runtime>,
    /// Event loop proxy for sending events from background tasks.
    pub(crate) proxy: Option<EventLoopProxy<AppEvent>>,
    /// CLI --profile override: switch to this profile ID at startup.
    pub(crate) profile_override: Option<String>,
}

impl App {
    fn new(
        config: AppConfig,
        runtime: Arc<tokio::runtime::Runtime>,
        profile_override: Option<String>,
    ) -> Self {
        Self {
            config,
            state: None,
            runtime,
            proxy: None,
            profile_override,
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        lifecycle::handle_resumed(self, event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        window::events::handle_window_event(self, event_loop, window_id, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        events::handle_user_event(self, event_loop, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        render::handle_about_to_wait(self, event_loop);
    }
}

pub fn run(
    config: AppConfig,
    runtime: Arc<tokio::runtime::Runtime>,
    profile_override: Option<String>,
) {
    log::info!("Starting cronymax event loop");
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(config, runtime, profile_override);
    app.proxy = Some(event_loop.create_proxy());
    event_loop.run_app(&mut app).expect("Event loop error");
}
