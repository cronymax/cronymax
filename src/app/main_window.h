#pragma once

#include <map>
#include <memory>
#include <string>
#include <vector>

#include "app/client_handler.h"
#include "app/space_manager.h"
#include "app/tab_manager.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_label_button.h"
#include "include/views/cef_overlay_controller.h"
#include "include/views/cef_panel.h"
#include "include/views/cef_textfield.h"
#include "include/views/cef_window.h"

namespace cronymax {

class MainWindow : public CefWindowDelegate,
                   public CefButtonDelegate,
                   public CefTextfieldDelegate {
 public:
  static void Create();

  MainWindow();

  void OnWindowCreated(CefRefPtr<CefWindow> window) override;
  void OnWindowDestroyed(CefRefPtr<CefWindow> window) override;
  bool CanClose(CefRefPtr<CefWindow> window) override;
  CefSize GetPreferredSize(CefRefPtr<CefView> view) override;
  cef_runtime_style_t GetWindowRuntimeStyle() override;

  void OnButtonPressed(CefRefPtr<CefButton> button) override;
  using CefWindowDelegate::OnKeyEvent;
  bool OnKeyEvent(CefRefPtr<CefTextfield> textfield,
                  const CefKeyEvent& event) override;
  void OnWindowBoundsChanged(CefRefPtr<CefWindow> window,
                             const CefRect& new_bounds) override;

 private:
  std::string ResourceUrl(const std::string& relative_path) const;
  void BuildChrome(CefRefPtr<CefWindow> window);

  // Open a new web tab navigating to `url`. Returns the new tab id (empty
  // on failure). Mounts/activates the tab in the content host.
  std::string OpenWebTab(const std::string& url);

  // Mount the active tab's card into the content host (no-op if already
  // mounted), hide every other tab card, and show the active one.
  void ShowActiveTab();

  // Open/close the popover overlay. `owner_browser_id` pairs the popover
  // with that tab; when the user switches to a different web tab, the
  // popover hides.
  void OpenPopover(const std::string& url, int owner_browser_id = 0);
  void ClosePopover();
  void UpdatePopoverVisibility();

  void PushToSidebar(const std::string& event_name,
                     const std::string& json_payload);
  // Broadcast an event to every chrome panel BrowserView. In the
  // tab-system world this means the sidebar plus every tab's content
  // browser (so tab renderers can react to lifecycle events).
  void BroadcastToAllPanels(const std::string& event_name,
                            const std::string& json_payload);
  static std::string JsEsc(const std::string& s);

  SpaceManager space_manager_;
  CefRefPtr<ClientHandler> client_handler_;
  // arc-style-tab-cards (Phase 4+): TabManager owns the entire tab
  // universe. BrowserManager has been removed; per-kind *_view_ members
  // and SwitchToPanel have been removed (Phase 9).
  std::unique_ptr<TabManager> tabs_;

  // Layout views.
  CefRefPtr<CefBrowserView> sidebar_view_;   // web/public/sidebar.html
  CefRefPtr<CefPanel>       content_panel_;  // FillLayout, hosts active card
  // native-title-bar: root layout flipped from H to V; the body box hosts
  // the existing `[sidebar | content_outer]` row directly under the title
  // bar.
  CefRefPtr<CefPanel>        titlebar_panel_;
  CefRefPtr<CefPanel>        body_panel_;
  CefRefPtr<CefPanel>        lights_pad_;
  CefRefPtr<CefPanel>        spacer_;
  CefRefPtr<CefPanel>        win_pad_;
  CefRefPtr<CefLabelButton>  btn_web_;
  CefRefPtr<CefLabelButton>  btn_term_;
  CefRefPtr<CefLabelButton>  btn_chat_;
  CefRefPtr<CefLabelButton>  btn_settings_;
  // Track which tab cards are mounted in `content_panel_` so we never
  // re-add the same CefView (which CEF rejects).
  std::map<std::string, bool> mounted_cards_;

  // 4.5: tab → SpaceStore row id, so we can update title and delete on
  // close. Keyed by TabId (string). Only web tabs are persisted today.
  std::map<std::string, int64_t> tab_db_ids_;
  // Snapshot of the last persisted title for each tab — used to skip
  // redundant UpdateTab calls in the SetOnChange tab snapshot pump.
  std::map<std::string, std::string> tab_persisted_titles_;
  // Persist a freshly-opened web tab to SpaceStore. Records the row id in
  // tab_db_ids_. No-op if there is no active Space.
  void PersistTabCreated(const std::string& tab_id,
                         const std::string& url,
                         const std::string& title);
  // Best-effort title sync: walks current web tabs and writes any title
  // that has changed since the last persist. Cheap when nothing changed.
  void PersistTabTitlesIfChanged();
  // Remove a tab row from SpaceStore. No-op if the tab was never persisted.
  void PersistTabClosed(const std::string& tab_id);

  // Popover (overlay inside the main window — Arc "Little Arc" style).
  CefRefPtr<CefBrowserView>      popover_view_;
  // Native chrome strip (rounded toolbar with URL field + actions). The
  // strip is a CefPanel hosted as an overlay above the content view; it
  // replaced the legacy HTML chrome BrowserView in arc-style-tab-cards.
  CefRefPtr<CefPanel>            popover_chrome_panel_;
  CefRefPtr<CefTextfield>        popover_url_field_;
  CefRefPtr<CefPanel>            popover_root_;
  CefRefPtr<CefOverlayController> popover_overlay_;
  CefRefPtr<CefOverlayController> popover_chrome_overlay_;
  CefRefPtr<CefWindow>           main_window_;
  int popover_owner_browser_id_ = 0;
  int popover_content_browser_id_ = 0;
  void LayoutPopover();
  // Build the native popover chrome panel (URL field + action buttons).
  // Stores `popover_url_field_` for later URL push-down.
  CefRefPtr<CefPanel> BuildPopoverChromePanel(const std::string& initial_url);
  // Navigate the popover content view to whatever the URL field holds.
  void NavigatePopoverToFieldUrl();

  // native-title-bar: build the top title-bar panel
  // (lights pad | spacer | btn_web | btn_term | btn_chat | win pad).
  CefRefPtr<CefPanel> BuildTitleBar();
  // native-title-bar: open a new tab of `kind` ("web"|"terminal"|"chat")
  // and broadcast shell.tab_created. Mirrors the sh.new_tab_kind bridge
  // path for the title-bar buttons.
  void OpenNewTabKind(const std::string& kind);
  // native-title-bar: (re)install the macOS AppKit drag overlay above the
  // title-bar spacer so dragging from that strip moves the window.
  void RefreshTitleBarDragRegion();

  IMPLEMENT_REFCOUNTING(MainWindow);
  DISALLOW_COPY_AND_ASSIGN(MainWindow);
};

}  // namespace cronymax
