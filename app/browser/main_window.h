#pragma once

#include <map>
#include <memory>
#include <string>
#include <vector>

#include "browser/client_handler.h"
#include "browser/space_manager.h"
#include "browser/tab_manager.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_label_button.h"
#include "include/views/cef_menu_button.h"
#include "include/views/cef_overlay_controller.h"
#include "include/views/cef_panel.h"
#include "include/views/cef_textfield.h"
#include "include/views/cef_window.h"
#include "runtime/crony_bridge.h"
#include "runtime/crony_proxy.h"

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

  // refine-ui-theme-layout: invoked by the macOS appearance observer
  // (NSDistributedNotificationCenter on AppleInterfaceThemeChanged) when
  // the system flips Light/Dark while the user is in `system` mode.
  // Re-resolves and re-broadcasts.
  void OnSystemAppearanceChanged();

 private:
  std::string ResourceUrl(const std::string& relative_path) const;
  void BuildChrome(CefRefPtr<CefWindow> window);

  // refine-ui-theme-layout: chrome color descriptor pushed to every
  // native surface. This mirrors the shell-relevant semantic token
  // subset from theme.css rather than the full renderer palette.
  struct ThemeChrome {
    cef_color_t bg_body;       // titlebar, sidebar, window background
    cef_color_t bg_base;       // content frame base surface
    cef_color_t bg_float;      // floating surfaces (mirrored to renderer)
    cef_color_t bg_mask;       // scrims/overlays (mirrored to renderer)
    cef_color_t border;        // 1 px content frame outline
    cef_color_t text_title;    // primary text on shell surfaces
    cef_color_t text_caption;  // secondary text on shell surfaces
  };
  // Compute the canonical chrome for a resolved appearance.
  static ThemeChrome ChromeFor(const std::string& resolved /*"light"|"dark"*/);
  // Push current_chrome_ to every native surface and broadcast
  // theme.changed to renderers.
  void ApplyThemeChrome(const ThemeChrome& chrome);
  // Returns "light"|"dark" — collapses theme_mode_ via system follow.
  std::string ResolveAppearance() const;
  // Invoked by HandleTheme via callback. Persists, recomputes, broadcasts.
  void HandleThemeModeChange(const std::string& mode);
  // Compose JSON used by both `theme.get` and `theme.changed`.
  std::string ThemeStateJson(bool include_chrome) const;

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

  // (task 4.2) Runtime bridge and proxy — owned at MainWindow scope since
  // they span the full application lifetime (one per process, shared by
  // all Spaces). Initialized in BuildChrome; nullptr until the runtime
  // binary is located and the handshake succeeds.
  std::unique_ptr<RuntimeBridge> runtime_bridge_;
  std::unique_ptr<RuntimeProxy>  runtime_proxy_;
  // arc-style-tab-cards (Phase 4+): TabManager owns the entire tab
  // universe. BrowserManager has been removed; per-kind *_view_ members
  // and SwitchToPanel have been removed (Phase 9).
  std::unique_ptr<TabManager> tabs_;

  // Layout views.
  CefRefPtr<CefBrowserView> sidebar_view_;   // web/public/sidebar.html
  CefRefPtr<CefPanel>       content_panel_;  // FillLayout, hosts active card
  // refine-ui-theme-layout: outer wrapper that paints the rounded 12 px
  // border around content_panel_. Inset by 8 px from body_panel_.
  CefRefPtr<CefPanel>       content_frame_;
  // Outer box with inside_border_insets providing the floating-card gap.
  // Needs window_bg color so the gap strips are visually visible.
  CefRefPtr<CefPanel>       content_outer_;
  // native-title-bar: root layout flipped from H to V; the body box hosts
  // the existing `[sidebar | content_outer]` row directly under the title
  // bar.
  CefRefPtr<CefPanel>        titlebar_panel_;
  CefRefPtr<CefPanel>        body_panel_;
  CefRefPtr<CefPanel>        lights_pad_;
  CefRefPtr<CefPanel>        spacer_;
  CefRefPtr<CefPanel>        win_pad_;
  CefRefPtr<CefMenuButton>   btn_space_;           // workspace selector dropdown
  CefRefPtr<CefLabelButton>  btn_sidebar_toggle_;  // hides/shows sidebar
  CefRefPtr<CefLabelButton>  btn_web_;
  CefRefPtr<CefLabelButton>  btn_term_;
  CefRefPtr<CefLabelButton>  btn_chat_;
  CefRefPtr<CefLabelButton>  btn_settings_;
  bool sidebar_visible_ = true;  // tracks current sidebar visibility
  // Toggle sidebar visibility (called by btn_sidebar_toggle_ press).
  void ToggleSidebar();

  // Native folder-picker delegate — set once in BuildChrome when ShellCallbacks
  // are wired, then called from the titlebar "Open Folder…" command.
  std::function<void(std::function<void(const std::string&)>)> run_file_dialog_;
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

  // Sidebar tab persistence: serialize chat/terminal tabs to SpaceStore kv.
  // RestoreSidebarTabs returns true if any tabs were restored, false if the
  // caller should open the default tab instead.
  void PersistSidebarTabs();
  bool RestoreSidebarTabs();

  // Popover (overlay inside the main window — Arc "Little Arc" style).
  CefRefPtr<CefBrowserView>      popover_view_;
  // Native chrome strip (URL toolbar + action buttons) as a CefPanel overlay.
  CefRefPtr<CefPanel>            popover_chrome_panel_;
  CefRefPtr<CefLabelButton>      popover_url_label_;  // read-only URL display inside panel
  CefRefPtr<CefLabelButton>      popover_btn_reload_;
  CefRefPtr<CefLabelButton>      popover_btn_open_tab_;
  CefRefPtr<CefLabelButton>      popover_btn_close_;
  std::string                    popover_current_url_; // last navigated URL for open-as-tab
  CefRefPtr<CefPanel>            popover_root_;
  CefRefPtr<CefOverlayController> popover_overlay_;
  CefRefPtr<CefOverlayController> popover_chrome_overlay_;
  CefRefPtr<CefWindow>           main_window_;
  int popover_owner_browser_id_ = 0;
  int popover_content_browser_id_ = 0;
  // True when the current popover is one of the bundled `panels/*` pages
  // (e.g. Settings). Those panels provide their own title bar, so the
  // native URL-bar chrome strip is suppressed.
  bool popover_is_builtin_ = false;
  void LayoutPopover();
  // Build the native popover chrome strip CefPanel (URL field + buttons).
  CefRefPtr<CefPanel> BuildPopoverChromePanel();

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
  // Arc-style: change the top and bottom insets of content_outer_ and force
  // re-layout. Used to vertically center the content card when a popover is open.
  void SetContentOuterVInsets(int top, int bottom);

  // refine-ui-theme-layout: persisted theme mode (`system|light|dark`)
  // and the most recently applied chrome (so subsequent paints can
  // short-circuit and the broadcast can include accurate hex colors).
  std::string theme_mode_ = "system";
  ThemeChrome current_chrome_{};
  // Opaque NSDistributedNotificationCenter observer token (macOS only).
  // Released by RemoveSystemAppearanceObserver in the destructor.
  void* appearance_observer_ = nullptr;

  IMPLEMENT_REFCOUNTING(MainWindow);
  DISALLOW_COPY_AND_ASSIGN(MainWindow);
};

}  // namespace cronymax
