// Copyright (c) 2026.
//
// TabBehavior — abstract per-kind interface. Concrete behaviors live in
// src/app/tab_behaviors/ and are added in later phases (3+). This header
// declares only the contract.

#pragma once

#include "browser/tab.h"
#include "include/views/cef_view.h"

namespace cronymax {

class TabToolbar;

class TabBehavior {
 public:
  virtual ~TabBehavior() = default;

  virtual TabKind Kind() const = 0;

  // Called once during Tab::Build with the freshly-constructed (empty)
  // toolbar. Implementations populate `leading`, `middle`, `trailing` slots.
  // `context` outlives the behavior.
  virtual void BuildToolbar(TabToolbar* toolbar, TabContext* context) = 0;

  // Construct the content view (typically a CefBrowserView). Called once
  // during Tab::Build. The returned view becomes the only child of the
  // tab's content host (FillLayout).
  virtual CefRefPtr<CefView> BuildContent(TabContext* context) = 0;

  // Apply a renderer-pushed toolbar state. Default no-op so behaviors that
  // don't push state (or haven't been migrated yet) need not override.
  virtual void ApplyToolbarState(const ToolbarState& /*state*/) {}

  // Optional: return the CEF browser identifier for this behavior's primary
  // browser, or 0 if it does not host a browser yet (or is not a browser-
  // backed kind). Used by MainWindow to pair browser events to the owning
  // Tab during the BrowserManager → TabManager migration.
  virtual int BrowserId() const { return 0; }
};

}  // namespace cronymax
