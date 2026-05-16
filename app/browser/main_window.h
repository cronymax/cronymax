#pragma once

#include <map>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "browser/client_handler.h"
#include "browser/models/profile_context_manager.h"
#include "browser/models/view_context.h"
#include "browser/models/view_dispatcher.h"
#include "browser/models/view_model.h"
#include "browser/views/content_view.h"
#include "browser/views/popover.h"
#include "browser/views/profile_picker_overlay.h"
#include "browser/views/sidebar_view.h"
#include "browser/views/titlebar_view.h"
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
                   public CefTextfieldDelegate,
                   public ThemeContext,
                   public SpaceContext,
                   public TabsContext,
                   public WindowActionContext,
                   public OverlayActionContext,
                   public ResourceContext {
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
  void BuildChrome(CefRefPtr<CefWindow> window);
  // Pre-allocate fixed overlay slots (Phase 6). Called at end of BuildChrome.
  void BuildOverlaySlots();

  // native-views-mvc: ThemeChrome is now the global struct from
  // shell_observer.h. This alias preserves all existing references to
  // MainWindow::ThemeChrome.
  using ThemeChrome = ::ThemeChrome;
  // Push current_chrome_ to every native surface and broadcast
  // theme.changed to renderers.
  void ApplyThemeChrome(const ThemeChrome& chrome);
  // Invoked by HandleTheme via callback. Persists, recomputes, broadcasts.
  void HandleThemeModeChange(const std::string& mode);

  // Open a new web tab: implements TabsContext::OpenWebTab. Also called
  // internally. Returns the new tab id (empty on failure).
  // (declaration via context override below)

  // Open/close the popover overlay: implements OverlayActionContext.
  // (declarations via context overrides below)
  void UpdatePopoverVisibility();

  void PushToSidebar(const std::string& event_name,
                     const std::string& json_payload);
  // Broadcast an event to every chrome panel BrowserView. In the
  // tab-system world this means the sidebar plus every tab's content
  // browser (so tab renderers can react to lifecycle events).
  void BroadcastToAllPanels(const std::string& event_name,
                            const std::string& json_payload);

  // native-views-mvc Phase 4: shared state (TabManager, SpaceManager, theme,
  // observer lists) lives in ShellModel. Declared before client_handler_ so
  // the member initializer list can pass &shell_model_.space_manager_ to
  // ClientHandler's constructor.
  ViewModel shell_model_;
  // native-views-mvc Phase 5: ShellDispatcher — owns the ShellCallbacks wiring
  // block. Declared before client_handler_ so it is destroyed AFTER
  // client_handler_ (C++ destroys in reverse-declaration order), ensuring the
  // callbacks stored in ClientHandler are already released before the
  // dispatcher's captured pointers become invalid.
  std::unique_ptr<ViewDispatcher> dispatcher_;
  CefRefPtr<ClientHandler> client_handler_;

  // (task 4.2) Runtime bridge and proxy — owned at MainWindow scope since
  // they span the full application lifetime (one per process, shared by
  // all Spaces). Initialized in BuildChrome; nullptr until the runtime
  // binary is located and the handshake succeeds.
  std::unique_ptr<RuntimeBridge> runtime_bridge_;
  std::unique_ptr<RuntimeProxy> runtime_proxy_;

  // Per-profile CefRequestContext manager.  Initialized in BuildChrome once
  // app_data_dir is known.  Used to wire profile-scoped webview storage into
  // tab browser views and overlay browser views.
  std::unique_ptr<ProfileContextManager> profile_ctx_manager_;
  // arc-style-tab-cards: TabManager lives in shell_model_.tabs_.

  // Layout views.
  // native-views-mvc Phase 10: sidebar owned by SidebarView.
  std::unique_ptr<SidebarView> sidebar_view_obj_;
  // Convenience accessor — returns browser_view_ from sidebar_view_obj_.
  // Code using sidebar_view_ directly is migrated in Phase 10.
  CefRefPtr<CefBrowserView> sidebar_view() const {
    return sidebar_view_obj_ ? sidebar_view_obj_->browser_view() : nullptr;
  }
  // native-views-mvc Phase 8: content panels + card management owned by
  // ContentView.
  std::unique_ptr<ContentView> content_view_;
  // native-views-mvc Phase 9: title bar owned by TitleBarView.
  std::unique_ptr<TitleBarView> titlebar_view_;
  // native-title-bar: root layout flipped from H to V; the body box hosts
  // the existing `[sidebar | content_outer]` row directly under the title bar.
  CefRefPtr<CefPanel>
      titlebar_panel_;  // root panel returned by TitleBarView::Build()
  CefRefPtr<CefPanel> body_panel_;
  bool sidebar_visible_ = true;  // tracks current sidebar visibility
  // ToggleSidebar: implements WindowActionContext::ToggleSidebar.
  // (declaration via context override below)

  // Native folder-picker delegate — set once in BuildChrome when ShellCallbacks
  // are wired, then called from the titlebar "Open Folder…" command.
  std::function<void(std::function<void(const std::string&)>)> run_file_dialog_;
  // (mounted_cards_ moved to ContentView in Phase 8)
  // (titlebar buttons moved to TitleBarView in Phase 9)

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

  // unified-toolbar: merged popover lifecycle + overlay management.
  // Constructed by BuildOverlaySlots().
  std::unique_ptr<Popover> popover_;
  // workspace-with-profile D9: native profile picker dialog.
  // Constructed by BuildOverlaySlots().
  std::unique_ptr<ProfilePickerOverlay> profile_picker_overlay_;

  CefRefPtr<CefWindow> main_window_;

  // native-title-bar: build the top title-bar panel
  // native-title-bar: open a new tab of `kind` ("web"|"terminal"|"chat")
  // and broadcast shell.tab_created. Mirrors the sh.new_tab_kind bridge
  // path for the title-bar buttons.
  void OpenNewTabKind(const std::string& kind);
  // Phase 9: RefreshTitleBarDragRegion() → TitleBarView::RefreshDragRegion().
  void RefreshTitleBarDragRegion();
  // Delegates to content_view_->SetVInsets(). Used by UpdatePopoverVisibility
  // and PopoverCtrl::Host to create breathing room around the content card.
  void SetContentOuterVInsets(int top, int bottom);

  // Opaque NSDistributedNotificationCenter observer token (macOS only).
  // Released by RemoveSystemAppearanceObserver in the destructor.
  void* appearance_observer_ = nullptr;
  // native-views-mvc Phase 4: theme_mode_, current_chrome_, tabs_,
  // space_manager_, and all four ShellObserverLists live in shell_model_.

  // native-views-mvc Phase 3: ThemeContext implementation.
  ThemeChrome GetCurrentChrome() const override;
  void AddThemeObserver(ViewObserver<ThemeChanged>* obs) override;
  void RemoveThemeObserver(ViewObserver<ThemeChanged>* obs) override;

  // native-views-mvc Phase 3: SpaceContext implementation.
  std::string GetCurrentSpaceId() const override;
  std::string GetCurrentSpaceName() const override;
  std::vector<std::pair<std::string, std::string>> GetSpaces() const override;
  void SwitchSpace(const std::string& space_id) override;
  void AddSpaceObserver(ViewObserver<SpaceChanged>* obs) override;
  void RemoveSpaceObserver(ViewObserver<SpaceChanged>* obs) override;

  // native-views-mvc Phase 3: TabsContext implementation.
  std::string GetActiveTabUrl() const override;
  std::string OpenWebTab(const std::string& url) override;
  void AddTabsObserver(ViewObserver<TabsChanged>* obs) override;
  void RemoveTabsObserver(ViewObserver<TabsChanged>* obs) override;
  void AddActiveTabObserver(ViewObserver<ActiveTabChanged>* obs) override;
  void RemoveActiveTabObserver(ViewObserver<ActiveTabChanged>* obs) override;

  // native-views-mvc Phase 3: WindowActionContext implementation.
  void ToggleSidebar() override;
  void SetTitleBarDragRegion(const CefRect& rect) override;

  // native-views-mvc Phase 3: OverlayActionContext implementation.
  // Default owner_browser_id = 0 preserved for internal single-arg call sites.
  void OpenPopover(const std::string& url, int owner_browser_id = 0) override;
  void ClosePopover() override;
  void ShowFloat(const std::string& url) override;
  void DismissFloat() override;

  // native-views-mvc Phase 3: ResourceContext implementation.
  std::string ResourceUrl(const std::string& relative) const override;

  IMPLEMENT_REFCOUNTING(MainWindow);
  DISALLOW_COPY_AND_ASSIGN(MainWindow);
};

}  // namespace cronymax
