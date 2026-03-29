use super::*;
use crate::ui::types::UiState;

pub struct SettingsModal<'a> {
    pub agent_registry: Option<&'a mut crate::ai::agent::AgentRegistry>,
    pub profile_manager: Option<&'a mut crate::profile::ProfileManager>,
    pub task_store: Option<&'a mut crate::ai::scheduler::ScheduledTaskStore>,
    pub scheduler_history: &'a [crate::ai::scheduler::ExecutionRecord],
}

impl Widget<egui::Context> for SettingsModal<'_> {
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <egui::Context as crate::ui::widget::Painter>::Ref<'a>,
        #[allow(unused)] mut ctx: crate::ui::widget::Context<'a>,
    ) {
        if !ctx.ui_state.settings_state.open {
            return;
        }

        let ui_state = &mut *ctx.ui_state;
        let styles = ctx.styles;
        let colors = std::rc::Rc::clone(&ctx.colors);
        let actions = &mut ctx.dirties.actions;

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(colors.bg_float)
                    .corner_radius(egui::CornerRadius::same(styles.radii.md as _))
                    .inner_margin(egui::Margin::same(0)),
            )
            .show(ui, |ui| {
                self.draw_inner(ui, ui_state, styles, &colors, actions);
            });

        // Escape to close.
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.ui_state.settings_state.open = false;
            ctx.dirties.actions.push(UiAction::CloseSettings);
        }
    }
}

impl SettingsModal<'_> {
    /// Shared inner content for Settings: title bar + sidebar + content area.
    ///
    /// Called by both [`SettingsState::draw`] (main window Area) and
    /// [`SettingsState::draw_child`] (child window CentralPanel).
    pub(super) fn draw_inner(
        &mut self,
        ui: &mut egui::Ui,
        ui_state: &mut UiState,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
    ) {
        // ── Title bar ────────────────────────────────────────
        egui::Frame::new()
            .inner_margin(styles.spacing.medium)
            .show(ui, |ui| {
                let avail = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), styles.typography.title5),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let close_btn_width = styles.typography.title5 + styles.spacing.medium;
                        ui.add_space((avail - close_btn_width) / 2.0 - styles.typography.title5);

                        ui.label(
                            egui::RichText::new(i18n::t("titlebar.settings"))
                                .color(colors.text_title)
                                .size(styles.typography.title5)
                                .strong(),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(styles.spacing.medium);
                            let close = icon_button(
                                ui,
                                IconButtonCfg {
                                    icon: Icon::ChromeClose,
                                    tooltip: "Close",
                                    base_color: colors.text_caption,
                                    hover_color: colors.text_title,
                                    pixel_size: styles.typography.title5,
                                    margin: styles.spacing.small,
                                },
                            );
                            if close.clicked() {
                                ui_state.settings_state.open = false;
                                actions.push(UiAction::CloseSettings);
                            }
                        });
                    },
                );
            });

        ui.separator();

        // ── Body: sidebar + content ──────────────────────────
        self.draw_body(ui_state, styles, colors, actions, ui);
    }

    fn draw_body(
        &mut self,
        ui_state: &mut UiState,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
        ui: &mut egui::Ui,
    ) {
        let body_rect = ui.available_rect_before_wrap();

        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(
                styles.spacing.large as i8,
                styles.spacing.medium as i8,
            ))
            .show(ui, |ui| {
                let available_height = ui.available_height();
                ui.horizontal(|ui| {
                    ui.set_height(available_height);
                    // Sidebar.
                    self.draw_sidebar(ui_state, styles, colors, body_rect, ui);

                    // Vertical separator.
                    ui.separator();

                    // Content area.
                    ui.with_layout(
                        // egui::vec2(body_rect.width() - sidebar_width - 20.0, body_rect.height()),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            ui.add_space(styles.spacing.medium);
                            self.draw_content(ui_state, styles, colors, actions, ui);
                        },
                    );
                });
            });
    }

    fn draw_content(
        &mut self,
        ui_state: &mut UiState,
        styles: &Styles,
        colors: &Colors,
        actions: &mut Vec<UiAction>,
        ui: &mut egui::Ui,
    ) {
        match ui_state.settings_state.active_section {
            SettingsSection::General => {
                let gen_actions = ui_state.general_ui_state.draw(ui, styles, colors);
                actions.extend(gen_actions);
            }
            SettingsSection::Profiles => {
                if let Some(mgr) = self.profile_manager.as_mut() {
                    actions.extend(ui_state.profiles_ui_state.draw(ui, mgr, styles, colors));
                } else {
                    Self::draw_section_placeholder(
                        ui,
                        ui_state.settings_state.active_section,
                        styles,
                        colors,
                    );
                }
            }
            SettingsSection::Channels => {
                let onboarding = ui_state.onboarding_wizard_state.as_mut();
                let ch_actions = ui_state
                    .channels_ui_state
                    .draw(ui, styles, colors, None, onboarding);
                actions.extend(ch_actions);
            }
            SettingsSection::LLMProviders => {
                let prov_actions = ui_state.providers_ui_state.draw(ui, styles, colors);
                actions.extend(prov_actions);
            }
            SettingsSection::AgentsAndSkills => {
                if let Some(reg) = self.agent_registry.as_mut() {
                    let agent_actions = ui_state.agents_ui_state.draw(
                        ui,
                        reg,
                        styles,
                        colors,
                        Some(&mut ui_state.skills_panel_state),
                    );
                    actions.extend(agent_actions);
                } else {
                    Self::draw_section_placeholder(
                        ui,
                        ui_state.settings_state.active_section,
                        styles,
                        colors,
                    );
                }
            }
            SettingsSection::ScheduledTasks => {
                if let Some(ts) = self.task_store.as_mut() {
                    let schedule_actions = ui_state.scheduler_ui_state.draw(
                        ui,
                        ts,
                        self.scheduler_history,
                        styles,
                        colors,
                    );
                    actions.extend(schedule_actions);
                } else {
                    Self::draw_section_placeholder(
                        ui,
                        ui_state.settings_state.active_section,
                        styles,
                        colors,
                    );
                }
            }
        }
    }

    fn draw_sidebar(
        &mut self,
        ui_state: &mut UiState,
        styles: &Styles,
        colors: &Colors,
        body_rect: egui::Rect,
        ui: &mut egui::Ui,
    ) -> egui::InnerResponse<()> {
        ui.allocate_ui_with_layout(
            egui::vec2(styles.typography.line_height * 7.0, body_rect.height()),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.add_space(styles.spacing.medium);
                for section in SettingsSection::all() {
                    let is_active = *section == ui_state.settings_state.active_section;
                    let text_color = if is_active {
                        colors.primary
                    } else {
                        colors.text_title
                    };

                    let resp = egui::Frame::new()
                        .corner_radius(egui::CornerRadius::same(styles.radii.sm as _))
                        .inner_margin(egui::Margin::symmetric(
                            styles.spacing.medium as _,
                            styles.spacing.small as _,
                        ))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(section.label())
                                        .color(text_color)
                                        .size(styles.typography.body2),
                                )
                                .selectable(false)
                                .extend()
                                .sense(egui::Sense::click()),
                            )
                        })
                        .inner;

                    if resp.clicked() {
                        ui_state.settings_state.active_section = *section;
                        ui.ctx().request_repaint();
                    }
                }
            },
        )
    }

    /// Draw stub content for each settings section.
    fn draw_section_placeholder(
        ui: &mut egui::Ui,
        section: SettingsSection,
        styles: &Styles,
        colors: &Colors,
    ) {
        match section {
            SettingsSection::General => {
                ui.label(
                    egui::RichText::new("General")
                        .color(colors.text_title)
                        .size(styles.typography.title3)
                        .strong(),
                );
                ui.add_space(styles.spacing.medium);
                ui.label(
                    egui::RichText::new("General application settings.")
                        .color(colors.text_caption)
                        .size(styles.typography.body2),
                );
            }
            SettingsSection::Profiles => {
                ui.label(
                    egui::RichText::new("Profiles")
                        .color(colors.text_title)
                        .size(styles.typography.title3)
                        .strong(),
                );
                ui.add_space(styles.spacing.medium);
                ui.label(
                    egui::RichText::new(
                        "Manage profiles with LLM configuration, sandbox rules, \
                     and environment variables.\n\n\
                     (Content will be implemented in US4.)",
                    )
                    .color(colors.text_caption)
                    .size(styles.typography.body2),
                );
            }
            SettingsSection::Channels => {
                ui.label(
                    egui::RichText::new("Channels")
                        .color(colors.text_title)
                        .size(styles.typography.title3)
                        .strong(),
                );
                ui.add_space(styles.spacing.medium);
                ui.label(
                    egui::RichText::new("Configure external messaging channels for Claw mode.")
                        .color(colors.text_caption)
                        .size(styles.typography.body2),
                );
            }
            SettingsSection::LLMProviders => {
                ui.label(
                    egui::RichText::new("LLM Providers")
                        .color(colors.text_title)
                        .size(styles.typography.title3)
                        .strong(),
                );
                ui.add_space(styles.spacing.medium);
                ui.label(
                    egui::RichText::new("Configure LLM provider endpoints and API keys.")
                        .color(colors.text_caption)
                        .size(styles.typography.body2),
                );
            }
            SettingsSection::AgentsAndSkills => {
                ui.label(
                    egui::RichText::new("[A] Agents & Skills")
                        .color(colors.text_title)
                        .size(styles.typography.title3)
                        .strong(),
                );
                ui.add_space(styles.spacing.medium);
                ui.label(
                    egui::RichText::new(
                        "Install and manage community-built agents. \
                     Enable/disable agents and their skills.\n\n\
                     (Content will be implemented in US4.)",
                    )
                    .color(colors.text_caption)
                    .size(styles.typography.body2),
                );
            }
            SettingsSection::ScheduledTasks => {
                ui.label(
                    egui::RichText::new("[S] Scheduled Tasks")
                        .color(colors.text_title)
                        .size(styles.typography.title3)
                        .strong(),
                );
                ui.add_space(styles.spacing.medium);
                ui.label(
                    egui::RichText::new(
                        "View and manage scheduled tasks. \
                     Create new tasks with cron schedules.\n\n\
                     (Content will be implemented in US7.)",
                    )
                    .color(colors.text_caption)
                    .size(styles.typography.body2),
                );
            }
        }
    }
} // impl SettingsState
