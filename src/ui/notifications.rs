//! Toast notification system — in-app toast queue rendered in bottom-right corner.

use std::collections::VecDeque;
use std::time::Instant;

use egui::{Pos2, Vec2};

use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;

/// Severity level for a toast notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Success,
    Warning,
    Error,
}

/// A single toast notification.
#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u32,
    pub title: String,
    pub body: String,
    pub severity: Severity,
    pub created_at: Instant,
    pub ttl_ms: u64,
    pub dismissible: bool,
}

impl Toast {
    /// Whether this toast has expired.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed().as_millis() as u64 >= self.ttl_ms
    }

    /// Progress fraction (1.0 = just created, 0.0 = expired).
    pub fn progress(&self) -> f32 {
        let elapsed = self.created_at.elapsed().as_millis() as f32;
        let ttl = self.ttl_ms as f32;
        (1.0 - elapsed / ttl).clamp(0.0, 1.0)
    }
}

/// State for the toast notification system.
#[derive(Debug)]
pub struct NotificationState {
    pub toasts: VecDeque<Toast>,
    pub max_visible: usize,
    next_id: u32,
}

impl Default for NotificationState {
    fn default() -> Self {
        Self {
            toasts: VecDeque::new(),
            max_visible: 3,
            next_id: 1,
        }
    }
}

impl NotificationState {
    /// Push a new toast notification.
    pub fn push(&mut self, title: impl Into<String>, body: impl Into<String>, severity: Severity) {
        let toast = Toast {
            id: self.next_id,
            title: title.into(),
            body: body.into(),
            severity,
            created_at: Instant::now(),
            ttl_ms: match severity {
                Severity::Error => 8000,
                Severity::Warning => 6000,
                _ => 4000,
            },
            dismissible: true,
        };
        self.next_id += 1;
        self.toasts.push_back(toast);
    }

    /// Push a quick info toast.
    pub fn info(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push(title, body, Severity::Info);
    }

    /// Push a success toast.
    pub fn success(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push(title, body, Severity::Success);
    }

    /// Push a warning toast.
    pub fn warning(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push(title, body, Severity::Warning);
    }

    /// Push an error toast.
    pub fn error(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.push(title, body, Severity::Error);
    }

    /// Dismiss a toast by ID.
    pub fn dismiss(&mut self, id: u32) {
        self.toasts.retain(|t| t.id != id);
    }

    /// Remove expired toasts and return whether any were removed.
    pub fn gc(&mut self) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|t| !t.is_expired());
        self.toasts.len() != before
    }

    /// Draw toasts in the bottom-right corner of the screen.
    pub fn draw(&mut self, ctx: &egui::Context, styles: &Styles, colors: &Colors) {
        // Garbage-collect expired toasts.
        if self.gc() {
            ctx.request_repaint();
        }

        if self.toasts.is_empty() {
            return;
        }

        // Request repaint while toasts are visible (for TTL countdown).
        // Use a 1-second interval instead of immediate repaint to avoid
        // spinning the GPU at max frame rate for the entire toast duration.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        let screen = ctx.screen_rect();
        let toast_width = 320.0_f32;
        let toast_margin = 12.0_f32;
        let toast_spacing = 8.0_f32;
        let right_edge = screen.max.x - toast_margin;
        let mut y = screen.max.y - toast_margin;

        let visible: Vec<_> = self.toasts.iter().rev().take(self.max_visible).collect();

        let mut dismiss_id = None;

        for toast in &visible {
            let toast_height = 56.0_f32;
            y -= toast_height;
            let toast_rect = egui::Rect::from_min_size(
                Pos2::new(right_edge - toast_width, y),
                Vec2::new(toast_width, toast_height),
            );
            y -= toast_spacing;

            let alpha = (toast.progress() * 3.0).min(1.0);

            let area = egui::Area::new(egui::Id::new(("toast", toast.id)))
                .fixed_pos(toast_rect.min)
                .order(egui::Order::Foreground)
                .interactable(true);

            area.show(ctx, |ui| {
                let severity_color = match toast.severity {
                    Severity::Info => colors.info,
                    Severity::Success => colors.success,
                    Severity::Warning => colors.warning,
                    Severity::Error => colors.danger,
                };

                let bg = colors.bg_float.linear_multiply(alpha);
                let frame = egui::Frame::new()
                    .fill(bg)
                    .inner_margin(egui::Margin::from(styles.spacing.medium))
                    .corner_radius(styles.radii.sm)
                    .stroke(egui::Stroke::new(
                        1.0,
                        severity_color.linear_multiply(alpha * 0.5),
                    ));

                frame.show(ui, |ui| {
                    ui.set_width(toast_width - styles.spacing.medium * 2.0);
                    ui.horizontal(|ui| {
                        // Severity indicator dot.
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                        ui.painter().circle_filled(
                            rect.center(),
                            4.0,
                            severity_color.linear_multiply(alpha),
                        );

                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&toast.title)
                                    .size(styles.typography.body0)
                                    .color(colors.text_title.linear_multiply(alpha))
                                    .strong(),
                            );
                            if !toast.body.is_empty() {
                                ui.label(
                                    egui::RichText::new(&toast.body)
                                        .size(styles.typography.caption0)
                                        .color(colors.text_caption.linear_multiply(alpha)),
                                );
                            }
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if toast.dismissible && ui.small_button("✕").clicked() {
                                dismiss_id = Some(toast.id);
                            }
                        });
                    });
                });
            });
        }

        if let Some(id) = dismiss_id {
            self.dismiss(id);
        }
    }
}
