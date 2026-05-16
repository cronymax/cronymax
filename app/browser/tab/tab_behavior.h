// Copyright (c) 2026.
//
// TabBehavior — abstract per-kind interface. Concrete behaviors live in
// src/app/tab_behaviors/ and are added in later phases (3+). This header
// declares only the contract.

#pragma once

#include "browser/models/theme_aware_view.h"
#include "browser/tab/tab.h"
#include "include/views/cef_view.h"

namespace cronymax {

class TabToolbar;

class TabBehavior : public ThemeAwareView {
 public:
  ~TabBehavior() override = default;

  virtual TabKind Kind() const = 0;

  // Return false to suppress the toolbar panel entirely. When false,
  // BuildToolbar is not called and no toolbar height is reserved so the
  // content view fills the full card. Defaults to true (web tabs have a
  // toolbar; builtin panels such as Chat/Terminal/Settings do not).
  virtual bool HasToolbar() const { return true; }

  // Called once during Tab::Build with the freshly-constructed (empty)
  // toolbar. Implementations populate `leading`, `middle`, `trailing` slots.
  // Only called when HasToolbar() returns true.
  // `context` outlives the behavior.
  virtual void BuildToolbar(TabToolbar* toolbar, TabContext* context) = 0;

  // Construct the content view (typically a CefBrowserView). Called once
  // during Tab::Build. The returned view becomes the only child of the
  // tab's content host (FillLayout).
  virtual CefRefPtr<CefView> BuildContent(TabContext* context) = 0;

  // Apply a renderer-pushed toolbar state. Default no-op so behaviors that
  // don't push state (or haven't been migrated yet) need not override.
  virtual void ApplyToolbarState(const ToolbarState& /*state*/) {}

  // Called whenever the shell theme changes so the behavior can update any
  // hardcoded widget colors. Default no-op for behaviors with no owned widgets.
  // Implementations should also handle page-driven chrome calls (SetChromeTheme
  // may call this with a synthesized ThemeChrome derived from the page color).
  void ApplyTheme(const ThemeChrome& /*chrome*/) override {}

  // Optional: return the CEF browser identifier for this behavior's primary
  // browser, or 0 if it does not host a browser yet (or is not a browser-
  // backed kind). Used by MainWindow to pair browser events to the owning
  // Tab during the BrowserManager → TabManager migration.
  virtual int BrowserId() const { return 0; }
};

}  // namespace cronymax
