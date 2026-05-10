# Native Views MVC — Architecture Design

> **Status**: Design complete, implementation pending. See `openspec/changes/native-views-mvc/` for full proposal, specs, and tasks.
>
> This document captures the architecture decisions from the explore phase. The goal is a pure structural refactor of `main_window.cc` (2285 lines → ~400) with zero runtime behavior change.

---

## Problem

`app/browser/main_window.cc` conflates six concerns in 2285 lines:

- Layout construction (`BuildChrome`, `BuildTitleBar`)
- Tab management (delegating to `TabManager`)
- Popover lifecycle (`OpenPopover`, `ClosePopover`, `LayoutPopover`)
- macOS platform workarounds (19 `#if __APPLE__` guards)
- Theme state (`ThemeChrome`, `ChromeFor`, `ResolveAppearance`)
- ShellCallbacks wiring (~600 lines of lambdas in `BuildChrome`)

The file is impossible to navigate and unsafe to modify in isolation.

---

## Target File Structure

```
app/browser/
  main_window.cc/.h           lifecycle only (~400 lines), implements context interfaces
  shell_model.cc/.h           TabManager, SpaceManager, theme state, observer bus
  shell_dispatcher.cc/.h      ShellCallbacks wiring (~600 lines extracted from BuildChrome)
  shell_observer.h            ShellObserver<EventT> template + ShellObserverList<EventT>
  shell_context.h             six narrow context interfaces

  views/
    titlebar_view.cc/.h       BuildTitleBar(), RefreshTitleBarDragRegion(), btn_* members
    sidebar_view.cc/.h        sidebar BrowserView, transparent styling
    content_view.cc/.h        content_outer/frame/panel, ShowActiveCard(), SetInsets()
    popover_ctrl.cc/.h        Open/Close/Layout, scrim management
    popover_overlay.cc/.h     composite of two fixed CEF overlay slots

  platform/
    view_style.h              unified interface — inline no-ops for non-Apple
    view_style_mac.mm         ~900 lines (-140: CornerPunchView deleted)
    view_style_win.cc         Windows no-op stubs
    view_style_linux.cc       Linux no-op stubs
```

**CMake:** `cmake/CronymaxApp.cmake` updated — all `views/*.cc`, all new `platform/*` files, old `mac_view_style.mm` removed.

---

## Architecture: MVC with Narrow Context Interfaces

```
┌─────────────────────────────────────────────────────────────────────┐
│  MainWindow  (implements all context interfaces, wires everything)  │
│                                                                     │
│  ┌─────────────┐   ┌──────────────────────────────────────────┐    │
│  │ ShellModel  │   │  Context Interfaces (all → MainWindow)   │    │
│  │             │   │                                          │    │
│  │ TabManager  │   │  ThemeContext        SpaceContext         │    │
│  │ SpaceManager│   │  TabsContext         ResourceContext      │    │
│  │ ThemeState  │   │  WindowActionContext OverlayActionContext  │    │
│  │ ObserverBus │   └──────────────────────────────────────────┘    │
│  └─────────────┘                                                    │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │  ShellDispatcher  (ShellCallbacks wiring)                    │  │
│  │  receives: TabsContext* SpaceContext* OverlayActionContext*   │  │
│  │            ResourceContext* ClientHandler*                   │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  Views (each receives only the interfaces it needs)                 │
│  ┌──────────────┐  ┌─────────────┐  ┌──────────────────────────┐  │
│  │ TitleBarView │  │ SidebarView │  │ ContentView              │  │
│  │ ThemeContext*│  │ ThemeContext*│  │ ThemeContext*            │  │
│  │ SpaceContext*│  │             │  │ TabsContext*              │  │
│  │ TabsContext* │  └─────────────┘  └──────────────────────────┘  │
│  │ WindowAction*│                                                   │
│  │ OverlayAct.* │  ┌─────────────────────────────────────────────┐ │
│  └──────────────┘  │ PopoverCtrl                                 │ │
│                    │ ThemeContext*  OverlayActionContext*         │ │
│                    │ owns: PopoverOverlay (two-slot composite)   │ │
│                    └─────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
```

The pattern follows the existing `TabContext` / `TabBehavior` precedent in `tab.h` — narrow interfaces injected via constructor, `MainWindow` implements them all.

---

## Context Interfaces

No `I` prefix — matches the `TabContext` precedent.

```cpp
// ThemeContext — any view that reacts to theme changes
class ThemeContext {
 public:
  virtual const ThemeChrome& current_chrome() const = 0;
  virtual void AddThemeObserver(ShellObserver<ThemeChanged>*) = 0;
  virtual void RemoveThemeObserver(ShellObserver<ThemeChanged>*) = 0;
};

// SpaceContext — TitleBarView, ShellDispatcher
class SpaceContext {
 public:
  virtual const std::vector<Space*>& spaces() const = 0;
  virtual Space* active_space() const = 0;
  virtual void SwitchSpace(const std::string& id) = 0;
  virtual void AddSpaceObserver(ShellObserver<SpaceChanged>*) = 0;
  virtual void RemoveSpaceObserver(ShellObserver<SpaceChanged>*) = 0;
};

// TabsContext — shell-level view of the tab collection
// Distinct from TabContext (per-TabBehavior injection — unchanged)
class TabsContext {
 public:
  virtual void OpenWebTab(const std::string& url) = 0;
  virtual std::string_view ActiveTabUrl() const = 0;
  virtual void AddTabObserver(ShellObserver<TabsChanged>*) = 0;
  virtual void RemoveTabObserver(ShellObserver<TabsChanged>*) = 0;
};

// WindowActionContext — TitleBarView
class WindowActionContext {
 public:
  virtual void ToggleSidebar() = 0;
  virtual void SetTitleBarDragRegion(const CefRect&) = 0;
};

// OverlayActionContext — ShellDispatcher, TitleBarView (settings btn)
class OverlayActionContext {
 public:
  virtual void OpenPopover(const std::string& url, int owner_id = 0) = 0;
  virtual void ClosePopover() = 0;
  virtual void ShowFloat(const std::string& url) = 0;
};

// ResourceContext — view constructors that load bundled URLs
class ResourceContext {
 public:
  virtual std::string ResourceUrl(const std::string& relative) const = 0;
};
```

Constructor signatures:

```cpp
TitleBarView(ThemeContext*, SpaceContext*, TabsContext*,
             WindowActionContext*, OverlayActionContext*)
SidebarView(ThemeContext*)
ContentView(ThemeContext*, TabsContext*)
PopoverCtrl(ThemeContext*, OverlayActionContext*, PopoverOverlay*)
ShellDispatcher(TabsContext*, SpaceContext*, OverlayActionContext*,
                ResourceContext*, ClientHandler*)
```

---

## Observer Pattern

`base::CheckedObserver` / `base::ObserverList` are **not** in the CEF distribution — confirmed by grep. Observer infrastructure is built from scratch following the same explicit-unsubscribe contract.

```cpp
template<typename EventT>
class ShellObserver {
 public:
  virtual ~ShellObserver() = default;
  virtual void OnShellEvent(const EventT&) = 0;
};

template<typename EventT>
class ShellObserverList {
  std::vector<ShellObserver<EventT>*> observers_;
 public:
  void Add(ShellObserver<EventT>* obs);
  void Remove(ShellObserver<EventT>* obs);
  void Notify(const EventT& e);      // snapshots before iterating
  ~ShellObserverList() { DCHECK(observers_.empty()); }  // mirrors CheckedObserver
};
```

Event structs (plain, no CefRefPtr members):

```cpp
struct ThemeChanged    { ThemeChrome chrome; };
struct SpaceChanged    { std::string new_id; std::string new_name; };
struct TabsChanged     {};
struct ActiveTabChanged{ std::string url; int browser_id; };
```

Views inherit multiple specializations:

```cpp
class TitleBarView : public ShellObserver<ThemeChanged>,
                     public ShellObserver<SpaceChanged> {
  ~TitleBarView() {
    theme_ctx_->RemoveThemeObserver(this);
    space_ctx_->RemoveSpaceObserver(this);
  }
  void OnShellEvent(const ThemeChanged& e) override { /* repaint */ }
  void OnShellEvent(const SpaceChanged& e) override { /* update label */ }
};
```

---

## Fixed Overlay Slots

### Why two overlays per popover (and why that can't change)

CEF `BrowserView` creates an IOSurface-backed CALayer that WindowServer composites **above all AppKit CA layers** within the same NSWindow. A `CefPanel` (the chrome strip) is AppKit-rendered. If both are in one overlay NSWindow, the BrowserView IOSurface is permanently on top regardless of view hierarchy order.

The solution is two separate overlay NSWindows. NSWindow z-order among siblings is applied by WindowServer above the IOSurface boundary — overlay NSWindow #2 (chrome strip) composites above overlay NSWindow #1 (content) unconditionally.

```
NSWindow z-order (WindowServer level):
  main window
  ├── overlay NSWindow #1  ← content BrowserView (IOSurface)
  └── overlay NSWindow #2  ← chrome CefPanel (AppKit-rendered, on top)

Within one NSWindow (cannot work):
  ├── chrome CefPanel       ← AppKit layer — hidden under IOSurface!
  └── BrowserView IOSurface ← always composites above AppKit layers
```

### CEF overlay z-order constraint

`CefOverlayController` has **no z-order API** — no `BringToFront`, `SetZOrder`, `OrderAbove`. Z-order is insertion-order only. This means z-order must be established permanently at startup.

### Fixed Slots topology

Three slots pre-allocated at `OnWindowCreated` → `BuildOverlaySlots()`:

```
startup (AddOverlayView call order = permanent z-order):

  slot [0]  popover content BrowserView    z: lowest
  slot [1]  popover chrome CefPanel        z: middle
  slot [2]  float BrowserView (reserved)   z: highest
             └── hidden until float feature exists

runtime (no AddOverlayView ever again):
  PopoverCtrl::Open(url)
    └── PopoverOverlay::Show(url, total_rect, with_chrome)
          slot[0]: SetBounds, LoadURL, SetVisible(true)
          slot[1]: SetBounds, SetVisible(true or false)

  PopoverCtrl::Close()
    └── PopoverOverlay::Hide()
          slot[0]: SetVisible(false)
          slot[1]: SetVisible(false)
```

**Benefit:** eliminates the per-open `CefPostTask`-deferred styling dance. The deferred task (needed because CEF defers `addChildWindow:` by one event loop tick) runs **once** at startup, not on every `Open()`.

### PopoverOverlay composite

`PopoverOverlay` encapsulates the two-slot detail behind a single-rect API. Callers never touch `CefOverlayController` directly.

```
PopoverOverlay::Show(url, total_rect, with_chrome=true):

  total_rect = {x, y, w, h}
  ┌──────────────────────────────┐  y
  │   chrome strip  (44px)       │  ← slot[1].SetBounds({x, y, w, 44})
  ├──────────────────────────────┤  y+44
  │                              │
  │   BrowserView content        │  ← slot[0].SetBounds({x, y+44, w, h-44})
  │                              │
  └──────────────────────────────┘  y+h

  Corner masks:
    web popover:   slot[0] → kCornerBottom,  slot[1] → kCornerTop
    builtin panel: slot[0] → kCornerAll,     slot[1] hidden

PopoverOverlay::Show(url, total_rect, with_chrome=false):

  total_rect = {x, y, w, h}
  ┌──────────────────────────────┐  y
  │                              │
  │   BrowserView content        │  ← slot[0].SetBounds({x, y, w, h})
  │   (all four corners rounded) │
  │                              │
  └──────────────────────────────┘  y+h
```

**Scrim stays in `PopoverCtrl`** — it covers the content card (background tab), not the popover, and its rect is computed from sidebar/titlebar dimensions independently:

```
PopoverCtrl::LayoutPopover():
  popover_overlay_->UpdateBounds(popover_rect)   ← two-slot management
  ShowPopoverScrim(card_rect)                     ← card, not popover
```

---

## Platform Abstraction

### Why `#if __APPLE__` is everywhere

Every call to `mac_view_style.h` functions is guarded. On non-Apple platforms the header doesn't compile, so callers must guard every call site.

### Solution: `platform/view_style.h`

```
Before:                              After:

  mac_view_style.h                     platform/view_style.h
    (Apple only, doesn't compile         inline no-ops for non-Apple
     on other platforms)                 extern forwarding for Apple

  main_window.cc                       main_window.cc
    #if __APPLE__                        StylePopoverContent(...)  // always safe
      StylePopoverContent(...)
    #endif
```

CMake selects the right translation unit:

```cmake
if(APPLE)
  list(APPEND SRCS platform/view_style_mac.mm)
elseif(WIN32)
  list(APPEND SRCS platform/view_style_win.cc)
else()
  list(APPEND SRCS platform/view_style_linux.cc)
endif()
```

### CornerPunchView → CAShapeLayer mask

`CronymaxCornerPunchView` (four NSViews painting chrome background color over corners to fake rounding) is deleted. CAShapeLayer masks work correctly on content BrowserViews for the same reason they work on overlay BrowserViews:

```
CAShapeLayer mask applied by WindowServer at blend time
    ↓ applied AFTER IOSurface compositor output
    ↓ clips both AppKit layers AND IOSurface output
    → correctly rounds BrowserView corners

cornerRadius + masksToBounds (the naive approach):
    ↓ applied to AppKit CA layer only
    ↓ does NOT clip IOSurface sublayers
    → corners not rounded on BrowserView content
```

`StyleContentBrowserView` is rewritten using the same walk-up + `CAShapeLayer` technique as `StyleOverlayBrowserView`.

---

## Migration Phases

Five phases, each independently reviewable with zero behavior change across all:

```
Phase 1: Platform         mac_view_style.mm → platform/view_style_mac.mm
                          platform/view_style.h (no-op stubs)
                          CornerPunchView deleted
                          StyleContentBrowserView → CAShapeLayer mask

Phase 2: ShellModel       Extract TabManager + SpaceManager + theme state
                          ShellObserverList instances live here

Phase 3: ShellDispatcher  Extract ShellCallbacks block from BuildChrome
                          ~600 lines → shell_dispatcher.cc

Phase 4: View extraction  PopoverCtrl (highest complexity, proves pattern)
                          → ContentView → TitleBarView → SidebarView

Phase 5: Observer wiring  Replace direct MainWindow method calls with
                          subscriptions where safe
                          Paint-only updates → subscriptions
                          Complex mutations → remain as direct calls
```

---

## Key Constraints (from codebase investigation)

| Constraint                                              | Source                                                           | Impact                                                                              |
| ------------------------------------------------------- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `CefOverlayController` has no z-order API               | `cef_overlay_controller.h`                                       | Fixed Slots topology required; z-order via insertion order only                     |
| BrowserView IOSurface composites above AppKit CA layers | `mac_view_style.mm` comments                                     | Two overlay NSWindows required for popover; chrome+content cannot share one overlay |
| `base::CheckedObserver` / `ObserverList` not in CEF     | grep confirms absence                                            | Custom `ShellObserverList<EventT>` required                                         |
| `window->GetBounds()` valid inside `OnWindowCreated`    | `GetPreferredSize()` returns `1440×920` before `OnWindowCreated` | Overlay slot creation is safe in `BuildOverlaySlots()`                              |
| CEF defers `addChildWindow:` by one event loop tick     | `mac_view_style.mm` + `CaptureLastChildNSView`                   | One-time `CefPostTask` in `BuildOverlaySlots()` for initial NSWindow styling        |
| `TabContext` / `TabBehavior` precedent already exists   | `tab.h`, `tab_behavior.h`                                        | Context interface pattern is established — no new pattern needed                    |
