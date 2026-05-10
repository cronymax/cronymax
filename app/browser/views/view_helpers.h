// app/browser/views/view_helpers.h
//
// native-views-mvc Phase 9: shared std::function-backed CEF delegate helpers.
// Include this in any TU that needs FnButtonDelegate, FnMenuButtonDelegate,
// FnMenuModelDelegate, or SizedPanelDelegate.  All are defined inline so
// each TU gets its own copy inside its anonymous namespace — the usual pattern
// for small, TU-local CEF helper classes.
//
// Usage:
//   namespace { // anonymous namespace in your .cc
//   #include "browser/views/view_helpers.h"  ← not recommended; instead:
//   } // end anonymous namespace
// Better: just include the header and reference the types — they don't have
// ODR issues since they're template-free and all definitions are identical.

#pragma once

#include <functional>
#include <string>

#include "include/cef_menu_model_delegate.h"
#include "include/views/cef_button_delegate.h"
#include "include/views/cef_menu_button.h"
#include "include/views/cef_panel.h"

namespace cronymax {

// ---------------------------------------------------------------------------
// SizedPanelDelegate — CefPanel that reports a fixed preferred size
// ---------------------------------------------------------------------------

class SizedPanelDelegate : public CefPanelDelegate {
 public:
  explicit SizedPanelDelegate(CefSize preferred_size)
      : preferred_size_(preferred_size) {}
  CefSize GetPreferredSize(CefRefPtr<CefView> view) override {
    (void)view;
    return preferred_size_;
  }

 private:
  CefSize preferred_size_;
  IMPLEMENT_REFCOUNTING(SizedPanelDelegate);
  DISALLOW_COPY_AND_ASSIGN(SizedPanelDelegate);
};

// ---------------------------------------------------------------------------
// FnButtonDelegate — CefButtonDelegate backed by a std::function
// ---------------------------------------------------------------------------

class FnButtonDelegate : public CefButtonDelegate {
 public:
  explicit FnButtonDelegate(std::function<void()> on_click)
      : on_click_(std::move(on_click)) {}
  void OnButtonPressed(CefRefPtr<CefButton>) override {
    if (on_click_) on_click_();
  }

 private:
  std::function<void()> on_click_;
  IMPLEMENT_REFCOUNTING(FnButtonDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnButtonDelegate);
};

// ---------------------------------------------------------------------------
// FnMenuButtonDelegate — CefMenuButtonDelegate backed by a std::function
// ---------------------------------------------------------------------------

class FnMenuButtonDelegate : public CefMenuButtonDelegate {
 public:
  using PressFn = std::function<void(CefRefPtr<CefMenuButton>,
                                     const CefPoint&,
                                     CefRefPtr<CefMenuButtonPressedLock>)>;
  explicit FnMenuButtonDelegate(PressFn fn) : fn_(std::move(fn)) {}
  void OnMenuButtonPressed(CefRefPtr<CefMenuButton> btn,
                           const CefPoint& pt,
                           CefRefPtr<CefMenuButtonPressedLock> lock) override {
    if (fn_) fn_(btn, pt, lock);
  }
  void OnButtonPressed(CefRefPtr<CefButton>) override {}

 private:
  PressFn fn_;
  IMPLEMENT_REFCOUNTING(FnMenuButtonDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnMenuButtonDelegate);
};

// ---------------------------------------------------------------------------
// FnMenuModelDelegate — CefMenuModelDelegate backed by a std::function
// ---------------------------------------------------------------------------

class FnMenuModelDelegate : public CefMenuModelDelegate {
 public:
  using ExecFn = std::function<void(int)>;
  explicit FnMenuModelDelegate(ExecFn fn) : fn_(std::move(fn)) {}
  void ExecuteCommand(CefRefPtr<CefMenuModel>, int cmd,
                      cef_event_flags_t) override {
    if (fn_) fn_(cmd);
  }

 private:
  ExecFn fn_;
  IMPLEMENT_REFCOUNTING(FnMenuModelDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnMenuModelDelegate);
};

}  // namespace cronymax
