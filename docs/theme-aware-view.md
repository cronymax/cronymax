# Theme-Aware View Pattern

This document describes the `ThemeAwareView` mixin and the typed
`ViewObserver` bus that deliver shell theme changes to every native CEF
view and tab component without coupling them to `MainWindow`.

---

## Problem

Before this refactor, `MainWindow::ApplyThemeChrome` manually called
`ApplyTheme(...)` on every view and iterated the full tab list:

```cpp
// Old: MainWindow had to know every subscriber
void MainWindow::ApplyThemeChrome(const ThemeChrome& chrome) {
  titlebar_view_->ApplyTheme(chrome.bg_body, chrome.text_title, ...);
  sidebar_view_->ApplyTheme(chrome.bg_body);
  content_view_->ApplyTheme(chrome.bg_body, chrome.bg_base);
  popover_ctrl_->ApplyTheme(chrome);
  for (auto& tab : shell_model_.tabs_->Snapshot())  // ← O(n) manual loop
    tab->ApplyTheme(chrome.bg_base, chrome.bg_float, chrome.text_title);
}
```

Problems:

- **Coupling**: `MainWindow` must know every consumer of theme data.
- **Brittleness**: adding a new component means editing `ApplyThemeChrome`.
- **Inconsistent signatures**: each view had its own subset of `ThemeChrome`
  fields passed as positional arguments.
- **Initial seeding gap**: views needed a separate call at construction time
  to paint their first frame, separate from the subscription.

---

## Solution: `ViewObserver<EventT>` bus + `ThemeAwareView` mixin

### Layer 1 — typed observer bus (`view_observer.h`)

```
ViewObserver<EventT>          ViewObserverList<EventT>
─────────────────────         ──────────────────────────────────────
virtual OnViewObserved(E&)    AddObserver / RemoveObserver
                              Notify(E&) → snapshots list, then calls
                                ObserverList::Notify(
                                  &ViewObserver<E>::OnViewObserved, e)
```

Four event types defined in the same header:

| Event struct       | Payload              | Notified by                    |
| ------------------ | -------------------- | ------------------------------ |
| `ThemeChanged`     | `ThemeChrome chrome` | `MainWindow::ApplyThemeChrome` |
| `SpaceChanged`     | `new_id`, `new_name` | space switch                   |
| `TabsChanged`      | _(empty)_            | tab open/close                 |
| `ActiveTabChanged` | `url`, `browser_id`  | tab activation                 |

`ThemeChrome` carries one field per CSS design token:

```cpp
struct ThemeChrome {
  cef_color_t bg_body;       // --color-cronymax-bg-body
  cef_color_t bg_base;       // --color-cronymax-bg-base
  cef_color_t bg_float;      // --color-cronymax-bg-float
  cef_color_t bg_mask;       // --color-cronymax-bg-mask
  cef_color_t border;        // --color-cronymax-border
  cef_color_t text_title;    // --color-cronymax-text-title
  cef_color_t text_caption;  // --color-cronymax-text-xs
};
```

### Layer 2 — `ThemeAwareView` mixin (`models/theme_aware_view.h`)

```cpp
class ThemeAwareView : public ViewObserver<ThemeChanged> {
public:
  void Register(ThemeContext* ctx);          // subscribe + seed immediately
  virtual void ApplyTheme(const ThemeChrome&) = 0;

protected:
  ~ThemeAwareView();                         // auto-unsubscribes
  ThemeContext* ThemeCtx() const;
};
```

`Register(ctx)`:

1. Stores `ctx`.
2. Calls `ctx->AddThemeObserver(this)`.
3. Calls `ApplyTheme(ctx->GetCurrentChrome())` **immediately** — so the
   view is painted correctly on its first frame without a separate seeding call.

`~ThemeAwareView()`:

- Calls `ctx->RemoveThemeObserver(this)` if registered. Safe even when the
  view is destroyed before `MainWindow` (the common case during tab close).

---

## Subscriber map

```
ThemeContext (MainWindow)
│
│  AddThemeObserver / RemoveThemeObserver
│  GetCurrentChrome() → ThemeChrome
│
├── TitleBarView         : ThemeAwareView + ViewObserver<SpaceChanged>
│     ApplyTheme → titlebar panel bg
│
├── SidebarView          : ThemeAwareView
│     ApplyTheme → sidebar BrowserView bg
│
├── ContentView          : ThemeAwareView + ViewObserver<ActiveTabChanged>
│     ApplyTheme → content outer/frame panel bg
│
├── PopoverCtrl          : ThemeAwareView
│     ApplyTheme → chrome strip panel colors
│
└── (per tab, created dynamically by TabManager::Open)
    │
    ├── Tab              : ThemeAwareView
    │     ApplyTheme → card panel bg, clears page-chrome override
    │
    ├── TabToolbar       : ThemeAwareView
    │     ApplyTheme → root/leading/middle/trailing panel bg (bg_base)
    │
    └── TabBehavior      : ThemeAwareView
          WebTabBehavior::ApplyTheme → url_field bg/fg, button fg/icon tints
          SimpleTabBehavior          → (no-op; HasToolbar=false, no widgets)
```

---

## Registration sequence for a new tab

```
TabManager::Open(kind, params)
  └── tab->Build(theme_ctx_)
        ├── TabToolbar::Build(theme_ctx_)
        │     └── Register(theme_ctx_)       ← toolbar panels seeded
        ├── behavior_->BuildToolbar(...)     ← url_field_, buttons created
        ├── behavior_->Register(theme_ctx_)  ← url_field_ / button fg seeded
        ├── behavior_->BuildContent(...)     ← BrowserView created
        └── Tab::Register(theme_ctx_)        ← card bg seeded
```

Order matters: `Register` must come **after** the CEF child views it
targets are created, otherwise `ApplyTheme` runs on null handles.

---

## `ThemeContext` interface (`view_context.h`)

```cpp
class ThemeContext {
 public:
  virtual ThemeChrome GetCurrentChrome() const = 0;
  virtual void AddThemeObserver(ViewObserver<ThemeChanged>*) = 0;
  virtual void RemoveThemeObserver(ViewObserver<ThemeChanged>*) = 0;
};
```

`MainWindow` implements all six context interfaces (`ThemeContext`,
`SpaceContext`, `TabsContext`, `WindowActionContext`,
`OverlayActionContext`, `ResourceContext`). Views receive only the
interface(s) they need — never a pointer to `MainWindow` itself.

---

## ViewModel member ordering constraint

`ViewModel` owns both the `ViewObserverList<ThemeChanged>` (observer list)
and the `TabManager` (which owns `Tab`s, which are `ThemeAwareView`s).

C++ destroys members in **reverse declaration order**. `Tab::~ThemeAwareView`
calls `RemoveThemeObserver(this)` — which requires `theme_observers` to still
be alive. Therefore `tabs_` must be declared **after** `theme_observers`:

```cpp
struct ViewModel {
  // Observer lists — declared first, destroyed last.
  ViewObserverList<ThemeChanged>     theme_observers;
  ViewObserverList<SpaceChanged>     space_observers;
  ViewObserverList<TabsChanged>      tabs_observers;
  ViewObserverList<ActiveTabChanged> active_tab_observers;

  // TabManager — declared last, destroyed first.
  // Tab::~ThemeAwareView calls RemoveThemeObserver() during TabManager
  // destruction; theme_observers must still be alive at that point.
  std::unique_ptr<TabManager> tabs_;
};
```

Reversing this order causes a DCHECK / use-after-free during teardown.

---

## Multiple `ViewObserver` bases and `-Woverloaded-virtual`

A class may inherit both `ThemeAwareView` (which provides a `final`
`OnViewObserved(const ThemeChanged&)`) and a second
`ViewObserver<X>` (which requires `OnViewObserved(const X&)`).
Clang's `-Woverloaded-virtual` (promoted to error) treats this as an
ambiguity because the two overloads have different signatures but the
same name.

Fix: add `using ThemeAwareView::OnViewObserved;` in the derived class to
make both overloads visible:

```cpp
class TitleBarView : public ThemeAwareView,
                     public ViewObserver<SpaceChanged> {
  using ThemeAwareView::OnViewObserved;           // ← required
  void OnViewObserved(const SpaceChanged&) override;
  void ApplyTheme(const ThemeChrome&) override;
};

class ContentView : public ThemeAwareView,
                    public ViewObserver<ActiveTabChanged> {
  using ThemeAwareView::OnViewObserved;           // ← required
  void OnViewObserved(const ActiveTabChanged&) override;
  void ApplyTheme(const ThemeChrome&) override;
};
```

---

## Page-driven chrome override (`SetChromeTheme`)

Web tabs can push a toolbar background color extracted from the loaded
page (via `tab.set_chrome_theme` bridge + injected JS). This is a
_local override_ layered on top of the shell theme:

```
Shell theme change (ThemeChanged event)
  → Tab::ApplyTheme(chrome)
      chrome_override_.clear()           ← discard stale page chrome
      card_->SetBackgroundColor(bg_base)
      toolbar_->SetChromeColor("")       ← restore toolbar to theme color
      behavior_->ApplyTheme(chrome)      ← update widget fg/bg

Page drives a color (SetChromeTheme)
  → Tab::SetChromeTheme(css_color)
      toolbar_->SetChromeColor(css)      ← page color overrides toolbar
      behavior_->ApplyTheme(page_chrome) ← derive fg from page bg luminance
                                           (TextColorForBg → light/dark text)
```

A full theme switch always clears the page override; the page will re-push
on its next navigation event.

---

## Simplified `ApplyThemeChrome` (MainWindow)

After the migration `MainWindow::ApplyThemeChrome` is minimal:

```cpp
void MainWindow::ApplyThemeChrome(const ThemeChrome& chrome) {
  // 1. Paint the window body panel (only piece MainWindow owns directly).
  body_panel_->SetBackgroundColor(chrome.bg_body);
  // 2. Persist in ViewModel for GetCurrentChrome() queries.
  shell_model_.current_chrome_ = chrome;
  // 3. Broadcast — all ThemeAwareView subscribers update themselves.
  shell_model_.theme_observers.Notify(ThemeChanged{chrome});
}
```

No manual per-view calls. No tab iteration loop.
