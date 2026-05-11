// app/browser/models/theme_aware_view.h
//
// ThemeAwareView — abstract mixin for CEF native views that respond to theme
// changes via the ViewObserver pattern.
//
// Usage:
//   1. Inherit ThemeAwareView (alongside any other ViewObserver<> bases).
//   2. Implement `void ApplyTheme(const ThemeChrome& chrome) override`.
//   3. Call `Register(theme_ctx)` at the end of the view's Build() method.
//      Register subscribes to the observer list and immediately seeds colors
//      by calling ApplyTheme(GetCurrentChrome()) — so CEF views must exist.
//
// Destructor automatically unsubscribes from the ThemeContext observer list.
// Safe for views that are destroyed before MainWindow (normal teardown order).

#pragma once

#include "browser/models/view_context.h"
#include "browser/models/view_observer.h"

namespace cronymax {

class ThemeAwareView : public ViewObserver<ThemeChanged> {
public:
  // ViewObserver<ThemeChanged>: final — delegates to ApplyTheme.
  void OnViewObserved(const ThemeChanged &e) final { ApplyTheme(e.chrome); }

  // Subscribe to theme changes and immediately seed with the current chrome.
  // Must be called after CEF child views exist (typically end of Build()).
  virtual void Register(ThemeContext *ctx) {
    theme_ctx_ = ctx;
    theme_ctx_->AddThemeObserver(this);
    ApplyTheme(theme_ctx_->GetCurrentChrome());
  }

  // Override to apply ThemeChrome color tokens to native CEF views.
  virtual void ApplyTheme(const ThemeChrome &chrome) = 0;

protected:
  ~ThemeAwareView() override {
    if (theme_ctx_)
      theme_ctx_->RemoveThemeObserver(this);
  }

  // Accessor for derived classes that need the ThemeContext after Register().
  ThemeContext *ThemeCtx() const { return theme_ctx_; }

private:
  ThemeContext *theme_ctx_ = nullptr;
};

} // namespace cronymax
