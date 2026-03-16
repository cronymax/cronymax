use std::collections::HashMap;
use std::sync::Arc;

mod chat;
mod cmd;
mod commands;
pub mod daemon;
mod draw;
mod events;
mod keybinds;
mod lifecycle;
mod mouse;
mod render;
pub mod session_persist;
mod state;
mod tabs;
mod util;
mod webview;
mod window;

use chat::*;
use commands::*;
use keybinds::*;
use mouse::*;
use state::*;
use tabs::*;
use util::*;
use webview::*;

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
use crate::renderer::cursor::{CursorRect, CursorShape};
use crate::renderer::egui_pass::{EguiIntegration, ScreenDescriptor};
use crate::renderer::scheduler::RenderSchedule;
use crate::renderer::text;
use crate::renderer::terminal::input;
use crate::renderer::terminal::{SessionId, TerminalSession};
use crate::ui::block::{Block, BlockMode};
use crate::ui::browser::{self, AddrBarButton};
use crate::ui::i18n::{t, t_fmt};
use crate::ui::prompt::{CommandBlock, PromptState};
use crate::ui::styles::Styles;
use crate::ui::{self, tiles};
use crate::ui::{AddressBarState, BrowserViewMode, TabInfo, UiAction, UiState};
use crate::renderer::webview::BrowserView;
use crate::renderer::panels::FloatPanelState;
use crate::renderer::webview::bridge::WebviewToRust;
use crate::renderer::webview::manager::{WebviewManager, ZLayer};
use crate::renderer::webview::split::{Bounds, VerticalSplit};

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
