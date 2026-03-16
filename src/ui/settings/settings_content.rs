use super::*;

impl SettingsState {
    pub fn draw_child(
        &mut self,
        ctx: &egui::Context,
        styles: &Styles,
        colors: &Colors,
        sctx: SettingsDrawCtx<'_>,
    ) -> Vec<UiAction> {
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(colors.bg_float)
                    .corner_radius(egui::CornerRadius::same(styles.radii.md as _))
                    .inner_margin(egui::Margin::same(0)),
            )
            .show(ctx, |ui| {
                self.draw_inner(ui, styles, colors, sctx, &mut actions)
            });

        // Escape to close.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.open = false;
            actions.push(UiAction::CloseSettings);
        }

        actions
    }

    /// Shared inner content for Settings: title bar + sidebar + content area.
    ///
    /// Called by both [`SettingsState::draw`] (main window Area) and
    /// [`SettingsState::draw_child`] (child window CentralPanel).
    pub(super) fn draw_inner(
        &mut self,
        ui: &mut egui::Ui,
        styles: &Styles,
        colors: &Colors,
        ctx: SettingsDrawCtx<'_>,
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
                                self.open = false;
                                actions.push(UiAction::CloseSettings);
                            }
                        });
                    },
                );
            });

        ui.separator();

        // ── Body: sidebar + content ──────────────────────────
        self.draw_body(styles, colors, ctx, actions, ui);
    }

    fn draw_body(
        &mut self,
        styles: &Styles,
        colors: &Colors,
        ctx: SettingsDrawCtx<'_>,
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
                    self.draw_sidebar(styles, colors, body_rect, ui);

                    // Vertical separator.
                    ui.separator();

                    // Content area.
                    ui.with_layout(
                        // egui::vec2(body_rect.width() - sidebar_width - 20.0, body_rect.height()),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            ui.add_space(styles.spacing.medium);
                            self.draw_content(styles, colors, ctx, actions, ui);
                        },
                    );
                });
            });
    }

    fn draw_content(
        &mut self,
        styles: &Styles,
        colors: &Colors,
        mut ctx: SettingsDrawCtx<'_>,
        actions: &mut Vec<UiAction>,
        ui: &mut egui::Ui,
    ) {
        match self.active_section {
            SettingsSection::General => {
                if let Some(gen_st) = ctx.general_ui_state.as_mut() {
                    let gen_actions = gen_st.draw(ui, styles, colors);
                    actions.extend(gen_actions);
                } else {
                    Self::draw_section_placeholder(ui, self.active_section, styles, colors);
                }
            }
            SettingsSection::Profiles => {
                if let (Some(mgr), Some(ui_st)) =
                    (ctx.profile_manager.as_mut(), ctx.profiles_ui_state.as_mut())
                {
                    actions.extend(ui_st.draw(ui, mgr, styles, colors));
                } else {
                    Self::draw_section_placeholder(ui, self.active_section, styles, colors);
                }
            }
            SettingsSection::Channels => {
                if let Some(ch_st) = ctx.channels_ui_state.as_mut() {
                    let ch_actions =
                        ch_st.draw(ui, styles, colors, None, ctx.onboarding_wizard_state);
                    actions.extend(ch_actions);
                } else {
                    Self::draw_section_placeholder(ui, self.active_section, styles, colors);
                }
            }
            SettingsSection::LLMProviders => {
                if let Some(prov_st) = ctx.providers_ui_state.as_mut() {
                    let prov_actions = prov_st.draw(ui, styles, colors);
                    actions.extend(prov_actions);
                } else {
                    Self::draw_section_placeholder(ui, self.active_section, styles, colors);
                }
            }
            SettingsSection::AgentsAndSkills => {
                if let (Some(reg), Some(ui_st)) =
                    (ctx.agent_registry.as_mut(), ctx.agents_ui_state.as_mut())
                {
                    let agent_actions = ui_st.draw(ui, reg, styles, colors, ctx.skills_panel_state);
                    actions.extend(agent_actions);
                } else {
                    Self::draw_section_placeholder(ui, self.active_section, styles, colors);
                }
            }
            SettingsSection::ScheduledTasks => {
                if let (Some(ts), Some(ui_st)) =
                    (ctx.task_store.as_mut(), ctx.scheduler_ui_state.as_mut())
                {
                    let sched_actions = ui_st.draw(ui, ts, ctx.scheduler_history, styles, colors);
                    actions.extend(sched_actions);
                } else {
                    Self::draw_section_placeholder(ui, self.active_section, styles, colors);
                }
            }
        }
    }

    fn draw_sidebar(
        &mut self,
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
                    let is_active = *section == self.active_section;
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
                        self.active_section = *section;
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
