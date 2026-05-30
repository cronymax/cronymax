#include "browser/main_window.h"
#include "browser/models/profile_context_manager.h"
#include "browser/models/view_model.h"
#include "browser/views/panel_window.h"

#include <algorithm>
#include <cctype>
#include <cstdlib>
#include <filesystem>
#include <functional>
#include <thread>
#include <utility>
#include <vector>

#include <nlohmann/json.hpp>

#include "include/base/cef_callback.h"
#include "include/cef_app.h"
#include "include/cef_menu_model.h"
#include "include/cef_menu_model_delegate.h"
#include "include/cef_parser.h"
#include "include/cef_path_util.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/views/cef_fill_layout.h"
#include "include/views/cef_menu_button.h"
#include "include/views/cef_panel_delegate.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"
#include "runtime/legacy_importer.h"

#include "browser/platform/view_style.h"

#if defined(__APPLE__)
#include "browser/icon_registry.h"
#include "browser/platform/mac_folder_picker.h"
#include "browser/tab/simple_tab_behavior.h"
#include "browser/tab/tab.h"
#include "browser/tab/tab_behavior.h"
#include "browser/tab/web_tab_behavior.h"
#endif

namespace cronymax {
namespace {

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

// Phase 10: SizedBrowserViewDelegate removed — now in sidebar_view.cc.

// Plain delegate that just forces Alloy runtime style for browser views
// hosted alongside other browser views in the same window.
class AlloyBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  AlloyBrowserViewDelegate() = default;
  cef_runtime_style_t GetBrowserRuntimeStyle() override {
    return CEF_RUNTIME_STYLE_ALLOY;
  }

 private:
  IMPLEMENT_REFCOUNTING(AlloyBrowserViewDelegate);
  DISALLOW_COPY_AND_ASSIGN(AlloyBrowserViewDelegate);
};

CefRefPtr<CefLabelButton> Button(CefButtonDelegate* delegate,
                                 const std::string& text) {
  return CefLabelButton::CreateLabelButton(delegate, text);
}
[[maybe_unused]] void EnsureButtonReferenced() {
  (void)&Button;
}

// Phase 8: kContentCornerRadius + RoundContentCorners() removed —
// now ContentView::RoundCornersFor().

// Small floating browser window used for popovers.
// (Legacy class kept compiling-only as a no-op; popovers are now overlays.)
class PopoverWindow : public CefWindowDelegate {
 public:
  PopoverWindow() = default;
  void OnWindowCreated(CefRefPtr<CefWindow>) override {}
  void OnWindowDestroyed(CefRefPtr<CefWindow>) override {}
  IMPLEMENT_REFCOUNTING(PopoverWindow);
  DISALLOW_COPY_AND_ASSIGN(PopoverWindow);
};

// std::function-backed CefButtonDelegate used for the native popover
// chrome (refresh / open-as-tab / close). Local to this TU.
class FnButtonDelegate : public CefButtonDelegate {
 public:
  explicit FnButtonDelegate(std::function<void()> on_click)
      : on_click_(std::move(on_click)) {}
  void OnButtonPressed(CefRefPtr<CefButton>) override {
    if (on_click_)
      on_click_();
  }

 private:
  std::function<void()> on_click_;
  IMPLEMENT_REFCOUNTING(FnButtonDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnButtonDelegate);
};

// std::function-backed CefMenuButtonDelegate — calls OnMenuButtonPressed
// handler then lets the handler call ShowMenu().
class FnMenuButtonDelegate : public CefMenuButtonDelegate {
 public:
  using PressFn = std::function<void(CefRefPtr<CefMenuButton>,
                                     const CefPoint&,
                                     CefRefPtr<CefMenuButtonPressedLock>)>;
  explicit FnMenuButtonDelegate(PressFn fn) : fn_(std::move(fn)) {}
  void OnMenuButtonPressed(CefRefPtr<CefMenuButton> btn,
                           const CefPoint& pt,
                           CefRefPtr<CefMenuButtonPressedLock> lock) override {
    if (fn_)
      fn_(btn, pt, lock);
  }
  void OnButtonPressed(CefRefPtr<CefButton>) override {}

 private:
  PressFn fn_;
  IMPLEMENT_REFCOUNTING(FnMenuButtonDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnMenuButtonDelegate);
};

// std::function-backed CefMenuModelDelegate for space-selector menu results.
class FnMenuModelDelegate : public CefMenuModelDelegate {
 public:
  using ExecFn = std::function<void(int)>;
  explicit FnMenuModelDelegate(ExecFn fn) : fn_(std::move(fn)) {}
  void ExecuteCommand(CefRefPtr<CefMenuModel>,
                      int cmd,
                      cef_event_flags_t) override {
    if (fn_)
      fn_(cmd);
  }

 private:
  ExecFn fn_;
  IMPLEMENT_REFCOUNTING(FnMenuModelDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnMenuModelDelegate);
};

// std::function-backed CefTextfieldDelegate for the popover URL textfield.
class FnTextfieldDelegate : public CefTextfieldDelegate {
 public:
  using KeyFn =
      std::function<bool(CefRefPtr<CefTextfield>, const CefKeyEvent&)>;
  explicit FnTextfieldDelegate(KeyFn fn) : fn_(std::move(fn)) {}
  bool OnKeyEvent(CefRefPtr<CefTextfield> tf, const CefKeyEvent& ev) override {
    return fn_ ? fn_(tf, ev) : false;
  }

 private:
  KeyFn fn_;
  IMPLEMENT_REFCOUNTING(FnTextfieldDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnTextfieldDelegate);
};

}  // namespace

// ---------------------------------------------------------------------------
// MainWindow
// ---------------------------------------------------------------------------

/*static*/ void MainWindow::Create() {
  CefWindow::CreateTopLevelWindow(new MainWindow());
}

MainWindow::MainWindow()
    : client_handler_(new ClientHandler(&shell_model_.space_manager_)) {}

void MainWindow::OnWindowCreated(CefRefPtr<CefWindow> window) {
  CEF_REQUIRE_UI_THREAD();
  window->SetTitle("cronymax");
  main_window_ = window;

  // Resources path — needed for builtin-doc-types regardless of where DB lives.
  CefString res_path;
  CefGetPath(PK_DIR_RESOURCES, res_path);

  // Store the database in the user-data directory so it survives app-bundle
  // rebuilds (the Chromium Framework Resources dir is wiped by
  // COPY_MAC_FRAMEWORK on every cmake build). Fall back to Resources if
  // user-data is unavailable.
  std::filesystem::path db_dir;
  CefString user_data_str;
  if (CefGetPath(PK_USER_DATA, user_data_str) && !user_data_str.empty()) {
    db_dir = std::filesystem::path(user_data_str.ToString());
  } else if (!res_path.empty()) {
    db_dir = res_path.ToString();
  } else {
    db_dir = std::filesystem::current_path();
  }

  if (!shell_model_.space_manager_.Init(db_dir / "cronymax.db"))
    LOG(ERROR) << "SpaceManager: failed to open database";

  // Phase A task 4.5: tell SpaceManager where the bundled built-in
  // doc-type YAMLs live so per-Space DocTypeRegistry can merge them with
  // workspace overrides.
  if (!res_path.empty()) {
    shell_model_.space_manager_.SetBuiltinDocTypesDir(
        std::filesystem::path(res_path.ToString()) / "builtin-doc-types");
    shell_model_.space_manager_.SetBuiltinFlowsDir(
        std::filesystem::path(res_path.ToString()) / "builtin-flows");
  }

  if (shell_model_.space_manager_.spaces().empty())
    shell_model_.space_manager_.CreateSpace(
        "Default", std::filesystem::current_path(), "default");

  // arc-style-tab-cards: TabManager owns every tab; per-kind *_view_
  // singletons are gone. All non-web kinds are singleton tabs whose
  // content browser loads the existing renderer HTML.
  shell_model_.tabs_ = std::make_unique<TabManager>(this, this);
  shell_model_.tabs_->SetClientHandler(client_handler_.get());
  // native-title-bar: terminal/chat are multi-instance now (each click of
  // "+ Terminal" / "+ Chat" creates a fresh tab). Agent/graph stay
  // singletons.
  shell_model_.tabs_->RegisterSingletonKind(TabKind::kSettings);
  shell_model_.tabs_->RegisterSingletonKind(TabKind::kActivity);
  shell_model_.tabs_->RegisterSingletonKind(TabKind::kFlows);
  shell_model_.tabs_->SetHiddenFromList(
      {TabKind::kActivity, TabKind::kFlows, TabKind::kExtensionView});

  // refine-ui-theme-layout: load persisted theme mode (defaults to
  // "system") and seed shell_model_.current_chrome_ before BuildChrome so the
  // title bar paints with the correct color on first frame.
  {
    std::string persisted =
        shell_model_.space_manager_.store().GetKv("ui.theme");
    if (persisted == "light" || persisted == "dark" || persisted == "system") {
      shell_model_.theme_mode_ = persisted;
    }
    shell_model_.current_chrome_ =
        ViewModel::ChromeFor(shell_model_.ResolveAppearance());
  }

  BuildChrome(window);

  // Restore persisted sidebar tabs (chat/terminal) from the previous session.
  // Falls back to opening a default Chat tab on first launch.
  if (!RestoreSidebarTabs()) {
    TabId id = shell_model_.tabs_->Open(TabKind::kChat, OpenParams{});
    // Tab::Register() in Open() already seeded colors — no ApplyTheme needed.
    if (!id.empty())
      shell_model_.tabs_->Activate(id);
  }
  // Restore web tabs from the previous session (lazy-load; no navigation until
  // each tab is first activated).
  RestoreWebTabs();

#if defined(__APPLE__)
  // Arc-style: translucent NSWindow with hidden title bar. Posted onto the
  // UI runner so the NSWindow is fully realized first.
  CefPostTask(TID_UI, base::BindOnce(
                          [](CefRefPtr<CefWindow> w, cef_color_t bg) {
                            StyleMainWindowTranslucent(w->GetWindowHandle(),
                                                       bg);
                          },
                          window, shell_model_.current_chrome_.bg_body));
  // refine-ui-theme-layout: install rounded 12 px frame + initial border
  // colour around the content panel. Posted so the NSView is realized.
  CefPostTask(TID_UI,
              base::BindOnce(
                  [](CefRefPtr<MainWindow> self) {
                    self->ApplyThemeChrome(self->shell_model_.current_chrome_);
                  },
                  CefRefPtr<MainWindow>(this)));
  // refine-ui-theme-layout: subscribe to the macOS appearance flip
  // notification so `system` mode tracks Light/Dark in real time.
  appearance_observer_ = AddSystemAppearanceObserver(
      [](void* user) {
        auto* self = reinterpret_cast<MainWindow*>(user);
        CefPostTask(TID_UI, base::BindOnce(
                                [](CefRefPtr<MainWindow> s) {
                                  s->OnSystemAppearanceChanged();
                                },
                                CefRefPtr<MainWindow>(self)));
      },
      this);
  // native-title-bar: install the AppKit drag overlay above the title-bar
  // spacer once the initial layout has run and the spacer has real bounds.
  CefPostTask(TID_UI, base::BindOnce(
                          [](CefRefPtr<MainWindow> self) {
                            self->RefreshTitleBarDragRegion();
                          },
                          CefRefPtr<MainWindow>(this)));
#endif

  shell_model_.space_manager_.SetSwitchCallback(
      [this](const std::string& old_id, const std::string& new_id) {
        // (task 4.2) Reconnect runtime event subscriptions for the new space.
        client_handler_->OnSpaceSwitch(old_id, new_id);
        // Phase 8: mounted_cards_ moved to ContentView. Card visibility is
        // driven by ContentView::OnShellEvent(ActiveTabChanged) which fires
        // via NotifyActiveTabChanged below. No explicit hide-all needed here.
        for (const auto& sp : shell_model_.space_manager_.spaces()) {
          if (sp->id == new_id) {
            PushToSidebar(
                "shell.space_changed",
                nlohmann::json{{"id", new_id}, {"name", sp->name}}.dump());
            // Phase 9: space button label updated via TitleBarView observer
            // (SpaceChanged event fires in shell_model_.NotifySpaceChanged).

            // Task 7.5: Update TabManager's request context so new tabs
            // opened after this switch get the new profile's webview storage.
            {
              const std::string pid =
                  sp->profile_id.empty() ? DEFAULT_PROFILE_ID : sp->profile_id;
              shell_model_.tabs_->SetRequestContext(
                  profile_ctx_manager_->GetContextForProfile(pid));
            }
            break;
          }
        }
      });

  // Wire runtime restart callback: on every space switch, restart the
  // Rust runtime with the new space's sandbox policy (design decision D4).
  shell_model_.space_manager_.SetRuntimeRestartCallback(
      [this](const std::string& workspace_root, const ProfileRecord& profile) {
        // Build the sandbox JSON that SpawnAndHandshake will inject.
        nlohmann::json sandbox;
        sandbox["workspace_root"] = workspace_root;
        sandbox["allow_network"] = profile.allow_network;
        {
          auto arr = nlohmann::json::array();
          for (const auto& p : profile.extra_read_paths)
            arr.push_back(p);
          sandbox["extra_read_paths"] = arr;
        }
        {
          auto arr = nlohmann::json::array();
          for (const auto& p : profile.extra_write_paths)
            arr.push_back(p);
          sandbox["extra_write_paths"] = arr;
        }
        {
          auto arr = nlohmann::json::array();
          for (const auto& p : profile.extra_deny_paths)
            arr.push_back(p);
          sandbox["extra_deny_paths"] = arr;
        }
        runtime_bridge_->SetSandboxConfig(sandbox);

        // Task 8.3: Update the profile context so that SpawnAndHandshake
        // computes the correct profile-scoped and workspace-scoped paths.
        runtime_bridge_->SetProfileContext(
            profile.id.empty() ? DEFAULT_PROFILE_ID : profile.id,
            profile.memory_id.empty()
                ? (profile.id.empty() ? DEFAULT_PROFILE_ID : profile.id)
                : profile.memory_id,
            std::filesystem::path(workspace_root));

        // Restart the runtime bridge on a background thread so the UI
        // thread is not blocked (the switch UX shows a loading indicator
        // until the new runtime is ready).
        std::thread([this]() {
          BroadcastToAllPanels("space.switch_loading", "{\"loading\":true}");
          runtime_bridge_->Stop();
          runtime_bridge_->Start();
          BroadcastToAllPanels("space.switch_loading", "{\"loading\":false}");
        }).detach();
      });

  window->Show();
}

void MainWindow::OnWindowDestroyed(CefRefPtr<CefWindow> window) {
  CEF_REQUIRE_UI_THREAD();
  (void)window;
  ClosePopover();
#if defined(__APPLE__)
  if (appearance_observer_) {
    RemoveSystemAppearanceObserver(appearance_observer_);
    appearance_observer_ = nullptr;
  }
#endif
  CefQuitMessageLoop();
}

bool MainWindow::CanClose(CefRefPtr<CefWindow> window) {
  (void)window;
  return true;
}

CefSize MainWindow::GetPreferredSize(CefRefPtr<CefView> view) {
  (void)view;
  return CefSize(1440, 920);
}

cef_runtime_style_t MainWindow::GetWindowRuntimeStyle() {
  return CEF_RUNTIME_STYLE_ALLOY;
}

// ---------------------------------------------------------------------------
// BuildChrome  —  Arc-style: [sidebar | content_panel] with the active
// tab's card mounted inside `content_panel_`. The topbar and per-kind
// *_view_ singletons have been removed (Phase 9).
// ---------------------------------------------------------------------------

void MainWindow::BuildChrome(CefRefPtr<CefWindow> window) {
  CefBrowserSettings web_settings;

  // native-title-bar: flip root layout from H to V; titlebar (fixed h) on
  // top, body (HBOX with [sidebar | content_outer]) below.
  CefBoxLayoutSettings root_box;
  root_box.horizontal = false;
  auto root_layout = window->SetToBoxLayout(root_box);

  // ── Title bar (Phase 9: owned by TitleBarView) ───────────────────────────
  {
    TitleBarView::Host tb_host;
    tb_host.get_spaces = [this]() { return GetSpaces(); };
    tb_host.open_new_tab = [this](const std::string& kind) {
      OpenNewTabKind(kind);
    };
    // run_file_dialog_ is assigned later in BuildChrome (after this block),
    // so capture by this-pointer and forward lazily at call time.
    tb_host.run_file_dialog =
        [this](std::function<void(const std::string&)> cb) {
          if (run_file_dialog_)
            run_file_dialog_(std::move(cb));
        };
    // profile_picker_overlay_ is constructed in BuildOverlaySlots() which runs
    // after BuildChrome, so capture by pointer and forward lazily.
    tb_host.show_profile_picker = [this](const std::string& path) {
      if (profile_picker_overlay_)
        profile_picker_overlay_->Show(path);
    };
    titlebar_view_ = std::make_unique<TitleBarView>(
        /*space=*/this, /*window_ctx=*/this, /*overlay=*/this,
        /*resources=*/this, /*theme_ctx=*/this, main_window_,
        std::move(tb_host));
    titlebar_panel_ = titlebar_view_->Build();
  }
  window->AddChildView(titlebar_panel_);
  root_layout->SetFlexForView(titlebar_panel_, 0);

  // ── Body row ─────────────────────────────────────────────────────────────
  body_panel_ = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings body_box;
  body_box.horizontal = true;
  auto body_layout = body_panel_->SetToBoxLayout(body_box);
  window->AddChildView(body_panel_);
  root_layout->SetFlexForView(body_panel_, 1);

  // ── Activity bar (leftmost vertical icon rail) ───────────────────────────
  // Added first so it sits left of the sidebar in the horizontal body row.
  activitybar_view_obj_ = std::make_unique<ActivityBarView>(
      /*resource_ctx=*/this, /*theme_ctx=*/this, client_handler_);
  {
    auto ab = activitybar_view_obj_->Build();
    body_panel_->AddChildView(ab);
    body_layout->SetFlexForView(ab, 0);
  }

  // ── Sidebar (Phase 10: owned by SidebarView) ─────────────────────────────
  // Activities / Flows opens now route from the ActivityBarView rail through
  // the `shell.tab_open_singleton` bridge channel (view_dispatcher.cc), so
  // SidebarView no longer needs host callbacks.
  sidebar_view_obj_ = std::make_unique<SidebarView>(
      /*resource_ctx=*/this, /*theme_ctx=*/this, client_handler_);
  auto sv = sidebar_view_obj_->Build();
  body_panel_->AddChildView(sv);
  body_layout->SetFlexForView(sv, 0);

  // ── Content host (Phase 8: owned by ContentView) ─────────────────────────
  {
    ContentView::Host cv_host;
    cv_host.active_tab = [this]() -> std::tuple<std::string, CefRefPtr<CefView>,
                                                CefRefPtr<CefBrowserView>> {
      Tab* active = shell_model_.tabs_ ? shell_model_.tabs_->Active() : nullptr;
      if (!active || !active->card())
        return {};
      CefRefPtr<CefBrowserView> bv;
      if (active->kind() == TabKind::kWeb) {
        if (auto* wb = static_cast<WebTabBehavior*>(active->behavior()))
          bv = wb->browser_view();
      } else {
        if (auto* sb = static_cast<SimpleTabBehavior*>(active->behavior()))
          bv = sb->browser_view();
      }
      return {active->tab_id(), active->card(), bv};
    };
    cv_host.refresh_drag_region = [this]() {
      // Deferred so CEF's NSView re-parenting completes first.
      CefPostTask(TID_UI, base::BindOnce(
                              [](CefRefPtr<MainWindow> self) {
                                self->RefreshTitleBarDragRegion();
                              },
                              CefRefPtr<MainWindow>(this)));
    };
    cv_host.request_focus = [this]() {
      Tab* active = shell_model_.tabs_ ? shell_model_.tabs_->Active() : nullptr;
      if (!active || active->kind() != TabKind::kWeb)
        return;
      if (auto* wb = static_cast<WebTabBehavior*>(active->behavior())) {
        // Lazy-load restored tabs on first activation.
        std::string pending = wb->TakePendingUrl();
        if (!pending.empty())
          wb->Navigate(pending);
        if (auto bv = wb->browser_view())
          bv->RequestFocus();
      }
    };
    cv_host.update_popover_visibility = [this]() { UpdatePopoverVisibility(); };
    content_view_ = std::make_unique<ContentView>(
        /*tabs=*/this, /*theme_ctx=*/this, main_window_, std::move(cv_host));
    auto content_outer = content_view_->Build();
    body_panel_->AddChildView(content_outer);
    body_layout->SetFlexForView(content_outer, 1);
  }

  // ── Right-side dock (rightmost, collapsible; target="right" views) ───────
  right_dock_view_obj_ =
      std::make_unique<RightDockView>(/*theme_ctx=*/this, client_handler_);
  {
    auto dock = right_dock_view_obj_->Build();
    body_panel_->AddChildView(dock);
    body_layout->SetFlexForView(dock, 0);
  }

  // ── native-views-mvc Phase 5: ShellDispatcher ───────────────────────────
  // Set up the DispatcherHost callbacks that provide access to MainWindow's
  // private members without exposing MainWindow* to ShellDispatcher.
  DispatcherHost disp_host;
  disp_host.push_to_sidebar = [this](const std::string& ev,
                                     const std::string& json) {
    PushToSidebar(ev, json);
  };
  disp_host.broadcast = [this](const std::string& ev, const std::string& json) {
    BroadcastToAllPanels(ev, json);
  };
  // Phase 8: show_active_tab removed from DispatcherHost — ContentView drives
  // card display via ShellObserver<ActiveTabChanged>.
  disp_host.persist_sidebar_tabs = [this]() { PersistSidebarTabs(); };
  disp_host.persist_tab_titles_if_changed = [this]() {
    PersistTabTitlesIfChanged();
  };
  disp_host.persist_tab_closed = [this](const std::string& id) {
    PersistTabClosed(id);
  };
  disp_host.remove_tab_card = [this](const std::string& tab_id) {
    if (content_view_)
      content_view_->RemoveCard(tab_id);
  };
  disp_host.get_popover_owner_browser_id = [this]() {
    return popover_ ? popover_->owner_browser_id() : 0;
  };
  disp_host.popover_reload = [this]() {
    if (popover_) {
      if (auto bv = popover_->content_view())
        if (auto b = bv->GetBrowser())
          b->Reload();
    }
  };
  disp_host.get_popover_url = [this]() -> std::string {
    if (!popover_)
      return {};
    if (auto bv = popover_->content_view())
      if (auto b = bv->GetBrowser())
        return b->GetMainFrame()->GetURL().ToString();
    return {};
  };
  disp_host.popover_navigate_url = [this](const std::string& url) {
    if (popover_) {
      if (auto bv = popover_->content_view())
        if (auto b = bv->GetBrowser())
          b->GetMainFrame()->LoadURL(url);
    }
  };
  disp_host.window_drag = [this]() {
#if defined(__APPLE__)
    if (main_window_)
      PerformWindowDrag(main_window_->GetWindowHandle());
#endif
  };
  disp_host.handle_theme_mode_change = [this](const std::string& mode) {
    CefPostTask(TID_UI, base::BindOnce(
                            [](CefRefPtr<MainWindow> self, std::string m) {
                              self->HandleThemeModeChange(m);
                            },
                            CefRefPtr<MainWindow>(this), mode));
  };
  // Wire run_file_dialog_ for the "Open Folder\u2026" titlebar command and the
  // space.open_folder bridge channel.  On macOS we use NSOpenPanel; the
  // callback is invoked on the main thread with the selected path (or ""
  // on cancel).
#if defined(__APPLE__)
  run_file_dialog_ = [](std::function<void(const std::string&)> cb) {
    ShowNativeFolderPicker(std::move(cb));
  };
#else
  run_file_dialog_ = [](std::function<void(const std::string&)> cb) {
    cb("");  // Not implemented on non-macOS platforms yet.
  };
#endif
  disp_host.run_file_dialog = run_file_dialog_;
  disp_host.notify_sidebar_active_kind = [this](const std::string& kind) {
    if (sidebar_view_obj_)
      sidebar_view_obj_->UpdateActiveButtonState(kind);
    PushActiveViewToRail();
  };
  disp_host.open_right_dock = [this](const std::string& view_key,
                                     const std::string& url,
                                     const std::string& title) {
    if (right_dock_view_obj_)
      right_dock_view_obj_->OpenOrToggle(view_key, url, title);
    // Reflow the body so the content area shrinks/expands for the dock.
    if (body_panel_)
      body_panel_->Layout();
    PushActiveViewToRail();
    // Round the dock's bottom-right corner once the browser + layout settle.
    CefPostTask(TID_UI, base::BindOnce(
                            [](CefRefPtr<MainWindow> self) {
                              if (self->right_dock_view_obj_)
                                self->right_dock_view_obj_->ApplyCornerRounding();
                            },
                            CefRefPtr<MainWindow>(this)));
  };
  disp_host.close_extension_views = [this](const std::string& ext_id) {
    CloseExtensionViews(ext_id);
  };

  dispatcher_ = std::make_unique<ViewDispatcher>(
      /*tabs_ctx=*/this, /*space_ctx=*/this,
      /*overlay_ctx=*/this, /*resource_ctx=*/this, client_handler_.get(),
      &shell_model_, std::move(disp_host));
  dispatcher_->Wire();

  // (task 4.2) Initialize the Rust runtime bridge and proxy. Start() finds
  // the cronymax-runtime binary in the app bundle, spawns it, and completes
  // the Hello/Welcome handshake. This is a best-effort async startup: if the
  // binary is missing or the handshake fails the app continues without the
  // runtime (degraded mode — all runtime-backed channels return 503).
  runtime_bridge_ = std::make_unique<RuntimeBridge>();
  runtime_proxy_ = std::make_unique<RuntimeProxy>();
  // (task 5.1 / 5.2) Run the one-shot legacy state importer before starting
  // the runtime so it sees the imported runs on its first load.
  // app_data_dir is declared here (outside the importer block) so the
  // runtime bridge start thread can capture it by value.
  //
  // PK_USER_DATA = CefSettings.root_cache_path = $appDataDir, so reading it
  // directly gives the real application data root.
  std::filesystem::path app_data_dir;
  {
    CefString _ud;
    if (CefGetPath(PK_USER_DATA, _ud) && !_ud.empty()) {
      app_data_dir = std::filesystem::path(_ud.ToString());
    } else {
      CefString _res;
      CefGetPath(PK_DIR_RESOURCES, _res);
      app_data_dir = (_res.empty() ? std::filesystem::current_path()
                                   : std::filesystem::path(_res.ToString()));
    }
  }
  {
    LegacyImporter importer(app_data_dir);
    if (!importer.AlreadyDone()) {
      std::vector<ImportSpaceInfo> space_infos;
      for (const auto& sp_ptr : shell_model_.space_manager_.spaces()) {
        ImportSpaceInfo info;
        info.space_id = sp_ptr->id;
        info.space_name = sp_ptr->name;
        info.workspace_root = sp_ptr->workspace_root;
        space_infos.push_back(std::move(info));
      }
      const auto res = importer.Run(space_infos);
      LOG(INFO) << "[LegacyImporter] import done: " << res.spaces_seeded
                << " spaces, " << res.runs_imported << " runs imported, "
                << res.runs_skipped << " already present, " << res.parse_errors
                << " parse errors";
    }
  }

  // Initialize the per-profile CefRequestContext manager and immediately
  // wire the initial profile's context into the TabManager so all tabs
  // opened at startup use the correct disk-backed webview storage.
  profile_ctx_manager_ = std::make_unique<ProfileContextManager>(app_data_dir);
  {
    const std::string init_pid = [this]() -> std::string {
      if (auto* sp = shell_model_.space_manager_.ActiveSpace())
        return sp->profile_id.empty() ? DEFAULT_PROFILE_ID : sp->profile_id;
      return DEFAULT_PROFILE_ID;
    }();
    shell_model_.tabs_->SetRequestContext(
        profile_ctx_manager_->GetContextForProfile(init_pid));
  }

  // Start the bridge on a background thread to avoid blocking the UI.
  // Set the profile context for the initial active space before starting.
  {
    const std::string init_profile_id = [this]() -> std::string {
      if (auto* sp = shell_model_.space_manager_.ActiveSpace())
        return sp->profile_id.empty() ? DEFAULT_PROFILE_ID : sp->profile_id;
      return DEFAULT_PROFILE_ID;
    }();
    const std::string init_memory_id = [this,
                                        &init_profile_id]() -> std::string {
      const auto maybe = shell_model_.space_manager_.profile_store().Get(
          init_profile_id.empty() ? DEFAULT_PROFILE_ID : init_profile_id);
      if (maybe && !maybe->memory_id.empty())
        return maybe->memory_id;
      return init_profile_id.empty() ? DEFAULT_PROFILE_ID : init_profile_id;
    }();
    const std::filesystem::path init_workspace_root =
        [this]() -> std::filesystem::path {
      if (auto* sp = shell_model_.space_manager_.ActiveSpace())
        return sp->workspace_root;
      return {};
    }();
    runtime_bridge_->SetProfileContext(init_profile_id, init_memory_id,
                                       init_workspace_root);
  }
  std::thread([this, app_data_dir]() {
    if (runtime_bridge_->Start({}, app_data_dir)) {
      runtime_proxy_->Attach(runtime_bridge_.get());
      // Wire the proxy to the bridge handler on the UI thread.
      CefPostTask(
          TID_UI,
          base::BindOnce(
              [](CefRefPtr<MainWindow> self) {
                self->client_handler_->SetRuntimeProxy(
                    self->runtime_proxy_.get());
                // Auto-subscribe to the initial active space's events.
                if (auto* sp =
                        self->shell_model_.space_manager_.ActiveSpace()) {
                  self->client_handler_->OnSpaceSwitch("", sp->id);
                }
              },
              CefRefPtr<MainWindow>(this)));
    } else {
      fprintf(stderr, "[MainWindow] RuntimeBridge failed to start: %s\n",
              runtime_bridge_->LastError().c_str());
    }
  }).detach();

  // ── Browser event callbacks (Phase 4: TabManager-routed) ──────────────
  // WebTabBehavior already registers per-browser listeners with
  // ClientHandler when its browser is realized, so it owns the toolbar UI
  // updates. The callbacks below are kept for cross-cutting concerns:
  // sidebar event mirroring and popover URL display.
  client_handler_->on_browser_created = [this](int browser_id) {
    // When the popover content browser is created, trigger LayoutPopover to
    // apply corner masks and scrim (GetBrowser() is now non-null on the
    // pre-allocated fixed slot).
    if (popover_ && popover_->IsOpen()) {
      if (auto bv = popover_->content_view()) {
        if (bv->GetBrowser() &&
            bv->GetBrowser()->GetIdentifier() == browser_id) {
          popover_->LayoutPopover();
        }
      }
    }
    // Round the content corners now that the browser (and its NSView tree)
    // is fully initialized. ContentView::ShowActiveCard posts the same call
    // but GetBrowser() is null on a brand-new tab at that point.
    Tab* t = shell_model_.tabs_
                 ? shell_model_.tabs_->FindByBrowserId(browser_id)
                 : nullptr;
    if (t) {
      CefRefPtr<CefBrowserView> bv;
      if (t->kind() == TabKind::kWeb) {
        if (auto* wb = static_cast<WebTabBehavior*>(t->behavior()))
          bv = wb->browser_view();
      } else {
        if (auto* sb = static_cast<SimpleTabBehavior*>(t->behavior()))
          bv = sb->browser_view();
      }
      ContentView::RoundCornersFor(bv, main_window_,
                                   shell_model_.current_chrome_.bg_body);
    }
    // If the overlay browser just finished async creation and there is a
    // pending URL queued from an OpenOverlay() call that arrived before
    // GetBrowser() became non-null, dispatch the navigation now.
    if (overlay_bv_ && overlay_bv_->GetBrowser() &&
        overlay_bv_->GetBrowser()->GetIdentifier() == browser_id &&
        !overlay_pending_url_.empty()) {
      overlay_bv_->GetBrowser()->GetMainFrame()->LoadURL(overlay_pending_url_);
      overlay_pending_url_.clear();
    }
  };

  client_handler_->on_title_change = [this](int browser_id,
                                            const std::string& title) {
    Tab* t = shell_model_.tabs_->FindByBrowserId(browser_id);
    if (!t)
      return;
    PushToSidebar("shell.tab_title_changed",
                  nlohmann::json{{"id", t->tab_id()}, {"title", title}}.dump());
  };

  client_handler_->on_address_change = [this](int browser_id,
                                              const std::string& url) {
    // Mirror popover content URL into the native chrome strip textfield.
    if (popover_ && popover_->IsOpen()) {
      if (auto bv = popover_->content_view()) {
        if (bv->GetBrowser() &&
            bv->GetBrowser()->GetIdentifier() == browser_id) {
          popover_->SetCurrentUrl(url);
          return;
        }
      }
    }
    Tab* t = shell_model_.tabs_->FindByBrowserId(browser_id);
    if (!t)
      return;
    PushToSidebar("shell.tab_url_changed",
                  nlohmann::json{{"id", t->tab_id()}, {"url", url}}.dump());
  };

  client_handler_->on_popup_request = [this](int browser_id,
                                             const std::string& url) -> bool {
    OpenPopover(url, browser_id);
    return true;  // suppress native popup
  };

  // DevTools: F12 or Cmd+Option+I shows the DevTools inspector for the
  // browser that received the key event (identified by browser_id).
  // Works for all tab kinds (kChat, kWeb, kSettings, kActivity, kFlows,
  // etc.) and panel windows.
  client_handler_->on_devtools_requested = [this](int browser_id) {
    CefRefPtr<CefBrowser> target;

    // 1. Look up the browser by id in the tab manager (all tab kinds).
    if (shell_model_.tabs_) {
      if (Tab* t = shell_model_.tabs_->FindByBrowserId(browser_id)) {
        CefRefPtr<CefBrowserView> bv;
        if (t->kind() == TabKind::kWeb) {
          if (auto* wb = static_cast<WebTabBehavior*>(t->behavior()))
            bv = wb->browser_view();
        } else {
          if (auto* sb = static_cast<SimpleTabBehavior*>(t->behavior()))
            bv = sb->browser_view();
        }
        if (bv)
          target = bv->GetBrowser();
      }
    }

    // 2. Fall back to panel windows (settings, flows, activities).
    if (!target) {
      for (const auto& bv : PanelWindow::AllBrowserViews()) {
        if (bv && bv->GetBrowser() &&
            bv->GetBrowser()->GetIdentifier() == browser_id) {
          target = bv->GetBrowser();
          break;
        }
      }
    }

    if (!target)
      return;
    CefWindowInfo wi;
    CefBrowserSettings bs;
    target->GetHost()->ShowDevTools(wi, nullptr, bs, CefPoint());
  };

#if defined(__APPLE__)
  // Forward CSS draggable-region updates from the sidebar AND from every
  // open PanelWindow to the native overlay. CEF Alloy does not honour
  // `-webkit-app-region` directly; it surfaces the rects via
  // OnDraggableRegionsChanged and we translate them into a transparent
  // NSView overlay (ApplyDraggableRegions). The sidebar uses this for its
  // top strip; PanelWindowHeader uses it so the user can drag a standalone
  // settings / flows / activities window from its header, since the
  // NSWindow's traffic-light strip is hidden under
  // NSWindowStyleMaskFullSizeContentView and the BrowserView covers the
  // entire content area (no NSWindow background pixels left for
  // movableByWindowBackground to act on).
  client_handler_->on_draggable_regions_changed =
      [this](int browser_id, const std::vector<CefDraggableRegion>& regions) {
        std::vector<DragRegion> rs;
        rs.reserve(regions.size());
        for (const auto& r : regions) {
          rs.push_back({r.bounds.x, r.bounds.y, r.bounds.width, r.bounds.height,
                        r.draggable != 0});
        }
        const DragRegion* data = rs.empty() ? nullptr : rs.data();

        // 1. Sidebar (in-window).
        if (auto _sv = sidebar_view()) {
          if (auto b = _sv->GetBrowser();
              b && b->GetIdentifier() == browser_id) {
            ApplyDraggableRegions(b->GetHost()->GetWindowHandle(), data,
                                  rs.size());
            return;
          }
        }

        // 2. Any open PanelWindow.
        if (auto handle = PanelWindow::LookupBrowserHandle(browser_id)) {
          ApplyDraggableRegions(handle, data, rs.size());
        }
      };
#endif
  // native-views-mvc Phase 6: pre-allocate fixed overlay slots.
  BuildOverlaySlots();
}

// ---------------------------------------------------------------------------
// Phase 6: fixed overlay slot pre-allocation
// ---------------------------------------------------------------------------

void MainWindow::BuildOverlaySlots() {
  // Slot 0: content BrowserView (lower z-order).
  // The overlay uses the global context (nullptr) — its URL navigations are
  // transient and do not need profile-scoped cookie persistence.
  CefBrowserSettings bs;
  bs.background_color = shell_model_.current_chrome_.bg_float != 0
                            ? shell_model_.current_chrome_.bg_float
                            : static_cast<cef_color_t>(0xFF1C1C1F);
  auto content_bv = CefBrowserView::CreateBrowserView(
      client_handler_, "about:blank", bs, nullptr, nullptr,
      new AlloyBrowserViewDelegate());
  auto content_oc = main_window_->AddOverlayView(
      content_bv, CEF_DOCKING_MODE_CUSTOM, /*can_activate=*/true);
  content_oc->SetVisible(false);

  // Slot 1: chrome CefPanel (higher z-order — added AFTER content so it
  // is above the content overlay in WindowServer z-order).
  auto chrome_panel = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(0, Popover::kChromeH)));
  chrome_panel->SetBackgroundColor(bs.background_color);
  auto chrome_oc = main_window_->AddOverlayView(
      chrome_panel, CEF_DOCKING_MODE_CUSTOM, /*can_activate=*/true);
  chrome_oc->SetVisible(false);

  Popover::Host phost;
  phost.open_web_tab = [this](const std::string& url) { OpenWebTab(url); };
  phost.set_content_insets = [this](int top, int bottom) {
    SetContentOuterVInsets(top, bottom);
  };
  phost.refresh_drag_region = [this]() { RefreshTitleBarDragRegion(); };
  phost.close_notify = nullptr;
  phost.get_sidebar_width = [this]() -> int {
    return sidebar_visible_ ? 240 : 0;
  };
  popover_ = std::make_unique<Popover>(
      /*theme_ctx=*/this, content_bv, content_oc, chrome_panel, chrome_oc,
      main_window_, std::move(phost));

  // One-time deferred styling: CEF defers addChildWindow: by one event-loop
  // tick, so the overlay NSWindow is not yet attached synchronously.
  CefPostTask(TID_UI,
              base::BindOnce(
                  [](CefRefPtr<CefBrowserView> bv, CefRefPtr<CefWindow> w,
                     cef_color_t bg) {
                    // Content slot: all-corners mask (will be overridden on
                    // first Show).
                    StyleOverlayBrowserView(
                        bv->GetBrowser()
                            ? bv->GetBrowser()->GetHost()->GetWindowHandle()
                            : nullptr,
                        12.0, kCornerAll, /*with_shadow=*/true);
                    // Chrome slot: top-corners mask + background.
                    void* main_nsv =
                        reinterpret_cast<void*>(w->GetWindowHandle());
                    void* nsview = CaptureLastChildNSView(main_nsv);
                    if (nsview) {
                      StyleOverlayPanel(nsview, 12.0, kCornerTop, bg);
                      SetOverlayWindowBackground(nsview, 0x00000000);
                    }
                  },
                  content_bv, main_window_,
                  shell_model_.current_chrome_.bg_float != 0
                      ? shell_model_.current_chrome_.bg_float
                      : static_cast<cef_color_t>(0xFF182625)));

  // ── Slot 2: OVERLAY BrowserView (z2 — Settings modal) ──────────────────
  overlay_bv_ = CefBrowserView::CreateBrowserView(
      client_handler_, "about:blank", bs, nullptr, nullptr,
      new AlloyBrowserViewDelegate());
  overlay_oc_ = main_window_->AddOverlayView(
      overlay_bv_, CEF_DOCKING_MODE_CUSTOM, /*can_activate=*/true);
  overlay_oc_->SetVisible(false);
  // Deferred: apply all-corner rounding + shadow to the OVERLAY NSWindow.
  CefPostTask(TID_UI,
              base::BindOnce(
                  [](CefRefPtr<CefBrowserView> bv) {
                    StyleOverlayBrowserView(
                        bv->GetBrowser()
                            ? bv->GetBrowser()->GetHost()->GetWindowHandle()
                            : nullptr,
                        12.0, kCornerAll, /*with_shadow=*/true);
                  },
                  overlay_bv_));

  // ── Slot 3: FLOAT BrowserView (z3 — contextual float panels) ────────────
  float_bv_ = CefBrowserView::CreateBrowserView(client_handler_, "about:blank",
                                                bs, nullptr, nullptr,
                                                new AlloyBrowserViewDelegate());
  float_oc_ = main_window_->AddOverlayView(float_bv_, CEF_DOCKING_MODE_CUSTOM,
                                           /*can_activate=*/true);
  float_oc_->SetVisible(false);

  // ── Profile picker overlay (workspace-with-profile D9) ──────────────────
  ProfilePickerOverlay::Host ph;
  ph.run_file_dialog = run_file_dialog_;
  ph.get_profiles = [this]() -> std::vector<ProfileRecord> {
    return shell_model_.space_manager_.profile_store().List();
  };
  ph.create_space = [this](const std::string& path,
                           const std::string& profile_id) -> std::string {
    const auto new_id = shell_model_.space_manager_.CreateSpace(
        std::filesystem::path(path), profile_id);
    if (new_id.empty())
      return {};
    for (const auto& sp : shell_model_.space_manager_.spaces()) {
      if (sp->id == new_id) {
        return nlohmann::json{
            {"id", sp->id},
            {"name", sp->name},
            {"profile_id", sp->profile_id},
            {"workspace_root", sp->workspace_root.string()},
        }
            .dump();
      }
    }
    return nlohmann::json{{"id", new_id}}.dump();
  };
  ph.send_space_created_event = [this](const std::string& space_json) {
    BroadcastToAllPanels("space.created", space_json);
  };
  profile_picker_overlay_ = std::make_unique<ProfilePickerOverlay>(
      /*theme_ctx=*/this, main_window_, std::move(ph));
  profile_picker_overlay_->Build();
}

// ---------------------------------------------------------------------------
// Tab card mounting (Phase 9: content_panel_ is the universal card host).
// ---------------------------------------------------------------------------

std::string MainWindow::OpenWebTab(const std::string& url) {
  const std::string final_url =
      url.find("://") == std::string::npos ? "https://" + url : url;
  OpenParams params;
  params.url = final_url;
  TabId id = shell_model_.tabs_->Open(TabKind::kWeb, params);
  if (id.empty())
    return {};
  // Tab::Register() in Open() seeded theme colors automatically.
  PersistTabCreated(id, final_url, "");
  shell_model_.tabs_->Activate(
      id);  // triggers NotifyActiveTabChanged → ContentView
  return id;
}

// Phase 8: ShowActiveTab() removed —
// ContentView::OnShellEvent<ActiveTabChanged> now drives card management via
// the observer.

// ---------------------------------------------------------------------------
// Popover (Phase 7: delegates to PopoverCtrl)
// ---------------------------------------------------------------------------

void MainWindow::OpenPopover(const std::string& url, int owner_browser_id) {
  if (!main_window_ || !popover_)
    return;
  popover_->Open(url, owner_browser_id);
}

void MainWindow::ClosePopover() {
  if (popover_)
    popover_->Close();
}

// Open a panel page (settings / flows / activities) in its own movable,
// resizable top-level window. Delegates to the static PanelWindow registry
// so a repeated open of the same URL focuses the existing window instead of
// duplicating. Replaces the in-window overlay popover that previously
// shrank the content area and dimmed it with a scrim.
// `main_window_` is forwarded as the parent so the new panel window can
// center itself over the main app window instead of the screen.
void MainWindow::OpenPanelWindow(const std::string& url,
                                 const std::string& title) {
  PanelWindow::OpenOrFocus(url, title,
                           /*resource_ctx=*/this, client_handler_.get(),
                           /*theme_ctx=*/this,
                           /*parent_window=*/main_window_);
}

void MainWindow::UpdatePopoverVisibility() {
  if (!popover_)
    return;
  // Re-implement visibility check here since TabsContext doesn't expose
  // browser_id directly.
  if (!popover_->IsOpen())
    return;
  const int owner_id = popover_->owner_browser_id();
  bool visible = (owner_id == 0);
  if (!visible) {
    Tab* active = shell_model_.tabs_->Active();
    visible = (active && active->kind() == TabKind::kWeb &&
               active->browser_id() == owner_id);
  }
  popover_->SetVisible(visible);
  SetContentOuterVInsets(visible ? 24 : 0, visible ? 24 : 8);
#if defined(__APPLE__)
  if (main_window_) {
    if (visible) {
      popover_->LayoutPopover();
    } else {
      HidePopoverScrim(main_window_->GetWindowHandle());
    }
  }
#endif
}

void MainWindow::OnWindowBoundsChanged(CefRefPtr<CefWindow> window,
                                       const CefRect& new_bounds) {
  (void)window;
  (void)new_bounds;
  if (popover_ && popover_->IsOpen())
    popover_->LayoutPopover();
  if (overlay_open_)
    UpdateOverlayRect();
  RefreshTitleBarDragRegion();
  // The dock's corner mask is computed from its bounds — recompute on resize.
  if (right_dock_view_obj_)
    right_dock_view_obj_->ApplyCornerRounding();
}

// ---------------------------------------------------------------------------
// Native title bar (CefPanel) — Phase 9: delegated to TitleBarView
// ---------------------------------------------------------------------------

void MainWindow::ToggleSidebar() {
  if (!sidebar_view_obj_)
    return;
  sidebar_visible_ = !sidebar_visible_;
  sidebar_view_obj_->SetVisible(sidebar_visible_);
  // Force a layout pass so the content area expands/contracts immediately.
  if (body_panel_)
    body_panel_->Layout();
  // Re-layout the popover scrim to match the new content frame origin.
  if (popover_ && popover_->IsOpen())
    popover_->LayoutPopover();
#if defined(__APPLE__)
  // Re-raise the drag overlay and re-punch card corners; layout may have
  // repositioned the content card so old punch views are in the wrong place.
  CefPostTask(TID_UI, base::BindOnce(
                          [](CefRefPtr<MainWindow> self) {
                            self->RefreshTitleBarDragRegion();
                            if (self->content_view_)
                              self->content_view_->RefreshCornerMasks();
                          },
                          CefRefPtr<MainWindow>(this)));
#endif
}

void MainWindow::OpenNewTabKind(const std::string& kind) {
  TabKind k;
  if (kind == "web")
    k = TabKind::kWeb;
  else if (kind == "terminal")
    k = TabKind::kTerminal;
  else if (kind == "chat")
    k = TabKind::kChat;
  else
    return;

  TabId id;
  std::string url_for_event;
  if (k == TabKind::kWeb) {
    url_for_event = "https://www.google.com";
    id = OpenWebTab(url_for_event);
  } else {
    id = shell_model_.tabs_->Open(k, OpenParams{});
    // Tab::Register() in Open() seeded theme colors automatically.
    if (!id.empty())
      shell_model_.tabs_->Activate(id);
  }
  if (id.empty())
    return;

  // Mirror the existing shell.tab_created (numeric-id) shape so the
  // sidebar's BrowserTab Zod schema accepts the event.
  int numeric = 0;
  static constexpr char kPrefix[] = "tab-";
  if (id.compare(0, sizeof(kPrefix) - 1, kPrefix) == 0) {
    numeric = std::atoi(id.c_str() + sizeof(kPrefix) - 1);
  }
  std::string created = nlohmann::json{
      {"id", numeric},
      {"url", url_for_event},
      {"title", ""},
      {"is_pinned", false}}.dump();
  PushToSidebar("shell.tab_created", created);
}

void MainWindow::RefreshTitleBarDragRegion() {
  // Phase 9: delegated to TitleBarView.
  if (titlebar_view_)
    titlebar_view_->RefreshDragRegion();
}

// ---------------------------------------------------------------------------
// Sidebar push helper
// ---------------------------------------------------------------------------

void MainWindow::PushToSidebar(const std::string& event_name,
                               const std::string& json_payload) {
  if (auto bv = sidebar_view()) {
    if (auto browser = bv->GetBrowser())
      client_handler_->SendBrowserEvent(browser, event_name, json_payload);
  }
}

void MainWindow::BroadcastToAllPanels(const std::string& event_name,
                                      const std::string& json_payload) {
  // BroadcastToAllPanels may be called from the RuntimeBridge pump thread.
  // CefBrowserView::GetBrowser() and TabManager are only safe on TID_UI, so
  // if we are not already on the UI thread, re-schedule the call there.
  if (!CefCurrentlyOn(TID_UI)) {
    CefPostTask(
        TID_UI,
        base::BindOnce(
            [](CefRefPtr<MainWindow> self, std::string ev, std::string body) {
              self->BroadcastToAllPanels(ev, body);
            },
            CefRefPtr<MainWindow>(this), event_name, json_payload));
    return;
  }

  if (auto bv = sidebar_view()) {
    if (auto browser = bv->GetBrowser())
      client_handler_->SendBrowserEvent(browser, event_name, json_payload);
  }
  // Also push to the in-window overlay popover (transient web URL popover)
  // when one is open.
  if (popover_ && popover_->IsOpen()) {
    if (auto bv = popover_->content_view()) {
      if (auto browser = bv->GetBrowser())
        client_handler_->SendBrowserEvent(browser, event_name, json_payload);
    }
  }
  // Push to every open PanelWindow (settings, flows, activities). These
  // are independent top-level windows and need every broadcast event
  // (theme.changed, space.switch_loading, etc.) to stay in sync with the
  // main window.
  for (const auto& bv : PanelWindow::AllBrowserViews()) {
    if (!bv)
      continue;
    if (auto browser = bv->GetBrowser())
      client_handler_->SendBrowserEvent(browser, event_name, json_payload);
  }
  // Phase 9: per-kind *_view_ singletons are gone. Broadcast to every
  // tab's content browser via the TabManager.
  if (!shell_model_.tabs_)
    return;
  const auto snap = shell_model_.tabs_->Snapshot();
  // fprintf(stderr, "[BroadcastToAllPanels] ev=%s tabs=%zu\n",
  // event_name.c_str(),
  //         snap.size());
  // fflush(stderr);
  for (const auto& s : snap) {
    Tab* t = shell_model_.tabs_->Get(s.id);
    if (!t || !t->behavior()) {
      fprintf(stderr, "[BroadcastToAllPanels] tab=%s behavior=NULL skip\n",
              s.id.c_str());
      fflush(stderr);
      continue;
    }
    // Find the corresponding CefBrowserView through whichever behavior
    // exposes one. Both WebTabBehavior and SimpleTabBehavior expose
    // browser_view(). We are on TID_UI here so GetBrowser() is safe.
    CefRefPtr<CefBrowserView> bv;
    if (t->kind() == TabKind::kWeb) {
      if (auto* wb = static_cast<WebTabBehavior*>(t->behavior())) {
        bv = wb->browser_view();
      }
    } else {
      if (auto* sb = static_cast<SimpleTabBehavior*>(t->behavior())) {
        bv = sb->browser_view();
      }
    }
    // fprintf(stderr, "[BroadcastToAllPanels] tab=%s kind=%s bv=%s\n",
    //         s.id.c_str(), TabKindToString(s.kind), bv ? "ok" : "NULL");
    // fflush(stderr);
    if (bv) {
      if (auto browser = bv->GetBrowser())
        client_handler_->SendBrowserEvent(browser, event_name, json_payload);
    }
  }
}

void MainWindow::PushActiveViewToRail() {
  if (!CefCurrentlyOn(TID_UI)) {
    CefPostTask(TID_UI, base::BindOnce(
                            [](CefRefPtr<MainWindow> self) {
                              self->PushActiveViewToRail();
                            },
                            CefRefPtr<MainWindow>(this)));
    return;
  }
  // Main content area: which rail-owned view is active (if any).
  std::string main_view;
  if (Tab* active = shell_model_.tabs_ ? shell_model_.tabs_->Active() : nullptr) {
    switch (active->kind()) {
      case TabKind::kActivity:
        main_view = "activity";
        break;
      case TabKind::kFlows:
        main_view = "flows";
        break;
      case TabKind::kExtensionView:
        main_view = active->GetMeta("ext_view");
        break;
      default:
        break;
    }
  }
  // Right dock: the open view key, if any.
  std::string dock_view =
      right_dock_view_obj_ ? right_dock_view_obj_->active_view_key()
                           : std::string();

  const std::string payload =
      nlohmann::json{{"main", main_view}, {"dock", dock_view}}.dump();
  if (activitybar_view_obj_) {
    if (auto bv = activitybar_view_obj_->browser_view()) {
      if (auto browser = bv->GetBrowser())
        client_handler_->SendBrowserEvent(browser, "shell.active_view_changed",
                                          payload);
    }
  }
}

void MainWindow::CloseExtensionViews(const std::string& ext_id) {
  if (!CefCurrentlyOn(TID_UI)) {
    CefPostTask(TID_UI, base::BindOnce(
                            [](CefRefPtr<MainWindow> self, std::string id) {
                              self->CloseExtensionViews(id);
                            },
                            CefRefPtr<MainWindow>(this), ext_id));
    return;
  }
  if (!shell_model_.tabs_)
    return;
  const std::string prefix = ext_id + "::";

  // Collapse the dock if it is showing one of this extension's views.
  if (right_dock_view_obj_) {
    const std::string dk = right_dock_view_obj_->active_view_key();
    if (dk.size() >= prefix.size() && dk.compare(0, prefix.size(), prefix) == 0) {
      right_dock_view_obj_->Hide();
      if (body_panel_)
        body_panel_->Layout();
    }
  }

  // Close every open view tab owned by this extension.
  for (const TabId& id :
       shell_model_.tabs_->FindAllByMetaPrefix("ext_view", prefix)) {
    if (content_view_)
      content_view_->RemoveCard(id);
    shell_model_.tabs_->Close(id);
  }

  // Activate the most recent chat tab (or any remaining tab) so the user
  // isn't left on a blank content area.
  TabId target;
  for (const auto& s : shell_model_.tabs_->Snapshot()) {
    if (s.kind == TabKind::kChat)
      target = s.id;  // Snapshot is creation-ordered; keep the last chat.
  }
  if (target.empty()) {
    const auto snap = shell_model_.tabs_->Snapshot();
    if (!snap.empty())
      target = snap.back().id;
  }
  if (!target.empty())
    shell_model_.tabs_->Activate(target);

  PushActiveViewToRail();
}

// ---------------------------------------------------------------------------
// Button + keyboard handlers
// ---------------------------------------------------------------------------

void MainWindow::OnButtonPressed(CefRefPtr<CefButton> button) {
  // No native top-bar buttons remain; the HTML topbar drives navigation via
  // shell.* bridge channels. Kept as a no-op so the Delegate stays valid.
  (void)button;
}

bool MainWindow::OnKeyEvent(CefRefPtr<CefTextfield> textfield,
                            const CefKeyEvent& event) {
  (void)textfield;
  (void)event;
  return false;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 4.5: Per-Space tab persistence (web tabs only). Title sync runs on every
// TabManager mutation; in practice that fires when the URL field updates
// after a navigation, which is when WebTabBehavior::OnTitleChange has
// usually already updated current_title().
// ---------------------------------------------------------------------------

void MainWindow::PersistTabCreated(const std::string& tab_id,
                                   const std::string& url,
                                   const std::string& title) {
  Space* sp = shell_model_.space_manager_.ActiveSpace();
  if (!sp)
    return;
  BrowserTabRow row;
  row.space_id = sp->id;
  row.url = url;
  row.title = title;
  row.is_pinned = false;
  row.last_accessed = 0;
  const int64_t db_id = shell_model_.space_manager_.store().CreateTab(row);
  if (db_id > 0) {
    tab_db_ids_[tab_id] = db_id;
    tab_persisted_titles_[tab_id] = title;
  }
}

void MainWindow::PersistTabTitlesIfChanged() {
  if (!shell_model_.tabs_)
    return;
  for (const auto& s : shell_model_.tabs_->Snapshot()) {
    if (s.kind != TabKind::kWeb)
      continue;
    auto it = tab_db_ids_.find(s.id);
    if (it == tab_db_ids_.end())
      continue;
    Tab* t = shell_model_.tabs_->Get(s.id);
    if (!t)
      continue;
    auto* wb = static_cast<WebTabBehavior*>(t->behavior());
    if (!wb)
      continue;
    const std::string& title = wb->current_title();
    const std::string& url = wb->current_url();
    auto last = tab_persisted_titles_.find(s.id);
    if (last != tab_persisted_titles_.end() && last->second == title) {
      continue;
    }
    BrowserTabRow row;
    row.id = it->second;
    row.url = url;
    row.title = title;
    row.is_pinned = false;
    row.last_accessed = 0;
    shell_model_.space_manager_.store().UpdateTab(row);
    tab_persisted_titles_[s.id] = title;
  }
}

void MainWindow::PersistTabClosed(const std::string& tab_id) {
  auto it = tab_db_ids_.find(tab_id);
  if (it == tab_db_ids_.end())
    return;
  shell_model_.space_manager_.store().DeleteTab(it->second);
  tab_db_ids_.erase(it);
  tab_persisted_titles_.erase(tab_id);
}

// ---------------------------------------------------------------------------
// Sidebar tab persistence (chat + terminal tabs survive app restarts)
// ---------------------------------------------------------------------------

void MainWindow::PersistSidebarTabs() {
  if (!shell_model_.tabs_)
    return;
  nlohmann::json obj = nlohmann::json::object();
  nlohmann::json arr = nlohmann::json::array();
  for (const auto& s : shell_model_.tabs_->Snapshot()) {
    if (s.kind != TabKind::kChat && s.kind != TabKind::kTerminal)
      continue;
    nlohmann::json entry;
    entry["id"] = s.id;
    entry["kind"] = TabKindToString(s.kind);
    entry["displayName"] = s.display_name;
    nlohmann::json meta_obj = nlohmann::json::object();
    for (const auto& [k, v] : s.meta)
      meta_obj[k] = v;
    entry["meta"] = meta_obj;
    arr.push_back(std::move(entry));
  }
  obj["tabs"] = std::move(arr);
  obj["activeTabId"] = shell_model_.tabs_->active_tab_id();
  shell_model_.space_manager_.store().SetKv("ui.sidebar_tabs", obj.dump());
}

bool MainWindow::RestoreSidebarTabs() {
  const std::string raw =
      shell_model_.space_manager_.store().GetKv("ui.sidebar_tabs");
  if (raw.empty())
    return false;
  nlohmann::json obj;
  obj = nlohmann::json::parse(raw, nullptr, /*allow_exceptions=*/false);
  if (obj.is_discarded())
    return false;
  const auto& arr = obj.value("tabs", nlohmann::json::array());
  if (!arr.is_array() || arr.empty())
    return false;

  std::string active_id = obj.value("activeTabId", std::string{});
  std::string first_id;

  for (const auto& entry : arr) {
    const std::string kind_s = entry.value("kind", std::string{});
    TabKind kind;
    bool kind_from_string_ok = false;
    if (kind_s == "chat") {
      kind = TabKind::kChat;
      kind_from_string_ok = true;
    } else if (kind_s == "terminal") {
      kind = TabKind::kTerminal;
      kind_from_string_ok = true;
    }
    if (!kind_from_string_ok)
      continue;

    OpenParams params;
    params.display_name = entry.value("displayName", std::string{});
    const auto& meta_obj = entry.value("meta", nlohmann::json::object());
    if (meta_obj.is_object()) {
      for (const auto& [k, v] : meta_obj.items()) {
        if (v.is_string())
          params.meta[k] = v.get<std::string>();
      }
    }

    const TabId id = shell_model_.tabs_->Open(kind, params);
    if (id.empty())
      continue;
    // Tab::Register() in Open() already seeded theme colors.
    if (first_id.empty())
      first_id = id;
    // Note: restored tabs get new IDs (TabManager::NewId), so we can't
    // map the stored activeTabId directly. We activate by position index.
  }

  // Activate: try to match by stored index (first chat tab if stored active
  // was a chat, first terminal if terminal). For simplicity just activate
  // the first restored tab.
  const std::string activate_id = first_id;
  if (!activate_id.empty())
    shell_model_.tabs_->Activate(activate_id);
  return !first_id.empty();
}

// ---------------------------------------------------------------------------
// Web tab session restore
// ---------------------------------------------------------------------------

void MainWindow::RestoreWebTabs() {
  if (!shell_model_.tabs_)
    return;
  Space* sp = shell_model_.space_manager_.ActiveSpace();
  if (!sp)
    return;
  const auto rows =
      shell_model_.space_manager_.store().ListTabsForSpace(sp->id);
  for (const auto& row : rows) {
    if (row.url.empty())
      continue;
    OpenParams params;
    params.url = row.url;
    params.lazy_load = true;
    params.restore_title = row.title;
    const TabId id = shell_model_.tabs_->Open(TabKind::kWeb, params);
    if (id.empty())
      continue;
    // Seed the DB mapping so PersistTabClosed() and PersistTabTitlesIfChanged()
    // operate on the pre-existing row instead of creating a duplicate.
    tab_db_ids_[id] = row.id;
    tab_persisted_titles_[id] = row.title;
  }
}

// ---------------------------------------------------------------------------
// refine-ui-theme-layout: theme application + persistence
// ---------------------------------------------------------------------------

// static

void MainWindow::ApplyThemeChrome(const ThemeChrome& chrome) {
  shell_model_.current_chrome_ = chrome;
  // Native panels: titlebar + body share the chrome fill so the visual
  // chrome is one continuous region (per refine-ui-theme-layout design).
  if (body_panel_)
    body_panel_->SetBackgroundColor(chrome.bg_body);
#if defined(__APPLE__)
  // Phase 9: macOS window background + appearance set by
  // TitleBarView::ApplyTheme. Phase 8: corner-punch views handled by
  // ContentView::ApplyTheme. No-op block retained for documentation only.
#endif
  // Broadcast to renderers so each panel's installThemeMirror flips the
  // <html data-theme="…"> attribute and React listeners refresh.
  BroadcastToAllPanels("theme.changed",
                       shell_model_.ThemeStateJson(/*include_chrome=*/true));
  // native-views-mvc Phase 4.5: notify subscribed view observers.
  // ThemeAwareView subscribers (titlebar, sidebar, content, popover, tabs)
  // receive ApplyTheme() via OnEvent() — no direct calls needed.
  shell_model_.NotifyThemeChanged(chrome);
}

void MainWindow::HandleThemeModeChange(const std::string& mode) {
  if (mode != "system" && mode != "light" && mode != "dark")
    return;
  shell_model_.theme_mode_ = mode;
  shell_model_.space_manager_.store().SetKv("ui.theme", mode);
  ApplyThemeChrome(ViewModel::ChromeFor(shell_model_.ResolveAppearance()));
}

void MainWindow::OnSystemAppearanceChanged() {
  // Only react in `system` mode; explicit Light/Dark pins ignore the OS.
  if (shell_model_.theme_mode_ != "system")
    return;
  ApplyThemeChrome(ViewModel::ChromeFor(shell_model_.ResolveAppearance()));
}

void MainWindow::SetContentOuterVInsets(int top, int bottom) {
  if (content_view_)
    content_view_->SetVInsets(top, bottom);
}

// ---------------------------------------------------------------------------
// native-views-mvc Phase 3: context interface implementations
// ---------------------------------------------------------------------------

// ThemeContext ----------------------------------------------------------
ThemeChrome MainWindow::GetCurrentChrome() const {
  return shell_model_.current_chrome_;
}

void MainWindow::AddThemeObserver(ViewObserver<ThemeChanged>* obs) {
  shell_model_.theme_observers.AddObserver(obs);
}

void MainWindow::RemoveThemeObserver(ViewObserver<ThemeChanged>* obs) {
  shell_model_.theme_observers.RemoveObserver(obs);
}

// SpaceContext ---------------------------------------------------------
std::string MainWindow::GetCurrentSpaceId() const {
  const Space* sp = shell_model_.space_manager_.ActiveSpace();
  return sp ? sp->id : std::string{};
}

std::string MainWindow::GetCurrentSpaceName() const {
  const Space* sp = shell_model_.space_manager_.ActiveSpace();
  return sp ? sp->name : std::string{};
}

void MainWindow::SwitchSpace(const std::string& space_id) {
  shell_model_.space_manager_.SwitchTo(space_id);
}

std::vector<std::pair<std::string, std::string>> MainWindow::GetSpaces() const {
  std::vector<std::pair<std::string, std::string>> result;
  for (const auto& sp : shell_model_.space_manager_.spaces())
    result.emplace_back(sp->id, sp->name);
  return result;
}

void MainWindow::AddSpaceObserver(ViewObserver<SpaceChanged>* obs) {
  shell_model_.space_observers.AddObserver(obs);
}

void MainWindow::RemoveSpaceObserver(ViewObserver<SpaceChanged>* obs) {
  shell_model_.space_observers.RemoveObserver(obs);
}

// TabsContext ----------------------------------------------------------
std::string MainWindow::GetActiveTabUrl() const {
  // No direct URL accessor on TabManager yet — return empty for now.
  return std::string{};
}

void MainWindow::AddTabsObserver(ViewObserver<TabsChanged>* obs) {
  shell_model_.tabs_observers.AddObserver(obs);
}

void MainWindow::RemoveTabsObserver(ViewObserver<TabsChanged>* obs) {
  shell_model_.tabs_observers.RemoveObserver(obs);
}

void MainWindow::AddActiveTabObserver(ViewObserver<ActiveTabChanged>* obs) {
  shell_model_.active_tab_observers.AddObserver(obs);
}

void MainWindow::RemoveActiveTabObserver(ViewObserver<ActiveTabChanged>* obs) {
  shell_model_.active_tab_observers.RemoveObserver(obs);
}

// WindowActionContext --------------------------------------------------
void MainWindow::SetTitleBarDragRegion(const CefRect& /*rect*/) {
  // Phase 3 stub: delegate to the existing full-recompute helper.
  // Phase 9 (TitleBarView) will pass the pre-computed rect directly.
  RefreshTitleBarDragRegion();
}

// OverlayActionContext -------------------------------------------------
void MainWindow::OpenOverlay(const std::string& url) {
  if (!overlay_bv_ || !overlay_oc_ || !main_window_)
    return;
  // Store pending URL so on_browser_created can load it if GetBrowser() is
  // still null (async browser creation on the first open).
  overlay_pending_url_ = url;
  if (auto b = overlay_bv_->GetBrowser()) {
    b->GetMainFrame()->LoadURL(url);
    overlay_pending_url_.clear();
  }
  overlay_open_ = true;
  UpdateOverlayRect();
#if defined(__APPLE__)
  if (!overlay_click_monitor_) {
    const CefRect wb = main_window_->GetBounds();
    const int oh = static_cast<int>(wb.height * 0.90);
    const int oy = (wb.height - oh) / 2;
    // Exclude rect covers the FULL window width from y=0 (top of frame,
    // including title bar) to the bottom of the overlay. This prevents
    // title-bar button clicks (e.g. Settings button, y ≈ 0-38) from
    // triggering a spurious overlay-close when the overlay is open.
    // Only clicks below the overlay (the thin strip at the bottom) will
    // trigger dismiss.
    const CefRect monitor_rect{0, 0, wb.width, oy + oh};
    overlay_click_monitor_ = InstallClickOutsideMonitor(
        reinterpret_cast<void*>(main_window_->GetWindowHandle()), monitor_rect,
        [](void* user) {
          auto* self = static_cast<MainWindow*>(user);
          CefPostTask(TID_UI, base::BindOnce(&MainWindow::CloseOverlay,
                                             CefRefPtr<MainWindow>(self)));
        },
        this);
  }
  // Raise the overlay NSWindow above any open popover child windows.
  CefPostTask(
      TID_UI,
      base::BindOnce(
          [](CefRefPtr<MainWindow> self) {
            if (self->overlay_bv_ && self->overlay_bv_->GetBrowser()) {
              void* h = reinterpret_cast<void*>(self->overlay_bv_->GetBrowser()
                                                    ->GetHost()
                                                    ->GetWindowHandle());
              RaiseOverlayWindow(h);
            }
          },
          CefRefPtr<MainWindow>(this)));
#endif
}

void MainWindow::CloseOverlay() {
  if (overlay_oc_) {
    // Do NOT zero bounds before hiding — mirrors Popover::Close() and
    // ProfilePickerOverlay::Hide() which only call SetVisible(false).
    // Zeroing bounds while visible causes the overlay to be positioned at
    // (0,0) with size 0; CEF on macOS may then ignore the subsequent
    // SetBounds(valid_rect) call made while the overlay is invisible,
    // resulting in a zero-size (invisible) overlay on the next open.
    overlay_oc_->SetVisible(false);
  }
  overlay_open_ = false;
#if defined(__APPLE__)
  if (overlay_click_monitor_) {
    RemoveClickOutsideMonitor(overlay_click_monitor_);
    overlay_click_monitor_ = nullptr;
  }
#endif
}

void MainWindow::UpdateOverlayRect() {
  if (!overlay_oc_ || !overlay_open_ || !main_window_)
    return;
  const CefRect wb = main_window_->GetBounds();
  const int ow = static_cast<int>(wb.width * 0.90);
  const int oh = static_cast<int>(wb.height * 0.90);
  const CefRect rect{(wb.width - ow) / 2, (wb.height - oh) / 2, ow, oh};
  overlay_oc_->SetBounds(rect);
  overlay_oc_->SetVisible(true);
}

void MainWindow::ShowFloat(const std::string& url) {
  if (!float_bv_ || !float_oc_ || !main_window_)
    return;
  DismissFloat();
  if (auto b = float_bv_->GetBrowser())
    b->GetMainFrame()->LoadURL(url);
  const CefRect wb = main_window_->GetBounds();
  constexpr int kFloatW = 400;
  constexpr int kFloatH = 300;
  constexpr int kMargin = 20;
  const int fw = std::min(kFloatW, wb.width - 2 * kMargin);
  const int fh = std::min(kFloatH, wb.height - 2 * kMargin);
  const CefRect rect{(wb.width - fw) / 2, (wb.height - fh) / 2, fw, fh};
  float_oc_->SetBounds(rect);
  float_oc_->SetVisible(true);
#if defined(__APPLE__)
  float_monitor_ = InstallClickOutsideMonitor(
      reinterpret_cast<void*>(main_window_->GetWindowHandle()), rect,
      [](void* user) {
        auto* self = static_cast<MainWindow*>(user);
        CefPostTask(TID_UI, base::BindOnce(&MainWindow::DismissFloat,
                                           CefRefPtr<MainWindow>(self)));
      },
      this);
#endif
}

void MainWindow::DismissFloat() {
  if (float_oc_) {
    float_oc_->SetBounds({0, 0, 0, 0});
    float_oc_->SetVisible(false);
  }
#if defined(__APPLE__)
  if (float_monitor_) {
    RemoveClickOutsideMonitor(float_monitor_);
    float_monitor_ = nullptr;
  }
#endif
}

// ResourceContext ------------------------------------------------------
// (ResourceUrl implementation already exists above — override resolved
//  automatically since the signature matches the interface.)

}  // namespace cronymax
