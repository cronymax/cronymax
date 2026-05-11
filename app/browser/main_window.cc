#include "browser/main_window.h"
#include "browser/models/view_model.h"

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
#include "include/cef_parser.h"
#include "runtime/legacy_importer.h"
#include "include/cef_path_util.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/views/cef_fill_layout.h"
#include "include/views/cef_menu_button.h"
#include "include/views/cef_panel_delegate.h"
#include "include/cef_menu_model.h"
#include "include/cef_menu_model_delegate.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

#include "browser/platform/view_style.h"

#if defined(__APPLE__)
#include "browser/icon_registry.h"
#include "browser/mac_folder_picker.h"
#include "browser/tab/tab.h"
#include "browser/tab/tab_behavior.h"
#include "browser/tab/web_tab_behavior.h"
#include "browser/tab/simple_tab_behavior.h"
#endif

namespace cronymax {
namespace {

class SizedPanelDelegate : public CefPanelDelegate {
 public:
  explicit SizedPanelDelegate(CefSize preferred_size)
      : preferred_size_(preferred_size) {}
  CefSize GetPreferredSize(CefRefPtr<CefView> view) override {
    (void)view; return preferred_size_;
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
[[maybe_unused]] void EnsureButtonReferenced() { (void)&Button; }

std::string EncodeFilePathForUrl(const std::string& path) {
  static constexpr char kHex[] = "0123456789ABCDEF";
  std::string out;
  out.reserve(path.size() + 16);
  for (unsigned char ch : path) {
    char c = static_cast<char>(ch);
    if (std::isalnum(ch) || c == '-' || c == '_' || c == '.' || c == '~' ||
        c == '/' || c == ':') {
      out.push_back(c);
      continue;
    }
    out.push_back('%');
    out.push_back(kHex[(ch >> 4) & 0x0F]);
    out.push_back(kHex[ch & 0x0F]);
  }
  return out;
}

std::string FileUrlFromPath(const std::filesystem::path& path) {
  auto normalized = path.lexically_normal().string();
  return "file://" + EncodeFilePathForUrl(normalized);
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
    if (on_click_) on_click_();
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
  using PressFn = std::function<void(
      CefRefPtr<CefMenuButton>,
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

// std::function-backed CefMenuModelDelegate for space-selector menu results.
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

// std::function-backed CefTextfieldDelegate for the popover URL textfield.
class FnTextfieldDelegate : public CefTextfieldDelegate {
 public:
  using KeyFn = std::function<bool(CefRefPtr<CefTextfield>, const CefKeyEvent&)>;
  explicit FnTextfieldDelegate(KeyFn fn) : fn_(std::move(fn)) {}
  bool OnKeyEvent(CefRefPtr<CefTextfield> tf,
                  const CefKeyEvent& ev) override {
    return fn_ ? fn_(tf, ev) : false;
  }
 private:
  KeyFn fn_;
  IMPLEMENT_REFCOUNTING(FnTextfieldDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnTextfieldDelegate);
};

// Forward declaration — defined later in this TU inside the helpers section.
void PushToView(CefRefPtr<CefBrowserView> view,
                const std::string& event_name,
                const std::string& json_payload);

}  // namespace

// ---------------------------------------------------------------------------
// MainWindow
// ---------------------------------------------------------------------------

/*static*/ void MainWindow::Create() {
  CefWindow::CreateTopLevelWindow(new MainWindow());
}

MainWindow::MainWindow() : client_handler_(new ClientHandler(&shell_model_.space_manager_)) {}

void MainWindow::OnWindowCreated(CefRefPtr<CefWindow> window) {
  CEF_REQUIRE_UI_THREAD();
  window->SetTitle("cronymax");
  main_window_ = window;

  // Resources path — needed for builtin-doc-types regardless of where DB lives.
  CefString res_path;
  CefGetPath(PK_DIR_RESOURCES, res_path);

  // Store the database in the user-data directory so it survives app-bundle
  // rebuilds (the Chromium Framework Resources dir is wiped by COPY_MAC_FRAMEWORK
  // on every cmake build). Fall back to Resources if user-data is unavailable.
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
    shell_model_.space_manager_.CreateSpace("Default", std::filesystem::current_path(), "default");

  // arc-style-tab-cards: TabManager owns every tab; per-kind *_view_
  // singletons are gone. All non-web kinds are singleton tabs whose
  // content browser loads the existing renderer HTML.
  shell_model_.tabs_ = std::make_unique<TabManager>(this);
  shell_model_.tabs_->SetClientHandler(client_handler_.get());
  // native-title-bar: terminal/chat are multi-instance now (each click of
  // "+ Terminal" / "+ Chat" creates a fresh tab). Agent/graph stay
  // singletons.
  shell_model_.tabs_->RegisterSingletonKind(TabKind::kSettings);
  shell_model_.tabs_->SetKindContentUrl(TabKind::kChat,
                           ResourceUrl("panels/chat/index.html"));
  shell_model_.tabs_->SetKindContentUrl(TabKind::kTerminal,
                           ResourceUrl("panels/terminal/index.html"));
  shell_model_.tabs_->SetKindContentUrl(TabKind::kSettings,
                           ResourceUrl("panels/settings/index.html"));

  // refine-ui-theme-layout: load persisted theme mode (defaults to
  // "system") and seed shell_model_.current_chrome_ before BuildChrome so the title
  // bar paints with the correct color on first frame.
  {
    std::string persisted = shell_model_.space_manager_.store().GetKv("ui.theme");
    if (persisted == "light" || persisted == "dark" || persisted == "system") {
      shell_model_.theme_mode_ = persisted;
    }
    shell_model_.current_chrome_ = ViewModel::ChromeFor(shell_model_.ResolveAppearance());
  }

  BuildChrome(window);

  // Restore persisted sidebar tabs (chat/terminal) from the previous session.
  // Falls back to opening a default Chat tab on first launch.
  if (!RestoreSidebarTabs()) {
    TabId id = shell_model_.tabs_->Open(TabKind::kChat, OpenParams{});
    // Tab::Register() in Open() already seeded colors — no ApplyTheme needed.
    if (!id.empty()) shell_model_.tabs_->Activate(id);
  }

#if defined(__APPLE__)
  // Arc-style: translucent NSWindow with hidden title bar. Posted onto the
  // UI runner so the NSWindow is fully realized first.
  CefPostTask(TID_UI, base::BindOnce([](CefRefPtr<CefWindow> w,
                                         cef_color_t bg) {
                StyleMainWindowTranslucent(w->GetWindowHandle(), bg);
              }, window, shell_model_.current_chrome_.bg_body));
  // refine-ui-theme-layout: install rounded 12 px frame + initial border
  // colour around the content panel. Posted so the NSView is realized.
  CefPostTask(TID_UI, base::BindOnce([](CefRefPtr<MainWindow> self) {
                self->ApplyThemeChrome(self->shell_model_.current_chrome_);
              }, CefRefPtr<MainWindow>(this)));
  // refine-ui-theme-layout: subscribe to the macOS appearance flip
  // notification so `system` mode tracks Light/Dark in real time.
  appearance_observer_ = AddSystemAppearanceObserver(
      [](void* user) {
        auto* self = reinterpret_cast<MainWindow*>(user);
        CefPostTask(TID_UI, base::BindOnce(
            [](CefRefPtr<MainWindow> s) { s->OnSystemAppearanceChanged(); },
            CefRefPtr<MainWindow>(self)));
      },
      this);
  // native-title-bar: install the AppKit drag overlay above the title-bar
  // spacer once the initial layout has run and the spacer has real bounds.
  CefPostTask(TID_UI, base::BindOnce(
      [](CefRefPtr<MainWindow> self) { self->RefreshTitleBarDragRegion(); },
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
            PushToSidebar("shell.space_changed",
                          nlohmann::json{{"id", new_id}, {"name", sp->name}}.dump());
            // Phase 9: space button label updated via TitleBarView observer
            // (SpaceChanged event fires in shell_model_.NotifySpaceChanged).
            break;
          }
        }
      });

  // Wire runtime restart callback: on every space switch, restart the
  // Rust runtime with the new space's sandbox policy (design decision D4).
  shell_model_.space_manager_.SetRuntimeRestartCallback(
      [this](const std::string& workspace_root,
             const ProfileRecord& profile) {
        // Build the sandbox JSON that SpawnAndHandshake will inject.
        nlohmann::json sandbox;
        sandbox["workspace_root"] = workspace_root;
        sandbox["allow_network"]  = profile.allow_network;
        {
          auto arr = nlohmann::json::array();
          for (const auto& p : profile.extra_read_paths) arr.push_back(p);
          sandbox["extra_read_paths"] = arr;
        }
        {
          auto arr = nlohmann::json::array();
          for (const auto& p : profile.extra_write_paths) arr.push_back(p);
          sandbox["extra_write_paths"] = arr;
        }
        {
          auto arr = nlohmann::json::array();
          for (const auto& p : profile.extra_deny_paths) arr.push_back(p);
          sandbox["extra_deny_paths"] = arr;
        }
        runtime_bridge_->SetSandboxConfig(sandbox);

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
  (void)window; return true;
}

CefSize MainWindow::GetPreferredSize(CefRefPtr<CefView> view) {
  (void)view; return CefSize(1440, 920);
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
    tb_host.run_file_dialog = [this](std::function<void(const std::string&)> cb) {
      if (run_file_dialog_) run_file_dialog_(std::move(cb));
    };
    titlebar_view_ = std::make_unique<TitleBarView>(
        /*space=*/this, /*window_ctx=*/this, /*overlay=*/this,
        /*resources=*/this, /*theme_ctx=*/this, main_window_, std::move(tb_host));
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

  // ── Sidebar (Phase 10: owned by SidebarView) ─────────────────────────────
  sidebar_view_obj_ = std::make_unique<SidebarView>(/*resource_ctx=*/this,
                                                     /*theme_ctx=*/this,
                                                     client_handler_);
  auto sv = sidebar_view_obj_->Build();
  body_panel_->AddChildView(sv);
  body_layout->SetFlexForView(sv, 0);

  // ── Content host (Phase 8: owned by ContentView) ─────────────────────────
  {
    ContentView::Host cv_host;
    cv_host.active_tab = [this]()
        -> std::tuple<std::string, CefRefPtr<CefView>, CefRefPtr<CefBrowserView>> {
      Tab* active = shell_model_.tabs_ ? shell_model_.tabs_->Active() : nullptr;
      if (!active || !active->card()) return {};
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
          [](CefRefPtr<MainWindow> self) { self->RefreshTitleBarDragRegion(); },
          CefRefPtr<MainWindow>(this)));
    };
    cv_host.request_focus = [this]() {
      Tab* active = shell_model_.tabs_ ? shell_model_.tabs_->Active() : nullptr;
      if (!active || active->kind() != TabKind::kWeb) return;
      if (auto* wb = static_cast<WebTabBehavior*>(active->behavior()))
        if (auto bv = wb->browser_view()) bv->RequestFocus();
    };
    cv_host.update_popover_visibility = [this]() { UpdatePopoverVisibility(); };
    content_view_ = std::make_unique<ContentView>(
        /*tabs=*/this, /*theme_ctx=*/this, main_window_, std::move(cv_host));
    auto content_outer = content_view_->Build();
    body_panel_->AddChildView(content_outer);
    body_layout->SetFlexForView(content_outer, 1);
  }

  // ── native-views-mvc Phase 5: ShellDispatcher ───────────────────────────
  // Set up the DispatcherHost callbacks that provide access to MainWindow's
  // private members without exposing MainWindow* to ShellDispatcher.
  DispatcherHost disp_host;
  disp_host.push_to_sidebar = [this](const std::string& ev,
                                     const std::string& json) {
    PushToSidebar(ev, json);
  };
  disp_host.broadcast = [this](const std::string& ev,
                                const std::string& json) {
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
    if (content_view_) content_view_->RemoveCard(tab_id);
  };
  disp_host.get_popover_owner_browser_id = [this]() {
    return popover_ ? popover_->owner_browser_id() : 0;
  };
  disp_host.popover_reload = [this]() {
    if (popover_) {
      if (auto bv = popover_->content_view())
        if (auto b = bv->GetBrowser()) b->Reload();
    }
  };
  disp_host.get_popover_url = [this]() -> std::string {
    if (!popover_) return {};
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
    if (main_window_) PerformWindowDrag(main_window_->GetWindowHandle());
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

  dispatcher_ = std::make_unique<ViewDispatcher>(
      /*tabs_ctx=*/this, /*space_ctx=*/this,
      /*overlay_ctx=*/this, /*resource_ctx=*/this,
      client_handler_.get(), &shell_model_, std::move(disp_host));
  dispatcher_->Wire();


  // (task 4.2) Initialize the Rust runtime bridge and proxy. Start() finds
  // the cronymax-runtime binary in the app bundle, spawns it, and completes
  // the Hello/Welcome handshake. This is a best-effort async startup: if the
  // binary is missing or the handshake fails the app continues without the
  // runtime (degraded mode — all runtime-backed channels return 503).
  runtime_bridge_ = std::make_unique<RuntimeBridge>();
  runtime_proxy_  = std::make_unique<RuntimeProxy>();
  // (task 5.1 / 5.2) Run the one-shot legacy state importer before starting
  // the runtime so it sees the imported runs on its first load.
  // app_data_dir is declared here (outside the importer block) so the
  // runtime bridge start thread can capture it by value.
  std::filesystem::path app_data_dir;
  {
    CefString _ud;
    if (CefGetPath(PK_USER_DATA, _ud) && !_ud.empty()) {
      app_data_dir = std::filesystem::path(_ud.ToString()) / "runtime";
    } else {
      CefString _res;
      CefGetPath(PK_DIR_RESOURCES, _res);
      app_data_dir = (_res.empty() ? std::filesystem::current_path()
                                   : std::filesystem::path(_res.ToString())) /
                     "runtime";
    }
  }
  {
    LegacyImporter importer(app_data_dir);
    if (!importer.AlreadyDone()) {
      std::vector<ImportSpaceInfo> space_infos;
      for (const auto& sp_ptr : shell_model_.space_manager_.spaces()) {
        ImportSpaceInfo info;
        info.space_id      = sp_ptr->id;
        info.space_name    = sp_ptr->name;
        info.workspace_root = sp_ptr->workspace_root;
        space_infos.push_back(std::move(info));
      }
      const auto res = importer.Run(space_infos);
      LOG(INFO) << "[LegacyImporter] import done: "
                << res.spaces_seeded << " spaces, "
                << res.runs_imported  << " runs imported, "
                << res.runs_skipped   << " already present, "
                << res.parse_errors   << " parse errors";
    }
  }
  // Start the bridge on a background thread to avoid blocking the UI.
  std::thread([this, app_data_dir]() {
    if (runtime_bridge_->Start({}, app_data_dir)) {
      runtime_proxy_->Attach(runtime_bridge_.get());
      // Wire the proxy to the bridge handler on the UI thread.
      CefPostTask(TID_UI, base::BindOnce(
          [](CefRefPtr<MainWindow> self) {
            self->client_handler_->SetRuntimeProxy(self->runtime_proxy_.get());
            // Auto-subscribe to the initial active space's events.
            if (auto* sp = self->shell_model_.space_manager_.ActiveSpace()) {
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
    Tab* t = shell_model_.tabs_ ? shell_model_.tabs_->FindByBrowserId(browser_id) : nullptr;
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
  };

  client_handler_->on_title_change =
      [this](int browser_id, const std::string& title) {
        Tab* t = shell_model_.tabs_->FindByBrowserId(browser_id);
        if (!t) return;
        PushToSidebar("shell.tab_title_changed",
                      nlohmann::json{{"id", t->tab_id()}, {"title", title}}.dump());
      };

  client_handler_->on_address_change =
      [this](int browser_id, const std::string& url) {
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
        if (!t) return;
        PushToSidebar("shell.tab_url_changed",
                      nlohmann::json{{"id", t->tab_id()}, {"url", url}}.dump());
      };

  client_handler_->on_popup_request =
      [this](int browser_id, const std::string& url) -> bool {
    OpenPopover(url, browser_id);
    return true;  // suppress native popup
  };

  // DevTools: F12 or Cmd+Option+I shows the DevTools inspector for the
  // active web tab's browser. A new detached DevTools window opens.
  client_handler_->on_devtools_requested = [this](int /*browser_id*/) {
    CefRefPtr<CefBrowser> target;
    Tab* active = shell_model_.tabs_ ? shell_model_.tabs_->Active() : nullptr;
    if (active && active->kind() == TabKind::kWeb) {
      if (auto* wb = static_cast<WebTabBehavior*>(active->behavior())) {
        if (auto bv = wb->browser_view()) target = bv->GetBrowser();
      }
    }
    if (!target) return;
    CefWindowInfo wi;
    CefBrowserSettings bs;
    target->GetHost()->ShowDevTools(wi, nullptr, bs, CefPoint());
  };

#if defined(__APPLE__)
  // Forward CSS draggable-region updates from the sidebar to the native
  // overlay. The topbar pump is gone (Phase 9); sidebar still uses
  // -webkit-app-region: drag for its top strip.
  client_handler_->on_draggable_regions_changed =
      [this](int browser_id,
             const std::vector<CefDraggableRegion>& regions) {
    auto _sv = sidebar_view();
    if (!_sv) return;
    auto b = _sv->GetBrowser();
    if (!b || b->GetIdentifier() != browser_id) return;
    std::vector<DragRegion> rs;
    rs.reserve(regions.size());
    for (const auto& r : regions) {
      rs.push_back({r.bounds.x, r.bounds.y, r.bounds.width,
                    r.bounds.height, r.draggable != 0});
    }
    ApplyDraggableRegions(b->GetHost()->GetWindowHandle(),
                          rs.empty() ? nullptr : rs.data(), rs.size());
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
  phost.open_web_tab       = [this](const std::string& url) { OpenWebTab(url); };
  phost.set_content_insets = [this](int top, int bottom) {
    SetContentOuterVInsets(top, bottom);
  };
  phost.refresh_drag_region = [this]() { RefreshTitleBarDragRegion(); };
  phost.close_notify = nullptr;
  popover_ = std::make_unique<Popover>(
      /*theme_ctx=*/this,
      content_bv, content_oc,
      chrome_panel, chrome_oc,
      main_window_, std::move(phost));

  // One-time deferred styling: CEF defers addChildWindow: by one event-loop
  // tick, so the overlay NSWindow is not yet attached synchronously.
  CefPostTask(TID_UI, base::BindOnce(
      [](CefRefPtr<CefBrowserView> bv, CefRefPtr<CefWindow> w, cef_color_t bg) {
        // Content slot: all-corners mask (will be overridden on first Show).
        StyleOverlayBrowserView(bv->GetBrowser()
                                    ? bv->GetBrowser()->GetHost()->GetWindowHandle()
                                    : nullptr,
                                12.0, kCornerAll, /*with_shadow=*/true);
        // Chrome slot: top-corners mask + background.
        void* main_nsv = reinterpret_cast<void*>(w->GetWindowHandle());
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
  if (id.empty()) return {};
  // Tab::Register() in Open() seeded theme colors automatically.
  PersistTabCreated(id, final_url, "");
  shell_model_.tabs_->Activate(id);  // triggers NotifyActiveTabChanged → ContentView
  return id;
}

// Phase 8: ShowActiveTab() removed — ContentView::OnShellEvent<ActiveTabChanged>
// now drives card management via the observer.

// ---------------------------------------------------------------------------
// Popover (Phase 7: delegates to PopoverCtrl)
// ---------------------------------------------------------------------------

void MainWindow::OpenPopover(const std::string& url, int owner_browser_id) {
  if (!main_window_ || !popover_) return;
  popover_->Open(url, owner_browser_id);
}

void MainWindow::ClosePopover() {
  if (popover_) popover_->Close();
}

void MainWindow::UpdatePopoverVisibility() {
  if (!popover_) return;
  // Re-implement visibility check here since TabsContext doesn't expose
  // browser_id directly.
  if (!popover_->IsOpen()) return;
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
  (void)window; (void)new_bounds;
  if (popover_ && popover_->IsOpen()) popover_->LayoutPopover();
  RefreshTitleBarDragRegion();
}

// ---------------------------------------------------------------------------
// Native title bar (CefPanel) — Phase 9: delegated to TitleBarView
// ---------------------------------------------------------------------------

void MainWindow::ToggleSidebar() {
  if (!sidebar_view_obj_) return;
  sidebar_visible_ = !sidebar_visible_;
  sidebar_view_obj_->SetVisible(sidebar_visible_);
  // Force a layout pass so the content area expands/contracts immediately.
  if (body_panel_) body_panel_->Layout();
#if defined(__APPLE__)
  // Re-raise the drag overlay; layout may have repositioned views.
  CefPostTask(TID_UI, base::BindOnce(
      [](CefRefPtr<MainWindow> self) { self->RefreshTitleBarDragRegion(); },
      CefRefPtr<MainWindow>(this)));
#endif
}

void MainWindow::OpenNewTabKind(const std::string& kind) {
  TabKind k;
  if      (kind == "web")      k = TabKind::kWeb;
  else if (kind == "terminal") k = TabKind::kTerminal;
  else if (kind == "chat")     k = TabKind::kChat;
  else return;

  TabId id;
  std::string url_for_event;
  if (k == TabKind::kWeb) {
    url_for_event = "https://www.google.com";
    id = OpenWebTab(url_for_event);
  } else {
    id = shell_model_.tabs_->Open(k, OpenParams{});
    // Tab::Register() in Open() seeded theme colors automatically.
    if (!id.empty()) shell_model_.tabs_->Activate(id);
  }
  if (id.empty()) return;

  // Mirror the existing shell.tab_created (numeric-id) shape so the
  // sidebar's BrowserTab Zod schema accepts the event.
  int numeric = 0;
  static constexpr char kPrefix[] = "tab-";
  if (id.compare(0, sizeof(kPrefix) - 1, kPrefix) == 0) {
    numeric = std::atoi(id.c_str() + sizeof(kPrefix) - 1);
  }
  std::string created = nlohmann::json{{"id", numeric}, {"url", url_for_event}, {"title", ""}, {"is_pinned", false}}.dump();
  PushToSidebar("shell.tab_created", created);
}

void MainWindow::RefreshTitleBarDragRegion() {
  // Phase 9: delegated to TitleBarView.
  if (titlebar_view_) titlebar_view_->RefreshDragRegion();
}

// ---------------------------------------------------------------------------
// Sidebar push helper
// ---------------------------------------------------------------------------

namespace {

void PushToView(CefRefPtr<CefBrowserView> view,
                const std::string& event_name,
                const std::string& json_payload) {
  if (!view) return;
  if (!CefCurrentlyOn(TID_UI)) {
    CefPostTask(TID_UI, base::BindOnce(&PushToView, view, event_name,
                                       json_payload));
    return;
  }
  auto browser = view->GetBrowser();
  if (!browser) {
    fprintf(stderr, "[PushToView] ev=%s GetBrowser()=NULL\n", event_name.c_str());
    fflush(stderr);
    return;
  }
  auto frame = browser->GetMainFrame();
  if (!frame) {
    fprintf(stderr, "[PushToView] ev=%s GetMainFrame()=NULL\n", event_name.c_str());
    fflush(stderr);
    return;
  }
  const std::string js =
      "window.cronymax?.browser?.onDispatch?.(" +
      ("'" + event_name + "'") + "," + json_payload + ");";
  fprintf(stderr, "[PushToView] ev=%s bid=%d ExecuteJavaScript\n",
          event_name.c_str(), browser->GetIdentifier());
  fflush(stderr);
  frame->ExecuteJavaScript(js, frame->GetURL(), 0);
}

}  // namespace

void MainWindow::PushToSidebar(const std::string& event_name,
                               const std::string& json_payload) {
  PushToView(sidebar_view(), event_name, json_payload);
}

void MainWindow::BroadcastToAllPanels(const std::string& event_name,
                                      const std::string& json_payload) {
  // BroadcastToAllPanels may be called from the RuntimeBridge pump thread.
  // CefBrowserView::GetBrowser() and TabManager are only safe on TID_UI, so
  // if we are not already on the UI thread, re-schedule the call there.
  if (!CefCurrentlyOn(TID_UI)) {
    CefPostTask(TID_UI, base::BindOnce(
        [](CefRefPtr<MainWindow> self, std::string ev, std::string body) {
          self->BroadcastToAllPanels(ev, body);
        },
        CefRefPtr<MainWindow>(this), event_name, json_payload));
    return;
  }

  PushToView(sidebar_view(), event_name, json_payload);
  // Also push to the popover content view if one is open (e.g. Settings).
  if (popover_ && popover_->IsOpen())
    PushToView(popover_->content_view(), event_name, json_payload);
  // Phase 9: per-kind *_view_ singletons are gone. Broadcast to every
  // tab's content browser via the TabManager.
  if (!shell_model_.tabs_) return;
  const auto snap = shell_model_.tabs_->Snapshot();
  fprintf(stderr, "[BroadcastToAllPanels] ev=%s tabs=%zu\n",
          event_name.c_str(), snap.size());
  fflush(stderr);
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
    // browser_view(). We are on TID_UI here so GetBrowser() is safe;
    // PushToView handles a null bv gracefully.
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
    fprintf(stderr, "[BroadcastToAllPanels] tab=%s kind=%s bv=%s\n",
            s.id.c_str(), TabKindToString(s.kind),
            bv ? "ok" : "NULL");
    fflush(stderr);
    PushToView(bv, event_name, json_payload);
  }
}

/*static*/ std::string MainWindow::JsEsc(const std::string& s) {
  std::string out;
  for (char c : s) {
    if      (c == '\\') out += "\\\\";
    else if (c == '"')  out += "\\\"";
    else if (c == '\'') out += "\\'";
    else if (c == '\n') out += "\\n";
    else if (c == '\r') out += "\\r";
    else                out += c;
  }
  return out;
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

std::string MainWindow::ResourceUrl(const std::string& relative_path) const {
  // Dev mode: when CRONYMAX_DEV is set, panels are served by Vite at
  // http://localhost:5173/<relative_path>. Allows HMR while the C++ shell
  // continues to mount each panel as its own CefBrowserView.
  if (const char* dev = std::getenv("CRONYMAX_DEV"); dev && *dev) {
    return std::string("http://localhost:5173/") + relative_path;
  }

  std::vector<std::filesystem::path> candidates;

  CefString resources_path;
  if (CefGetPath(PK_DIR_RESOURCES, resources_path)) {
    const auto resources = std::filesystem::path(resources_path.ToString());
    candidates.push_back(resources / "web" / relative_path);
    candidates.push_back(resources / relative_path);
  }

  CefString exe_path;
  if (CefGetPath(PK_DIR_EXE, exe_path)) {
    // PK_DIR_EXE is already a directory (Contents/MacOS on macOS).
    const auto exe_dir = std::filesystem::path(exe_path.ToString());
    candidates.push_back(exe_dir / "../Resources/web" / relative_path);
    candidates.push_back(exe_dir / "../../Resources/web" / relative_path);
  }

  const auto cwd = std::filesystem::current_path();
  candidates.push_back(cwd / "web" / relative_path);
  candidates.push_back(cwd / "../web" / relative_path);
  candidates.push_back(cwd / "../../web" / relative_path);

  for (const auto& candidate : candidates) {
    std::error_code ec;
    const auto normalized =
        std::filesystem::absolute(candidate, ec).lexically_normal();
    if (ec) continue;
    if (std::filesystem::exists(normalized, ec) && !ec) {
      return FileUrlFromPath(normalized);
    }
  }

  // Keep previous behavior as a deterministic fallback for diagnostics.
  if (!candidates.empty()) {
    return FileUrlFromPath(std::filesystem::absolute(candidates.front()));
  }

  return "about:blank";
}

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
  if (!sp) return;
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
  if (!shell_model_.tabs_) return;
  for (const auto& s : shell_model_.tabs_->Snapshot()) {
    if (s.kind != TabKind::kWeb) continue;
    auto it = tab_db_ids_.find(s.id);
    if (it == tab_db_ids_.end()) continue;
    Tab* t = shell_model_.tabs_->Get(s.id);
    if (!t) continue;
    auto* wb = static_cast<WebTabBehavior*>(t->behavior());
    if (!wb) continue;
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
  if (it == tab_db_ids_.end()) return;
  shell_model_.space_manager_.store().DeleteTab(it->second);
  tab_db_ids_.erase(it);
  tab_persisted_titles_.erase(tab_id);
}

// ---------------------------------------------------------------------------
// Sidebar tab persistence (chat + terminal tabs survive app restarts)
// ---------------------------------------------------------------------------

void MainWindow::PersistSidebarTabs() {
  if (!shell_model_.tabs_) return;
  nlohmann::json obj = nlohmann::json::object();
  nlohmann::json arr = nlohmann::json::array();
  for (const auto& s : shell_model_.tabs_->Snapshot()) {
    if (s.kind != TabKind::kChat && s.kind != TabKind::kTerminal) continue;
    nlohmann::json entry;
    entry["id"]          = s.id;
    entry["kind"]        = TabKindToString(s.kind);
    entry["displayName"] = s.display_name;
    nlohmann::json meta_obj = nlohmann::json::object();
    for (const auto& [k, v] : s.meta) meta_obj[k] = v;
    entry["meta"] = meta_obj;
    arr.push_back(std::move(entry));
  }
  obj["tabs"]        = std::move(arr);
  obj["activeTabId"] = shell_model_.tabs_->active_tab_id();
  shell_model_.space_manager_.store().SetKv("ui.sidebar_tabs", obj.dump());
}

bool MainWindow::RestoreSidebarTabs() {
  const std::string raw = shell_model_.space_manager_.store().GetKv("ui.sidebar_tabs");
  if (raw.empty()) return false;
  nlohmann::json obj;
  obj = nlohmann::json::parse(raw, nullptr, /*allow_exceptions=*/false);
  if (obj.is_discarded()) return false;
  const auto& arr = obj.value("tabs", nlohmann::json::array());
  if (!arr.is_array() || arr.empty()) return false;

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
    if (!kind_from_string_ok) continue;

    OpenParams params;
    params.display_name = entry.value("displayName", std::string{});
    const auto& meta_obj = entry.value("meta", nlohmann::json::object());
    if (meta_obj.is_object()) {
      for (const auto& [k, v] : meta_obj.items()) {
        if (v.is_string()) params.meta[k] = v.get<std::string>();
      }
    }

    const TabId id = shell_model_.tabs_->Open(kind, params);
    if (id.empty()) continue;
    // Tab::Register() in Open() already seeded theme colors.
    if (first_id.empty()) first_id = id;
    // Note: restored tabs get new IDs (TabManager::NewId), so we can't
    // map the stored activeTabId directly. We activate by position index.
  }

  // Activate: try to match by stored index (first chat tab if stored active
  // was a chat, first terminal if terminal). For simplicity just activate
  // the first restored tab.
  const std::string activate_id = first_id;
  if (!activate_id.empty()) shell_model_.tabs_->Activate(activate_id);
  return !first_id.empty();
}

// ---------------------------------------------------------------------------
// refine-ui-theme-layout: theme application + persistence
// ---------------------------------------------------------------------------

// static

void MainWindow::ApplyThemeChrome(const ThemeChrome& chrome) {
  shell_model_.current_chrome_ = chrome;
  // Native panels: titlebar + body share the chrome fill so the visual
  // chrome is one continuous region (per refine-ui-theme-layout design).
  if (body_panel_) body_panel_->SetBackgroundColor(chrome.bg_body);
#if defined(__APPLE__)
  // Phase 9: macOS window background + appearance set by TitleBarView::ApplyTheme.
  // Phase 8: corner-punch views handled by ContentView::ApplyTheme.
  // No-op block retained for documentation only.
#endif
  // Broadcast to renderers so each panel's installThemeMirror flips the
  // <html data-theme="…"> attribute and React listeners refresh.
  BroadcastToAllPanels("theme.changed", shell_model_.ThemeStateJson(/*include_chrome=*/true));
  // native-views-mvc Phase 4.5: notify subscribed view observers.
  // ThemeAwareView subscribers (titlebar, sidebar, content, popover, tabs)
  // receive ApplyTheme() via OnEvent() — no direct calls needed.
  shell_model_.NotifyThemeChanged(chrome);
}

void MainWindow::HandleThemeModeChange(const std::string& mode) {
  if (mode != "system" && mode != "light" && mode != "dark") return;
  shell_model_.theme_mode_ = mode;
  shell_model_.space_manager_.store().SetKv("ui.theme", mode);
  ApplyThemeChrome(ViewModel::ChromeFor(shell_model_.ResolveAppearance()));
}

void MainWindow::OnSystemAppearanceChanged() {
  // Only react in `system` mode; explicit Light/Dark pins ignore the OS.
  if (shell_model_.theme_mode_ != "system") return;
  ApplyThemeChrome(ViewModel::ChromeFor(shell_model_.ResolveAppearance()));
}

void MainWindow::SetContentOuterVInsets(int top, int bottom) {
  if (content_view_) content_view_->SetVInsets(top, bottom);
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

std::vector<std::pair<std::string, std::string>>
MainWindow::GetSpaces() const {
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

void MainWindow::RemoveActiveTabObserver(
    ViewObserver<ActiveTabChanged>* obs) {
  shell_model_.active_tab_observers.RemoveObserver(obs);
}

// WindowActionContext --------------------------------------------------
void MainWindow::SetTitleBarDragRegion(const CefRect& /*rect*/) {
  // Phase 3 stub: delegate to the existing full-recompute helper.
  // Phase 9 (TitleBarView) will pass the pre-computed rect directly.
  RefreshTitleBarDragRegion();
}

// OverlayActionContext -------------------------------------------------
void MainWindow::ShowFloat(const std::string& /*url*/) {
  // Phase 6+ (PopoverOverlay) will implement transient float panels.
}

void MainWindow::DismissFloat() {
  // Phase 6+ stub.
}

// ResourceContext ------------------------------------------------------
// (ResourceUrl implementation already exists above — override resolved
//  automatically since the signature matches the interface.)

}  // namespace cronymax
