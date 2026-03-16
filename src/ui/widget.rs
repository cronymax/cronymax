//! Trait-based widget architecture for standardized UI rendering.
//!
//! Defines `Widget<T>` — a unified trait for panel, inline, and pane widgets.
//!
//! - `Fragment<egui::Context>` (default): panel-level, holds `&egui::Context`
//! - `Fragment<egui::Ui>`: inline/pane-level, holds `&mut egui::Ui`
//! - Use `fragment.add(widget)` for composition at any level.

use std::rc::Rc;

use crate::terminal::SessionId;
use crate::ui::actions::UiAction;
use crate::ui::styles::Styles;
use crate::ui::styles::colors::Colors;
use crate::ui::tiles::TileRect;
use crate::ui::types::UiState;

/// A screen region requiring a custom wgpu render pass.
///
/// Produced by terminal pane widgets; consumed by the render loop
/// to draw wgpu terminal grids at the correct screen positions.
#[derive(Debug, Clone)]
pub struct GpuViewport {
    /// Terminal session to render in this viewport.
    pub session_id: SessionId,
    /// Logical pixel rect (converted to physical pixels by the renderer).
    pub rect: egui::Rect,
}

/// Aggregated response returned by widgets after rendering one frame.
#[derive(Default)]
pub struct Dirties {
    /// Actions the app should process after the frame completes.
    pub actions: Vec<UiAction>,
    /// Terminal viewport rects collected by the tiles widget.
    pub tile_rects: Vec<TileRect>,
    /// Commands submitted via prompt editors.
    pub commands: Vec<(SessionId, String)>,
    /// Screen rects that need custom wgpu rendering (terminal grids).
    pub gpu_viewports: Vec<GpuViewport>,
    /// Float tooltip request from this frame.
    pub float_tooltip: Option<crate::ui::types::TooltipRequest>,
    /// Debug-only: widget name that produced this `Dirties`.
    #[cfg(debug_assertions)]
    pub source: &'static str,
}

impl Dirties {
    /// Create a `Dirties` tagged with the type name of `T` (debug builds only).
    #[allow(unused_variables)]
    pub fn typed<T: ?Sized>() -> Self {
        Self {
            #[cfg(debug_assertions)]
            source: std::any::type_name::<T>(),
            ..Default::default()
        }
    }

    /// Merge another response into this one (for sequential widget rendering).
    pub fn merge(&mut self, other: Dirties) {
        #[cfg(debug_assertions)]
        if !other.actions.is_empty() || !other.commands.is_empty() {
            log::trace!(
                "[Dirties] merging from '{}': {} actions, {} cmds",
                other.source,
                other.actions.len(),
                other.commands.len(),
            );
        }
        self.actions.extend(other.actions);
        self.tile_rects.extend(other.tile_rects);
        self.commands.extend(other.commands);
        self.gpu_viewports.extend(other.gpu_viewports);
    }

    #[inline]
    pub fn collect_actions<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = UiAction>,
    {
        self.actions.extend(iter);
    }

    #[inline]
    pub fn collect_rects<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = TileRect>,
    {
        self.tile_rects.extend(iter);
    }

    #[inline]
    pub fn collect_commands<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = (SessionId, String)>,
    {
        self.commands.extend(iter);
    }

    #[inline]
    pub fn collect_viewports<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = GpuViewport>,
    {
        self.gpu_viewports.extend(iter);
    }

    pub fn mount_tooltip<I>(&mut self, tooltip: I)
    where
        I: Into<Option<crate::ui::types::TooltipRequest>>,
    {
        self.float_tooltip = tooltip.into();
    }
}

// ─── Painter: Type-Level Marker ───────────────────────────────────────

/// Maps a fragment level to its reference storage type via a GAT.
///
/// - `egui::Context` → `&'a egui::Context` (panel-level, shared reference)
/// - `egui::Ui` → `&'a mut egui::Ui` (inline/pane-level, exclusive reference)
pub trait Painter {
    type Ref<'a>: 'a
    where
        Self: 'a;
}

impl Painter for egui::Context {
    type Ref<'a> = &'a egui::Context;
}

impl Painter for egui::Ui {
    type Ref<'a> = &'a mut egui::Ui;
}

// ─── Fragment: Unified Rendering Context ─────────────────────────────────────

/// Unified rendering context passed to every widget.
///
/// Generic over `T: FragmentTarget`:
/// - `Fragment<egui::Context>` (default) — panel-level, holds `&egui::Context`
/// - `Fragment<egui::Ui>` — inline/pane-level, holds `&mut egui::Ui`
pub struct Fragment<'a, T: Painter + 'a = egui::Context> {
    /// Theme colors (resolved from config or defaults).
    pub colors: Rc<Colors>,
    /// Typography, spacing, colors.
    pub styles: &'a Styles,
    /// Transient UI state (focus, tabs, filter, scroll positions).
    pub ui_state: &'a mut UiState,
    /// Per-widget effect accumulator (actions, tile rects, commands, viewports).
    pub dirties: &'a mut Dirties,
    /// The fragment compositor — `&egui::Context` or `&mut egui::Ui`.
    pub(crate) painter: <T as Painter>::Ref<'a>,
}

pub struct Context<'a> {
    /// Theme colors (resolved from config or defaults).
    pub colors: Rc<Colors>,
    /// Typography, spacing, colors.
    pub styles: &'a Styles,
    /// Transient UI state (focus, tabs, filter, scroll positions).
    pub ui_state: &'a mut UiState,
    /// Per-widget effect accumulator (actions, tile rects, commands, viewports).
    pub dirties: &'a mut Dirties,
}

impl<'a> Context<'a> {
    /// Create a child fragment pointing at a different `&mut egui::Ui`,
    /// sharing styles/ui_state/dirties from this fragment.
    pub fn bind<'u, T: Painter + 'u>(&'u mut self, ui: <T as Painter>::Ref<'u>) -> Fragment<'u, T> {
        Fragment::<T> {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: &mut *self.ui_state,
            dirties: &mut *self.dirties,
            painter: ui,
        }
    }
}

impl<'a> std::ops::Deref for Fragment<'a, egui::Context> {
    type Target = egui::Context;
    fn deref(&self) -> &Self::Target {
        self.painter
    }
}

impl<'a> std::ops::Deref for Fragment<'a, egui::Ui> {
    type Target = egui::Ui;
    fn deref(&self) -> &Self::Target {
        self.painter
    }
}
impl<'a> std::ops::DerefMut for Fragment<'a, egui::Ui> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.painter
    }
}
// ── Shared methods (both fragment variants) ─────────────────────────────────

impl<'a, T: Painter + 'a> Fragment<'a, T> {
    /// Forward `children` into a `Fragment<egui::Ui>` wrapping the given `&mut egui::Ui`.
    ///
    /// Use inside widget `show` to inject children into an egui layout closure:
    /// ```ignore
    /// egui::TopBottomPanel::top("id").show(ctx, |ui| {
    ///     fragment.render_children(ui, children);
    /// });
    /// ```
    pub fn render(&mut self, ui: &mut egui::Ui, f: impl FnOnce(&mut Fragment<'_, egui::Ui>)) {
        let mut child = Fragment::<egui::Ui> {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: &mut *self.ui_state,
            dirties: &mut *self.dirties,
            painter: ui,
        };
        f(&mut child);
    }

    pub fn split(self) -> (<T as Painter>::Ref<'a>, Context<'a>) {
        (
            self.painter,
            Context {
                colors: Rc::clone(&self.colors),
                styles: self.styles,
                ui_state: self.ui_state,
                dirties: self.dirties,
            },
        )
    }
}

// ── Fragment<egui::Context> — panel-level ────────────────────────────────────

impl<'a> Fragment<'a> {
    /// Create a new panel-level fragment from individual components.
    pub fn new(
        ctx: &'a egui::Context,
        colors: Rc<Colors>,
        styles: &'a Styles,
        ui_state: &'a mut UiState,
        dirties: &'a mut Dirties,
    ) -> Self {
        Self {
            colors,
            styles,
            ui_state,
            dirties,
            painter: ctx,
        }
    }

    /// Execute a rendering tree. Returns the collected `Dirties`.
    ///
    /// Replaces the old `Framing` struct — creates a root panel-level fragment,
    /// runs the closure, and returns all accumulated effects.
    pub fn frame(
        ctx: &egui::Context,
        colors: Rc<Colors>,
        styles: &Styles,
        ui_state: &mut UiState,
        f: impl FnOnce(&mut Fragment<'_>),
    ) -> Dirties {
        let mut dirties = Dirties::default();
        let mut frag = Fragment::new(ctx, colors, styles, ui_state, &mut dirties);
        f(&mut frag);
        dirties
    }

    /// The egui context for creating panels, areas, windows.
    ///
    /// Returns `&'a egui::Context` — independent of `&self` borrow,
    /// so you can call this while other fragment fields are mutably borrowed.
    pub fn ctx(&self) -> &'a egui::Context {
        self.painter
    }

    /// Create a child fragment with duplicated references.
    pub fn duplicate(&mut self) -> Fragment<'_> {
        Fragment {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: &mut *self.ui_state,
            dirties: &mut *self.dirties,
            painter: self.painter,
        }
    }

    /// Transition to an inline/pane-level fragment by attaching a `&mut egui::Ui`.
    pub fn with(self, ui: &'a mut egui::Ui) -> Fragment<'a, egui::Ui> {
        Fragment {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: self.ui_state,
            dirties: self.dirties,
            painter: ui,
        }
    }

    /// Render a panel `Widget` with no children.
    ///
    /// Creates `Dirties::typed::<W>()`, calls the widget's `show` method,
    /// and merges the child dirties into `self.dirties`.
    pub fn add<W, B>(&mut self, mut widget: B)
    where
        W: Widget,
        B: std::borrow::BorrowMut<W>,
    {
        let mut child_dirties = Dirties::typed::<W>();
        let child_fragment = Fragment {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: &mut *self.ui_state,
            dirties: &mut child_dirties,
            painter: self.painter,
        };
        widget.borrow_mut().render(child_fragment);
        self.dirties.merge(child_dirties);
    }

    /// Render a panel `Widget` with a `children` callback.
    ///
    /// The widget receives `children` and can invoke them inside its egui
    /// layout closure via `fragment.render(ui, children)`.
    pub fn add_with<W, B>(&mut self, mut widget: B, children: impl FnOnce(Fragment<'_, egui::Ui>))
    where
        W: Widget,
        B: std::borrow::BorrowMut<W>,
    {
        let mut child_dirties = Dirties::typed::<W>();
        let child_fragment = Fragment {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: &mut *self.ui_state,
            dirties: &mut child_dirties,
            painter: self.painter,
        };
        widget
            .borrow_mut()
            .render_with_children(child_fragment, children);
        self.dirties.merge(child_dirties);
    }
}

// ── Fragment<egui::Ui> — inline/pane-level ───────────────────────────────────

impl<'a> Fragment<'a, egui::Ui> {
    /// The egui context (derived from the inner Ui).
    pub fn ctx(&self) -> &egui::Context {
        self.painter.ctx()
    }

    /// Access the inner `&mut egui::Ui`.
    pub fn ui(&mut self) -> &mut egui::Ui {
        &mut *self.painter
    }

    /// Create a child fragment with duplicated references.
    pub fn duplicate(&mut self) -> Fragment<'_, egui::Ui> {
        Fragment {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: &mut *self.ui_state,
            dirties: &mut *self.dirties,
            painter: &mut *self.painter,
        }
    }

    /// Create a child fragment pointing at a different `&mut egui::Ui`,
    /// sharing styles/ui_state/dirties from this fragment.
    pub fn with<'u>(&'u mut self, ui: &'u mut egui::Ui) -> Fragment<'u, egui::Ui> {
        Fragment {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: &mut *self.ui_state,
            dirties: &mut *self.dirties,
            painter: ui,
        }
    }

    /// Dispatch an inline `Widget<egui::Ui>` with no children.
    ///
    /// Creates child `Dirties`, calls `widget.show()`, and merges
    /// the child dirties into `self.dirties`.
    pub fn add<W, B>(&mut self, mut widget: B)
    where
        W: Widget<egui::Ui>,
        B: std::borrow::BorrowMut<W>,
    {
        let mut child_dirties = Dirties::typed::<W>();
        let child_fragment = Fragment::<egui::Ui> {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: &mut *self.ui_state,
            dirties: &mut child_dirties,
            painter: &mut *self.painter,
        };
        widget.borrow_mut().render(child_fragment);
        self.dirties.merge(child_dirties);
    }

    /// Dispatch an inline `Widget<egui::Ui>` with a `children` callback.
    pub fn add_with<W, B>(&mut self, mut widget: B, children: impl FnOnce(Fragment<'_, egui::Ui>))
    where
        W: Widget<egui::Ui>,
        B: std::borrow::BorrowMut<W>,
    {
        let mut child_dirties = Dirties::typed::<W>();
        let child_fragment = Fragment::<egui::Ui> {
            colors: Rc::clone(&self.colors),
            styles: self.styles,
            ui_state: &mut *self.ui_state,
            dirties: &mut child_dirties,
            painter: &mut *self.painter,
        };
        widget
            .borrow_mut()
            .render_with_children(child_fragment, children);
        self.dirties.merge(child_dirties);
    }
}

// ─── Widget Traits ───────────────────────────────────────────────────────────

/// Unified widget trait for both panel-level and inline widgets.
///
/// - `impl Widget for X` → panel widget receiving `Fragment<egui::Context>`
/// - `impl Widget<egui::Ui> for X` → inline widget receiving `Fragment<egui::Ui>`
///
/// `children` provides content to render inside the widget's layout.
/// Call `fragment.render_children(ui, children)` inside an egui layout closure
/// to inject children into the widget's `Ui` (React-style composition).
pub trait Widget<T: Painter = egui::Context> {
    /// Paint this widget without children for one frame.
    #[inline]
    fn render<'a>(&mut self, #[allow(unused)] mut f: Fragment<'a, T>) {
        let (ui, ctx) = f.split();
        self.render_with_context(ui, ctx);
    }

    /// Paint this widget with splitted context for one frame.
    #[inline]
    fn render_with_context<'a>(
        &mut self,
        #[allow(unused)] ui: <T as Painter>::Ref<'a>,
        #[allow(unused)] mut ctx: Context<'a>,
    ) {
    }

    /// Paint this widget for one frame.
    #[inline]
    fn render_with_children<'a>(
        &mut self,
        f: Fragment<'a, T>,
        #[allow(unused)] children: impl FnOnce(Fragment<'a, egui::Ui>),
    ) {
        self.render(f)
    }
}
