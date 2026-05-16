// app/browser/models/view_context.h
//
// Context interfaces partitioning MainWindow's surface area for views.
// Views receive narrowly-scoped pointers to these; they MUST NOT include
// main_window.h.
//
// Six pure-abstract interfaces:
//   ThemeContext         — read current chrome, observe theme changes
//   SpaceContext         — read/switch spaces, observe space changes
//   TabsContext          — open/query tabs, observe tab changes
//   WindowActionContext  — window-level actions (sidebar, drag region)
//   OverlayActionContext — popover / float overlay actions
//   ResourceContext      — resource URL resolution

#pragma once

#include <string>
#include <utility>
#include <vector>

#include "browser/models/view_observer.h"
#include "include/internal/cef_types_wrappers.h"

namespace cronymax {

// ---------------------------------------------------------------------------
// ThemeContext
// ---------------------------------------------------------------------------

class ThemeContext {
 public:
  virtual ThemeChrome GetCurrentChrome() const = 0;
  virtual void AddThemeObserver(ViewObserver<ThemeChanged>* obs) = 0;
  virtual void RemoveThemeObserver(ViewObserver<ThemeChanged>* obs) = 0;

 protected:
  virtual ~ThemeContext() = default;
};

// ---------------------------------------------------------------------------
// SpaceContext
// ---------------------------------------------------------------------------

class SpaceContext {
 public:
  virtual std::string GetCurrentSpaceId() const = 0;
  virtual std::string GetCurrentSpaceName() const = 0;
  virtual std::vector<std::pair<std::string, std::string>> GetSpaces()
      const = 0;
  virtual void SwitchSpace(const std::string& space_id) = 0;
  virtual void AddSpaceObserver(ViewObserver<SpaceChanged>* obs) = 0;
  virtual void RemoveSpaceObserver(ViewObserver<SpaceChanged>* obs) = 0;

 protected:
  virtual ~SpaceContext() = default;
};

// ---------------------------------------------------------------------------
// TabsContext
// ---------------------------------------------------------------------------

class TabsContext {
 public:
  virtual std::string GetActiveTabUrl() const = 0;
  virtual std::string OpenWebTab(const std::string& url) = 0;
  virtual void AddTabsObserver(ViewObserver<TabsChanged>* obs) = 0;
  virtual void RemoveTabsObserver(ViewObserver<TabsChanged>* obs) = 0;
  virtual void AddActiveTabObserver(ViewObserver<ActiveTabChanged>* obs) = 0;
  virtual void RemoveActiveTabObserver(ViewObserver<ActiveTabChanged>* obs) = 0;

 protected:
  virtual ~TabsContext() = default;
};

// ---------------------------------------------------------------------------
// WindowActionContext
// ---------------------------------------------------------------------------

class WindowActionContext {
 public:
  virtual void ToggleSidebar() = 0;
  virtual void SetTitleBarDragRegion(const CefRect& rect) = 0;

 protected:
  virtual ~WindowActionContext() = default;
};

// ---------------------------------------------------------------------------
// OverlayActionContext
// ---------------------------------------------------------------------------

class OverlayActionContext {
 public:
  virtual void OpenPopover(const std::string& url,
                           int owner_browser_id = 0) = 0;
  virtual void ClosePopover() = 0;
  virtual void ShowFloat(const std::string& url) = 0;
  virtual void DismissFloat() = 0;

 protected:
  virtual ~OverlayActionContext() = default;
};

// ---------------------------------------------------------------------------
// ResourceContext
// ---------------------------------------------------------------------------

class ResourceContext {
 public:
  virtual std::string ResourceUrl(const std::string& relative) const = 0;

 protected:
  virtual ~ResourceContext() = default;
};

}  // namespace cronymax
