// app/browser/views/titlebar_view.h
//
// native-views-mvc Phase 9: TitleBarView owns the native title-bar panel and
// all its child buttons.  Driven by ShellObserver<ThemeChanged> and
// ShellObserver<SpaceChanged>.
//
// MainWindow wires the Host callbacks, adds the root panel to the window
// layout, and delegates RefreshTitleBarDragRegion() / BuildTitleBar() to
// this class.

#pragma once

#include <functional>
#include <string>
#include <utility>
#include <vector>

#include "browser/models/view_observer.h"
#include "browser/models/view_context.h"
#include "include/views/cef_label_button.h"
#include "include/views/cef_menu_button.h"
#include "include/views/cef_panel.h"
#include "include/views/cef_window.h"

namespace cronymax {

class TitleBarView : public ViewObserver<ThemeChanged>,
                     public ViewObserver<SpaceChanged> {
 public:
  struct Host {
    // Returns all spaces as (id, name) pairs for the space-selector dropdown.
    std::function<std::vector<std::pair<std::string, std::string>>()> get_spaces;
    // Opens a new tab of the given kind ("web", "terminal", "chat").
    std::function<void(const std::string&)> open_new_tab;
    // Runs the native folder-picker dialog; calls `callback` with the chosen
    // path (empty string if the user cancelled).
    std::function<void(std::function<void(const std::string&)>)> run_file_dialog;
  };

  TitleBarView(SpaceContext* space,
               WindowActionContext* window_ctx,
               OverlayActionContext* overlay,
               ResourceContext* resources,
               CefRefPtr<CefWindow> main_win,
               Host host);
  ~TitleBarView() override;

  // Build and return the root titlebar CefPanel.  Called once during window
  // construction; the caller adds it to the window layout.
  CefRefPtr<CefPanel> Build();

  // Re-install the AppKit drag overlay above the title bar.  Must be called
  // after any layout change that repositions the title bar or its buttons.
  void RefreshDragRegion();

  // Update all button/panel background colors on a theme change.
  void ApplyTheme(const ThemeChrome& chrome);

  // Update the space-selector button label.
  void UpdateSpaceName(const std::string& name);

  // ShellObserver<ThemeChanged>
  void OnEvent(const ThemeChanged& e) override;
  // ShellObserver<SpaceChanged>
  void OnEvent(const SpaceChanged& e) override;

 private:
  SpaceContext*        space_ctx_;
  WindowActionContext* window_ctx_;
  OverlayActionContext* overlay_ctx_;
  ResourceContext*     resource_ctx_;
  CefRefPtr<CefWindow> main_win_;
  Host                 host_;

  // Child panels and buttons — owned by the CEF view tree once Build() adds
  // them to the panel.
  CefRefPtr<CefPanel>       titlebar_panel_;
  CefRefPtr<CefPanel>       lights_pad_;
  CefRefPtr<CefPanel>       spacer_;
  CefRefPtr<CefPanel>       win_pad_;
  CefRefPtr<CefMenuButton>  btn_space_;
  CefRefPtr<CefLabelButton> btn_sidebar_toggle_;
  CefRefPtr<CefLabelButton> btn_web_;
  CefRefPtr<CefLabelButton> btn_term_;
  CefRefPtr<CefLabelButton> btn_chat_;
  CefRefPtr<CefLabelButton> btn_settings_;
};

}  // namespace cronymax
