// app/browser/models/view_model.h
//
// ViewModel — owns all shared mutable state for the MVC views layer.
// Formerly ShellModel (now a type alias in the old shell_model.h).
//
// Holds:
//   - TabManager (the tab universe)
//   - SpaceManager (workspace lifecycle + SQLite store)
//   - Theme state (mode string, resolved chrome token set)
//   - Four ViewObserverList instances for the context-interface bus

#pragma once

#include <memory>
#include <string>

#include "browser/models/view_observer.h"
#include "browser/models/space_manager.h"
#include "browser/tab/tab_manager.h"

namespace cronymax {

class ViewModel {
 public:
  ViewModel();
  ~ViewModel() = default;

  // ── SpaceManager ────────────────────────────────────────────────────────
  SpaceManager space_manager_;

  // ── Theme state ─────────────────────────────────────────────────────────
  std::string theme_mode_ = "system";
  ThemeChrome current_chrome_{};

  // ── Observer lists ──────────────────────────────────────────────────────
  // NOTE: tabs_ is declared AFTER these lists so C++ destruction (reverse
  // order) destroys tabs_ first — allowing Tab::~ThemeAwareView() to safely
  // call RemoveThemeObserver() while theme_observers is still alive.
  ViewObserverList<ThemeChanged>     theme_observers;
  ViewObserverList<SpaceChanged>     space_observers;
  ViewObserverList<TabsChanged>      tabs_observers;
  ViewObserverList<ActiveTabChanged> active_tab_observers;

  // ── TabManager ──────────────────────────────────────────────────────────
  // Declared last so it is destroyed first, ensuring Tab ThemeAwareView
  // unsubscriptions happen before theme_observers is torn down.
  std::unique_ptr<TabManager> tabs_;

  // ── Theme helpers ────────────────────────────────────────────────────────
  static ThemeChrome ChromeFor(const std::string& resolved);
  std::string ResolveAppearance() const;
  std::string ThemeStateJson(bool include_chrome) const;

  // ── Observer notify helpers ──────────────────────────────────────────────
  void NotifyThemeChanged(const ThemeChrome& chrome);
  void NotifySpaceChanged(const std::string& id, const std::string& name);
  void NotifyTabsChanged();
  void NotifyActiveTabChanged(const std::string& url, int browser_id);

 private:
  ViewModel(const ViewModel&) = delete;
  ViewModel& operator=(const ViewModel&) = delete;
};

}  // namespace cronymax
