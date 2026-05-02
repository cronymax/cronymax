#include "browser/main_window.h"

#include <cctype>
#include <cstdlib>
#include <filesystem>
#include <functional>
#include <utility>
#include <vector>

#include "include/base/cef_callback.h"
#include "include/cef_app.h"
#include "include/cef_path_util.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/views/cef_fill_layout.h"
#include "include/views/cef_panel_delegate.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

#if defined(__APPLE__)
#include "browser/mac_view_style.h"
#include "browser/tab.h"
#include "browser/tab_behavior.h"
#include "browser/tab_behaviors/web_tab_behavior.h"
#include "browser/tab_behaviors/simple_tab_behavior.h"
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

class SizedBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  explicit SizedBrowserViewDelegate(CefSize preferred_size)
      : preferred_size_(preferred_size) {}
  CefSize GetPreferredSize(CefRefPtr<CefView> view) override {
    (void)view; return preferred_size_;
  }
  cef_runtime_style_t GetBrowserRuntimeStyle() override {
    return CEF_RUNTIME_STYLE_ALLOY;
  }
 private:
  CefSize preferred_size_;
  IMPLEMENT_REFCOUNTING(SizedBrowserViewDelegate);
  DISALLOW_COPY_AND_ASSIGN(SizedBrowserViewDelegate);
};

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

#if defined(__APPLE__)
constexpr double kContentCornerRadius = 10.0;

// Round the corners of a content BrowserView so it floats with margin
// inside the window (Arc Browser style). Posted onto the UI runner so the
// underlying NSView is realized first.
[[maybe_unused]] void RoundContentCorners(CefRefPtr<CefBrowserView> v) {
  if (!v) return;
  CefPostTask(TID_UI, base::BindOnce([](CefRefPtr<CefBrowserView> view) {
                auto b = view->GetBrowser();
                if (!b) return;
                StyleContentBrowserView(b->GetHost()->GetWindowHandle(),
                                        kContentCornerRadius,
                                        /*with_shadow=*/true);
              }, v));
}
#else
inline void RoundContentCorners(CefRefPtr<CefBrowserView>) {}
#endif

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

// std::function-backed CefTextfieldDelegate; reports key presses so the
// popover URL field can navigate on Enter.
class FnTextfieldDelegate : public CefTextfieldDelegate {
 public:
  using KeyHandler = std::function<bool(int /*windows_key_code*/)>;
  explicit FnTextfieldDelegate(KeyHandler on_key)
      : on_key_(std::move(on_key)) {}
  bool OnKeyEvent(CefRefPtr<CefTextfield>,
                  const CefKeyEvent& event) override {
    if (event.type != KEYEVENT_RAWKEYDOWN) return false;
    return on_key_ ? on_key_(event.windows_key_code) : false;
  }
 private:
  KeyHandler on_key_;
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

MainWindow::MainWindow() : client_handler_(new ClientHandler(&space_manager_)) {}

void MainWindow::OnWindowCreated(CefRefPtr<CefWindow> window) {
  CEF_REQUIRE_UI_THREAD();
  window->SetTitle("cronymax");
  main_window_ = window;

  CefString res_path;
  std::filesystem::path db_dir;
  if (CefGetPath(PK_DIR_RESOURCES, res_path))
    db_dir = res_path.ToString();
  else
    db_dir = std::filesystem::current_path();

  if (!space_manager_.Init(db_dir / "cronymax.db"))
    LOG(ERROR) << "SpaceManager: failed to open database";

  // Phase A task 4.5: tell SpaceManager where the bundled built-in
  // doc-type YAMLs live so per-Space DocTypeRegistry can merge them with
  // workspace overrides.
  if (!res_path.empty()) {
    space_manager_.SetBuiltinDocTypesDir(
        std::filesystem::path(res_path.ToString()) / "builtin-doc-types");
  }

  if (space_manager_.spaces().empty())
    space_manager_.CreateSpace("Default", std::filesystem::current_path());

  // arc-style-tab-cards: TabManager owns every tab; per-kind *_view_
  // singletons are gone. All non-web kinds are singleton tabs whose
  // content browser loads the existing renderer HTML.
  tabs_ = std::make_unique<TabManager>();
  tabs_->SetClientHandler(client_handler_.get());
  // native-title-bar: terminal/chat are multi-instance now (each click of
  // "+ Terminal" / "+ Chat" creates a fresh tab). Agent/graph stay
  // singletons.
  tabs_->RegisterSingletonKind(TabKind::kAgent);
  tabs_->RegisterSingletonKind(TabKind::kGraph);
  tabs_->SetKindContentUrl(TabKind::kTerminal,
                           ResourceUrl("panels/terminal/index.html"));
  tabs_->SetKindContentUrl(TabKind::kChat,
                           ResourceUrl("panels/chat/index.html"));
  tabs_->SetKindContentUrl(TabKind::kAgent,
                           ResourceUrl("panels/agent/index.html"));
  tabs_->SetKindContentUrl(TabKind::kGraph,
                           ResourceUrl("panels/graph/index.html"));

  BuildChrome(window);

  // Open a default web tab so the window has visible content on startup.
  OpenWebTab("https://www.google.com");

#if defined(__APPLE__)
  // Arc-style: translucent NSWindow with hidden title bar. Posted onto the
  // UI runner so the NSWindow is fully realized first.
  CefPostTask(TID_UI, base::BindOnce([](CefRefPtr<CefWindow> w) {
                StyleMainWindowTranslucent(w->GetWindowHandle());
              }, window));
  // native-title-bar: install the AppKit drag overlay above the title-bar
  // spacer once the initial layout has run and the spacer has real bounds.
  CefPostTask(TID_UI, base::BindOnce(
      [](CefRefPtr<MainWindow> self) { self->RefreshTitleBarDragRegion(); },
      CefRefPtr<MainWindow>(this)));
#endif

  space_manager_.SetSwitchCallback(
      [this](const std::string& /*old_id*/, const std::string& new_id) {
        // 4.2: hide every currently-mounted tab card so the previous
        // Space's surface disappears atomically, then re-mount the active
        // tab. CEF `SetVisible(false)` keeps the renderer alive so there
        // is no reload cost on the next Space switch.
        for (auto& kv : mounted_cards_) {
          if (Tab* t = tabs_ ? tabs_->Get(kv.first) : nullptr) {
            if (t->card()) t->card()->SetVisible(false);
          }
        }
        if (tabs_ && tabs_->Active()) {
          ShowActiveTab();
        }
        for (const auto& sp : space_manager_.spaces()) {
          if (sp->id == new_id) {
            PushToSidebar("shell.space_changed",
                          "{\"id\":\"" + new_id + "\",\"name\":\"" +
                              JsEsc(sp->name) + "\"}");
            break;
          }
        }
      });

  window->Show();
}

void MainWindow::OnWindowDestroyed(CefRefPtr<CefWindow> window) {
  CEF_REQUIRE_UI_THREAD();
  (void)window;
  ClosePopover();
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

  // ── Title bar ────────────────────────────────────────────────────────────
  titlebar_panel_ = BuildTitleBar();
  window->AddChildView(titlebar_panel_);
  root_layout->SetFlexForView(titlebar_panel_, 0);

  // ── Body row ─────────────────────────────────────────────────────────────
  body_panel_ = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings body_box;
  body_box.horizontal = true;
  auto body_layout = body_panel_->SetToBoxLayout(body_box);
  window->AddChildView(body_panel_);
  root_layout->SetFlexForView(body_panel_, 1);

  // ── Sidebar ──────────────────────────────────────────────────────────────
  // Sidebar uses a transparent CEF background so the NSVisualEffectView
  // vibrancy under the window shows through (matching the title bar).
  CefBrowserSettings shell_settings;
  shell_settings.background_color = 0x00000000;
  sidebar_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, ResourceUrl("panels/sidebar/index.html"), shell_settings,
      nullptr, nullptr, new SizedBrowserViewDelegate(CefSize(240, 900)));
  body_panel_->AddChildView(sidebar_view_);
  body_layout->SetFlexForView(sidebar_view_, 0);
#if defined(__APPLE__)
  // Clear the sidebar NSView's opaque chrome fill so AppKit's vibrancy
  // shows through cleanly. Posted so the underlying NSView is realized.
  CefPostTask(TID_UI, base::BindOnce([](CefRefPtr<CefBrowserView> v) {
                auto b = v->GetBrowser();
                if (!b) return;
                MakeBrowserViewTransparent(b->GetHost()->GetWindowHandle());
              }, sidebar_view_));
#endif

  // ── Content host: an outer box providing Arc-style insets, an inner
  //    FillLayout panel that swaps in the active tab's card root.
  auto content_outer = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings content_box;
  content_box.horizontal = false;
  // content_box.inside_border_insets = {6, 8, 8, 8};
  auto content_outer_layout = content_outer->SetToBoxLayout(content_box);
  body_panel_->AddChildView(content_outer);
  body_layout->SetFlexForView(content_outer, 1);

  content_panel_ = CefPanel::CreatePanel(nullptr);
  content_panel_->SetToFillLayout();
  content_outer->AddChildView(content_panel_);
  content_outer_layout->SetFlexForView(content_panel_, 1);

  // ── Shell callbacks (TabManager-backed) ────────────────────────────────
  ShellCallbacks sh;

  // Build a TabManager-backed JSON snapshot of the web tab list using the
  // legacy {id, url, title, is_pinned} shape. The id is now a string;
  // sidebar parsers rely on the broadened TabIdPayloadSchema (Phase 2)
  // to accept it. Phase 10 replaces this with shell.tabs_list/TabSummary.
  sh.list_tabs = [this]() -> std::string {
    std::string out = "{\"tabs\":[";
    bool first = true;
    std::string active = "null";
    for (const auto& s : tabs_->Snapshot()) {
      if (s.kind != TabKind::kWeb) continue;
      Tab* t = tabs_->Get(s.id);
      if (!t) continue;
      auto* wb = static_cast<WebTabBehavior*>(t->behavior());
      if (!first) out += ",";
      first = false;
      out += "{\"id\":\"";
      out += JsEsc(s.id);
      out += "\",\"url\":\"";
      out += JsEsc(wb ? wb->current_url() : "");
      out += "\",\"title\":\"";
      out += JsEsc(wb ? wb->current_title() : s.display_name);
      out += "\",\"is_pinned\":false}";
      if (s.id == tabs_->active_tab_id()) {
        active = std::string("\"") + JsEsc(s.id) + "\"";
      }
    }
    out += "],\"active_tab_id\":";
    out += active;
    out += "}";
    return out;
  };

  sh.new_tab = [this](const std::string& url) -> std::string {
    const std::string raw = url.empty() ? "https://www.google.com" : url;
    const TabId id = OpenWebTab(raw);
    if (id.empty()) return "{}";
    const std::string final_url =
        raw.find("://") == std::string::npos ? "https://" + raw : raw;
    std::string json = "{\"id\":\"";
    json += JsEsc(id);
    json += "\",\"url\":\"";
    json += JsEsc(final_url);
    json += "\",\"title\":\"\",\"is_pinned\":false}";
    PushToSidebar("shell.tab_created", json);
    return json;
  };

  // Legacy int sh.switch_tab/sh.close_tab are deliberately left unset; the
  // TabManager-backed string-id callbacks (tab_activate_str / tab_close_str)
  // below replace them. Phase 9: sh.show_panel and sh.set_drag_regions are
  // removed — every panel/kind is now a tab.

  sh.navigate = [this](const std::string& url) {
    Tab* tab = tabs_->Active();
    if (!tab || tab->kind() != TabKind::kWeb) {
      OpenWebTab(url);
      return;
    }
    if (auto* wb = static_cast<WebTabBehavior*>(tab->behavior())) {
      wb->Navigate(url);
    }
  };

  sh.go_back = [this]() {
    Tab* tab = tabs_->Active();
    if (!tab || tab->kind() != TabKind::kWeb) return;
    if (auto* wb = static_cast<WebTabBehavior*>(tab->behavior())) wb->GoBack();
  };

  sh.go_forward = [this]() {
    Tab* tab = tabs_->Active();
    if (!tab || tab->kind() != TabKind::kWeb) return;
    if (auto* wb = static_cast<WebTabBehavior*>(tab->behavior()))
      wb->GoForward();
  };

  sh.popover_open  = [this](const std::string& u) { OpenPopover(u); };
  sh.popover_close = [this]() { ClosePopover(); };
  sh.popover_refresh = [this]() {
    if (popover_view_ && popover_view_->GetBrowser())
      popover_view_->GetBrowser()->Reload();
  };
  sh.popover_open_as_tab = [this]() {
    if (!popover_view_ || !popover_view_->GetBrowser()) return;
    const std::string url =
        popover_view_->GetBrowser()->GetMainFrame()->GetURL().ToString();
    ClosePopover();
    const TabId id = OpenWebTab(url);
    if (id.empty()) return;
    std::string json = "{\"id\":\"";
    json += JsEsc(id);
    json += "\",\"url\":\"";
    json += JsEsc(url);
    json += "\",\"title\":\"\",\"is_pinned\":false}";
    PushToSidebar("shell.tab_created", json);
  };

  sh.reload = [this]() {
    Tab* tab = tabs_->Active();
    if (!tab || tab->kind() != TabKind::kWeb) return;
    if (auto* wb = static_cast<WebTabBehavior*>(tab->behavior())) wb->Reload();
  };

  sh.terminal_restart = [this]() {
    // Phase 9: terminal restart is broadcast to all panels (the active
    // terminal tab's content browser receives it). Renderer ignores when
    // not the addressee.
    BroadcastToAllPanels("terminal.restart_requested", "{}");
  };

  sh.window_drag = [this]() {
#if defined(__APPLE__)
    if (main_window_) {
      PerformWindowDrag(main_window_->GetWindowHandle());
    }
#endif
  };

  sh.broadcast_event = [this](const std::string& ev, const std::string& body) {
    BroadcastToAllPanels(ev, body);
  };

  // ── arc-style-tab-cards (Phase 2): TabManager-backed callbacks ────────
  // These coexist with the legacy BrowserManager-backed callbacks above.
  // They are no-ops until Phases 3-8 register concrete TabBehaviors.
  auto kind_from_string = [](const std::string& s,
                              TabKind* out) -> bool {
    if (s == "web")      { *out = TabKind::kWeb; return true; }
    if (s == "terminal") { *out = TabKind::kTerminal; return true; }
    if (s == "chat")     { *out = TabKind::kChat; return true; }
    if (s == "agent")    { *out = TabKind::kAgent; return true; }
    if (s == "graph")    { *out = TabKind::kGraph; return true; }
    return false;
  };

  sh.tab_activate_str = [this](const std::string& tab_id) -> bool {
    Tab* tab = tabs_->Get(tab_id);
    if (!tab) return false;
    tabs_->Activate(tab_id);
    return true;
  };

  sh.tab_close_str = [this](const std::string& tab_id) -> bool {
    Tab* tab = tabs_->Get(tab_id);
    if (!tab) return false;
    const int closed_browser_id = tab->browser_id();
    if (tab->card()) {
      content_panel_->RemoveChildView(tab->card());
    }
    mounted_cards_.erase(tab_id);
    PersistTabClosed(tab_id);
    if (closed_browser_id != 0 &&
        popover_owner_browser_id_ == closed_browser_id) {
      ClosePopover();
    }
    tabs_->Close(tab_id);
    PushToSidebar("shell.tab_closed",
                  std::string("{\"id\":\"") + JsEsc(tab_id) + "\"}");
    // Promote any remaining tab.
    if (tabs_->active_tab_id().empty()) {
      const auto snap = tabs_->Snapshot();
      if (!snap.empty()) tabs_->Activate(snap.front().id);
    }
    return true;
  };

  sh.tab_open_singleton =
      [this, kind_from_string](const std::string& kind_s) -> std::string {
    TabKind kind;
    if (!kind_from_string(kind_s, &kind)) {
      return "{\"tabId\":\"\",\"created\":false}";
    }
    // native-title-bar: reject non-singleton kinds explicitly so any
    // leftover renderer caller fails loudly instead of silently turning
    // multi-instance into singleton.
    if (!tabs_->IsSingletonKind(kind)) {
      return "{\"tabId\":\"\",\"created\":false}";
    }
    bool created = false;
    TabId id = tabs_->FindOrCreateSingleton(kind, &created);
    if (!id.empty()) tabs_->Activate(id);
    std::string out = "{\"tabId\":\"";
    out += JsEsc(id);
    out += "\",\"created\":";
    out += created ? "true" : "false";
    out += "}";
    return out;
  };

  // native-title-bar: one button → one new tab. Web/terminal/chat are the
  // shipped buttons; agent/graph remain singleton-only via dock activation.
  sh.new_tab_kind =
      [this, kind_from_string](const std::string& kind_s) -> std::string {
    TabKind kind;
    if (!kind_from_string(kind_s, &kind)) return "{}";
    TabId id;
    if (kind == TabKind::kWeb) {
      id = OpenWebTab("https://www.google.com");
    } else if (kind == TabKind::kTerminal || kind == TabKind::kChat) {
      id = tabs_->Open(kind, OpenParams{});
      if (!id.empty()) tabs_->Activate(id);
    } else {
      // Other kinds aren't surfaced from the title bar today.
      return "{}";
    }
    if (id.empty()) return "{}";
    // Mirror the existing shell.tab_created shape (numeric id) used by the
    // sidebar BrowserTab schema. Strip the "tab-" prefix and atoi.
    int numeric = 0;
    static constexpr char kPrefix[] = "tab-";
    if (id.compare(0, sizeof(kPrefix) - 1, kPrefix) == 0) {
      numeric = std::atoi(id.c_str() + sizeof(kPrefix) - 1);
    }
    std::string created = "{\"id\":";
    created += std::to_string(numeric);
    created += ",\"url\":\"";
    if (kind == TabKind::kWeb) created += "https://www.google.com";
    created += "\",\"title\":\"\",\"is_pinned\":false}";
    PushToSidebar("shell.tab_created", created);

    std::string resp = "{\"tabId\":\"";
    resp += JsEsc(id);
    resp += "\",\"kind\":\"";
    resp += kind_s;
    resp += "\"}";
    return resp;
  };

  sh.set_toolbar_state =
      [this, kind_from_string](const std::string& tab_id,
                               const std::string& state_json) -> bool {
    Tab* tab = tabs_->Get(tab_id);
    if (!tab) return false;
    // The state JSON's "kind" field must match the tab's kind.
    auto kind_at = state_json.find("\"kind\"");
    if (kind_at == std::string::npos) return false;
    auto colon = state_json.find(':', kind_at);
    if (colon == std::string::npos) return false;
    auto q1 = state_json.find('"', colon);
    if (q1 == std::string::npos) return false;
    auto q2 = state_json.find('"', q1 + 1);
    if (q2 == std::string::npos) return false;
    const std::string kind_s = state_json.substr(q1 + 1, q2 - q1 - 1);
    TabKind kind;
    if (!kind_from_string(kind_s, &kind)) return false;
    if (kind != tab->kind()) return false;
    tab->OnToolbarState(ToolbarState{kind, state_json});
    return true;
  };

  sh.set_chrome_theme = [this](const std::string& tab_id,
                                const std::string& css) -> bool {
    Tab* tab = tabs_->Get(tab_id);
    if (!tab) return false;
    tab->SetChromeTheme(css);
    return true;
  };

  // Emitter hook: broadcast snapshot + active id whenever TabManager mutates.
  tabs_->SetOnChange([this]() {
    // shell.tabs_list snapshot
    const auto snap = tabs_->Snapshot();
    std::string js = "{\"tabs\":[";
    for (size_t i = 0; i < snap.size(); ++i) {
      if (i) js += ",";
      js += "{\"kind\":\"";
      js += TabKindToString(snap[i].kind);
      js += "\",\"id\":\"";
      js += JsEsc(snap[i].id);
      js += "\",\"displayName\":\"";
      js += JsEsc(snap[i].display_name);
      js += "\"}";
    }
    js += "],\"activeTabId\":";
    if (tabs_->active_tab_id().empty()) {
      js += "null";
    } else {
      js += "\"";
      js += JsEsc(tabs_->active_tab_id());
      js += "\"";
    }
    js += "}";
    BroadcastToAllPanels("shell.tabs_list", js);

    if (!tabs_->active_tab_id().empty()) {
      std::string a = "{\"tabId\":\"";
      a += JsEsc(tabs_->active_tab_id());
      a += "\"}";
      BroadcastToAllPanels("shell.tab_activated", a);
    }

    // Phase 9: swap the visible card in content_panel_ to the active tab.
    ShowActiveTab();

    // 4.5: persist any title changes to SpaceStore so the active Space's
    // tabs come back with the right names after a switch / restart.
    PersistTabTitlesIfChanged();
  });

  client_handler_->SetShellCallbacks(std::move(sh));

  // ── Browser event callbacks (Phase 4: TabManager-routed) ──────────────
  // WebTabBehavior already registers per-browser listeners with
  // ClientHandler when its browser is realized, so it owns the toolbar UI
  // updates. The callbacks below are kept for cross-cutting concerns:
  // sidebar event mirroring and popover URL display.
  client_handler_->on_browser_created = [](int /*browser_id*/) {
    // No-op: per-tab pairing happens inside WebTabBehavior. Popover
    // browsers are paired via owner_browser_id passed to OpenPopover.
  };

  client_handler_->on_title_change =
      [this](int browser_id, const std::string& title) {
        Tab* t = tabs_->FindByBrowserId(browser_id);
        if (!t) return;
        PushToSidebar("shell.tab_title_changed",
                      std::string("{\"id\":\"") + JsEsc(t->tab_id()) +
                          "\",\"title\":\"" + JsEsc(title) + "\"}");
      };

  client_handler_->on_address_change =
      [this](int browser_id, const std::string& url) {
        // Mirror popover content URL into its address bar.
        if (popover_view_ && popover_view_->GetBrowser() &&
            popover_view_->GetBrowser()->GetIdentifier() == browser_id) {
          popover_content_browser_id_ = browser_id;
          if (popover_url_field_) popover_url_field_->SetText(url);
          return;
        }
        Tab* t = tabs_->FindByBrowserId(browser_id);
        if (!t) return;
        PushToSidebar("shell.tab_url_changed",
                      std::string("{\"id\":\"") + JsEsc(t->tab_id()) +
                          "\",\"url\":\"" + JsEsc(url) + "\"}");
      };

  client_handler_->on_popup_request =
      [this](int browser_id, const std::string& url) -> bool {
    OpenPopover(url, browser_id);
    return true;  // suppress native popup
  };

#if defined(__APPLE__)
  // Forward CSS draggable-region updates from the sidebar to the native
  // overlay. The topbar pump is gone (Phase 9); sidebar still uses
  // -webkit-app-region: drag for its top strip.
  client_handler_->on_draggable_regions_changed =
      [this](int browser_id,
             const std::vector<CefDraggableRegion>& regions) {
    if (!sidebar_view_) return;
    auto b = sidebar_view_->GetBrowser();
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
}

// ---------------------------------------------------------------------------
// Tab card mounting (Phase 9: content_panel_ is the universal card host).
// ---------------------------------------------------------------------------

std::string MainWindow::OpenWebTab(const std::string& url) {
  const std::string final_url =
      url.find("://") == std::string::npos ? "https://" + url : url;
  OpenParams params;
  params.url = final_url;
  TabId id = tabs_->Open(TabKind::kWeb, params);
  if (id.empty()) return {};
  PersistTabCreated(id, final_url, "");
  tabs_->Activate(id);  // triggers ShowActiveTab via on_change
  return id;
}

void MainWindow::ShowActiveTab() {
  Tab* active = tabs_->Active();
  // Hide every card we've ever mounted; show only the active.
  for (auto& kv : mounted_cards_) {
    if (Tab* t = tabs_->Get(kv.first)) {
      if (t->card() && t != active) t->card()->SetVisible(false);
    }
  }
  if (!active || !active->card()) return;
  if (!mounted_cards_[active->tab_id()]) {
    content_panel_->AddChildView(active->card());
    mounted_cards_[active->tab_id()] = true;
  }
  active->card()->SetVisible(true);
  content_panel_->Layout();

  // Activating a tab causes CEF to add (or re-parent) the browser's NSView
  // under the window's contentView, which can land on top of our title-bar
  // drag overlay. Re-raise it via a deferred UI tick so it sits above the
  // freshly-mounted browser surface.
#if defined(__APPLE__)
  CefPostTask(TID_UI, base::BindOnce(
      [](CefRefPtr<MainWindow> self) { self->RefreshTitleBarDragRegion(); },
      CefRefPtr<MainWindow>(this)));
#endif

  // For web tabs, give the content browser focus so keyboard input works.
  if (active->kind() == TabKind::kWeb) {
    if (auto* wb = static_cast<WebTabBehavior*>(active->behavior())) {
      if (auto bv = wb->browser_view()) bv->RequestFocus();
    }
  }

  UpdatePopoverVisibility();
}

// ---------------------------------------------------------------------------
// Popover
// ---------------------------------------------------------------------------

namespace {
[[maybe_unused]] std::string PercentEncodeAll(const std::string& s) {
  static constexpr char kHex[] = "0123456789ABCDEF";
  std::string out;
  out.reserve(s.size() + 16);
  for (unsigned char ch : s) {
    if (std::isalnum(ch) || ch == '-' || ch == '_' || ch == '.' || ch == '~') {
      out.push_back(static_cast<char>(ch));
    } else {
      out.push_back('%');
      out.push_back(kHex[(ch >> 4) & 0xF]);
      out.push_back(kHex[ch & 0xF]);
    }
  }
  return out;
}

#if defined(__APPLE__)
constexpr double kPopoverCornerRadius = 12.0;

[[maybe_unused]] void StylePopoverChrome(CefRefPtr<CefBrowserView> v) {
  if (!v) return;
  auto b = v->GetBrowser();
  if (!b) return;
  StyleOverlayBrowserView(b->GetHost()->GetWindowHandle(),
                          kPopoverCornerRadius,
                          kCornerTop,
                          /*with_shadow=*/true);
}

void StylePopoverContent(CefRefPtr<CefBrowserView> v) {
  if (!v) return;
  auto b = v->GetBrowser();
  if (!b) return;
  StyleOverlayBrowserView(b->GetHost()->GetWindowHandle(),
                          kPopoverCornerRadius,
                          kCornerBottom,
                          /*with_shadow=*/true);
}
#else
inline void StylePopoverChrome(CefRefPtr<CefBrowserView>) {}
inline void StylePopoverContent(CefRefPtr<CefBrowserView>) {}
#endif
}  // namespace

void MainWindow::OpenPopover(const std::string& url, int owner_browser_id) {
  if (!main_window_) return;

  // If a popover already exists, just navigate it and re-pair owner.
  if (popover_overlay_ && popover_overlay_->IsValid()) {
    popover_owner_browser_id_ = owner_browser_id;
    if (popover_view_) {
      auto b = popover_view_->GetBrowser();
      if (b) b->GetMainFrame()->LoadURL(url);
    }
    if (popover_url_field_) popover_url_field_->SetText(url);
    LayoutPopover();
    UpdatePopoverVisibility();
    if (popover_view_) popover_view_->RequestFocus();
    return;
  }

  popover_owner_browser_id_ = owner_browser_id;

  CefBrowserSettings bs;

  // 1) Content view (the actual popover URL) — overlay #1.
  popover_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, url, bs, nullptr, nullptr,
      new AlloyBrowserViewDelegate());
  popover_overlay_ = main_window_->AddOverlayView(
      popover_view_, CEF_DOCKING_MODE_CUSTOM, /*can_activate=*/true);

  // 2) Native chrome strip (URL textfield + action buttons) — overlay #2,
  //    added last so it sits above the content overlay.
  popover_chrome_panel_ = BuildPopoverChromePanel(url);
  popover_chrome_overlay_ = main_window_->AddOverlayView(
      popover_chrome_panel_, CEF_DOCKING_MODE_CUSTOM, /*can_activate=*/true);

  LayoutPopover();
  UpdatePopoverVisibility();
  if (popover_view_) popover_view_->RequestFocus();

  // Round corners + drop shadow on the content BrowserView NSView. Posted
  // onto the UI task runner so the view's NSView/superview is realized
  // first. The chrome strip is a CefPanel (no browser handle) so its
  // background colour does the visual lift instead of a CALayer mask.
  CefPostTask(TID_UI, base::BindOnce([](CefRefPtr<CefBrowserView> content) {
                StylePopoverContent(content);
              },
              popover_view_));
}

void MainWindow::ClosePopover() {
  if (popover_chrome_overlay_) {
    popover_chrome_overlay_->SetVisible(false);
    popover_chrome_overlay_->Destroy();
    popover_chrome_overlay_ = nullptr;
  }
  if (popover_overlay_) {
    popover_overlay_->SetVisible(false);
    popover_overlay_->Destroy();
    popover_overlay_ = nullptr;
  }
  popover_view_ = nullptr;
  popover_chrome_panel_ = nullptr;
  popover_url_field_ = nullptr;
  popover_root_ = nullptr;
  popover_owner_browser_id_ = 0;
  popover_content_browser_id_ = 0;
}

void MainWindow::UpdatePopoverVisibility() {
  if (!popover_overlay_ || !popover_overlay_->IsValid()) return;
  // Popover is only visible when its owner web tab is active.
  bool visible = false;
  if (popover_owner_browser_id_ != 0) {
    Tab* active = tabs_->Active();
    visible = (active && active->kind() == TabKind::kWeb &&
               active->browser_id() == popover_owner_browser_id_);
  }
  popover_overlay_->SetVisible(visible);
  if (popover_chrome_overlay_) popover_chrome_overlay_->SetVisible(visible);
}

void MainWindow::LayoutPopover() {
  if (!main_window_ || !popover_overlay_ || !popover_overlay_->IsValid()) return;
  const CefRect bounds = main_window_->GetBounds();
  // Float over the content (tab) pane: account for sidebar and topbar so we
  // appear centered relative to the tab area.
  constexpr int kSidebarW = 240;
  // native-title-bar: matches the kTitleBarH constant used by BuildTitleBar.
  constexpr int kTitleBarH = 38;
  constexpr int kChromeH  = 44;
  const int content_x = kSidebarW;
  const int content_y = kTitleBarH;
  const int content_w = std::max(320, bounds.width  - kSidebarW);
  const int content_h = std::max(360, bounds.height - kTitleBarH);
  // ~80% of content area, centered, capped to keep it readable.
  const int w = std::min(1280, std::max(560, content_w * 80 / 100));
  const int h = std::min(960,  std::max(420, content_h * 88 / 100));
  const int x = content_x + (content_w - w) / 2;
  const int y = content_y + (content_h - h) / 2;
  // Address bar on top, content below.
  if (popover_chrome_overlay_ && popover_chrome_overlay_->IsValid()) {
    popover_chrome_overlay_->SetBounds(CefRect(x, y, w, kChromeH));
  }
  popover_overlay_->SetBounds(
      CefRect(x, y + kChromeH, w, std::max(80, h - kChromeH)));

  // Re-assert corner mask + shadow after CEF lays out / re-parents.
  StylePopoverContent(popover_view_);
}

void MainWindow::OnWindowBoundsChanged(CefRefPtr<CefWindow> window,
                                       const CefRect& new_bounds) {
  (void)window; (void)new_bounds;
  LayoutPopover();
  RefreshTitleBarDragRegion();
}

// ---------------------------------------------------------------------------
// Popover chrome (native)
// ---------------------------------------------------------------------------

namespace {
constexpr cef_color_t kPopoverChromeBg  = 0xFFF4F4F6;  // light strip
constexpr cef_color_t kPopoverChromeFg  = 0xFF1F1F22;
constexpr cef_color_t kPopoverPillBg    = 0xFFFFFFFF;
constexpr cef_color_t kPopoverPillFg    = 0xFF1F1F22;
constexpr cef_color_t kPopoverBtnFg     = 0xFF3A3A3F;
}  // namespace

CefRefPtr<CefPanel> MainWindow::BuildPopoverChromePanel(
    const std::string& initial_url) {
  auto panel = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings root_box;
  root_box.horizontal = true;
  root_box.inside_border_insets = {6, 10, 6, 10};
  root_box.between_child_spacing = 6;
  auto layout = panel->SetToBoxLayout(root_box);
  panel->SetBackgroundColor(kPopoverChromeBg);

  // URL textfield (flex 1).
  popover_url_field_ = CefTextfield::CreateTextfield(
      new FnTextfieldDelegate([this](int vk) -> bool {
        if (vk == 0x0D /* VK_RETURN */) {
          NavigatePopoverToFieldUrl();
          return true;
        }
        return false;
      }));
  popover_url_field_->SetText(initial_url);
  popover_url_field_->SetBackgroundColor(kPopoverPillBg);
  popover_url_field_->SetTextColor(kPopoverPillFg);
  panel->AddChildView(popover_url_field_);
  layout->SetFlexForView(popover_url_field_, 1);

  auto add_btn = [&](const std::string& label, const std::string& tooltip,
                     std::function<void()> on_click) {
    auto btn = CefLabelButton::CreateLabelButton(
        new FnButtonDelegate(std::move(on_click)), label);
    btn->SetTextColor(CEF_BUTTON_STATE_NORMAL, kPopoverBtnFg);
    btn->SetTooltipText(tooltip);
    panel->AddChildView(btn);
    layout->SetFlexForView(btn, 0);
  };
  // Refresh
  add_btn("\u21BB" /* ↻ */, "Reload", [this]() {
    if (popover_view_ && popover_view_->GetBrowser())
      popover_view_->GetBrowser()->Reload();
  });
  // Open as tab
  add_btn("\u2197" /* ↗ */, "Open as tab", [this]() {
    if (!popover_view_ || !popover_view_->GetBrowser()) return;
    const std::string url =
        popover_view_->GetBrowser()->GetMainFrame()->GetURL().ToString();
    // Defer: ClosePopover() will tear down the panel that owns the button
    // and this very click delegate. Returning from the click first avoids a
    // use-after-free of the std::function being invoked.
    CefPostTask(TID_UI, base::BindOnce(
        [](CefRefPtr<MainWindow> self, std::string u) {
          self->ClosePopover();
          const TabId id = self->OpenWebTab(u);
          if (id.empty()) return;
          // The sidebar BrowserTab schema requires a numeric id; TabManager
          // hands out string ids of the form "tab-N". Strip the prefix so
          // the shell.tab_created event passes Zod validation and the new
          // tab actually shows up in the sidebar list.
          int numeric = 0;
          static constexpr char kPrefix[] = "tab-";
          if (id.compare(0, sizeof(kPrefix) - 1, kPrefix) == 0) {
            numeric = std::atoi(id.c_str() + sizeof(kPrefix) - 1);
          }
          std::string json = "{\"id\":";
          json += std::to_string(numeric);
          json += ",\"url\":\"";
          json += MainWindow::JsEsc(u);
          json += "\",\"title\":\"\",\"is_pinned\":false}";
          self->PushToSidebar("shell.tab_created", json);
        },
        CefRefPtr<MainWindow>(this), url));
  });
  // Close
  add_btn("\u2715" /* ✕ */, "Close", [this]() {
    CefPostTask(TID_UI, base::BindOnce(
        [](CefRefPtr<MainWindow> self) { self->ClosePopover(); },
        CefRefPtr<MainWindow>(this)));
  });

  (void)kPopoverChromeFg;
  return panel;
}

void MainWindow::NavigatePopoverToFieldUrl() {
  if (!popover_view_ || !popover_url_field_) return;
  const std::string url = popover_url_field_->GetText().ToString();
  if (url.empty()) return;
  if (auto b = popover_view_->GetBrowser())
    b->GetMainFrame()->LoadURL(url);
}

// ---------------------------------------------------------------------------
// Native title bar (CefPanel)
// ---------------------------------------------------------------------------

namespace {
constexpr int kTitleBarH = 38;
// Solid chrome color — matches the window background (#14141a in
// mac_view_style.mm) so the title-bar panel and the sidebar paint
// exactly the same pixel value.
constexpr cef_color_t kTitleBarBg     = 0xFF14141A;
constexpr cef_color_t kTitleBarBtnFg  = 0xFFE5E5EA;
#if defined(__APPLE__)
constexpr int kLightsPadW = 78;
constexpr int kWinPadW    = 0;
#else
constexpr int kLightsPadW = 0;
constexpr int kWinPadW    = 138;  // reserved for the Windows port
#endif
}  // namespace

CefRefPtr<CefPanel> MainWindow::BuildTitleBar() {
  auto panel = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(0, kTitleBarH)));
  panel->SetBackgroundColor(kTitleBarBg);

  CefBoxLayoutSettings box;
  box.horizontal = true;
  box.inside_border_insets = {6, 8, 6, 8};
  box.between_child_spacing = 6;
  auto layout = panel->SetToBoxLayout(box);

  // 1. macOS traffic-light reservation.
  lights_pad_ = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(kLightsPadW, kTitleBarH - 12)));
  panel->AddChildView(lights_pad_);
  layout->SetFlexForView(lights_pad_, 0);

  // 2. Drag spacer (drag overlay attaches here on macOS).
  spacer_ = CefPanel::CreatePanel(nullptr);
  panel->AddChildView(spacer_);
  layout->SetFlexForView(spacer_, 1);

  // 3. New-tab buttons.
  auto add_btn = [&](CefRefPtr<CefLabelButton>* slot, const std::string& label,
                     const std::string& tooltip, const std::string& kind) {
    auto btn = CefLabelButton::CreateLabelButton(
        new FnButtonDelegate([this, kind]() {
          // Defer to a UI tick so click handler unwinds before any tab
          // mutation walks the view tree.
          CefPostTask(TID_UI, base::BindOnce(
              [](CefRefPtr<MainWindow> self, std::string k) {
                self->OpenNewTabKind(k);
              },
              CefRefPtr<MainWindow>(this), kind));
        }),
        label);
    btn->SetTextColor(CEF_BUTTON_STATE_NORMAL, kTitleBarBtnFg);
    btn->SetTextColor(CEF_BUTTON_STATE_HOVERED, 0xFFFFFFFF);
    btn->SetTooltipText(tooltip);
    panel->AddChildView(btn);
    layout->SetFlexForView(btn, 0);
    *slot = btn;
  };
  add_btn(&btn_web_,  "\xE2\x8A\x95 Web",       "New web tab",  "web");
  add_btn(&btn_term_, "\xE2\x8C\xA8 Terminal",  "New terminal", "terminal");
  add_btn(&btn_chat_, "\xF0\x9F\x92\xAC Chat",  "New chat",     "chat");

  // Settings: opens the agent singleton tab (now hosting the Settings UI).
  // Treated as a separate code path because it activates an existing
  // singleton instead of creating a new tab.
  {
    auto btn = CefLabelButton::CreateLabelButton(
        new FnButtonDelegate([this]() {
          CefPostTask(TID_UI, base::BindOnce(
              [](CefRefPtr<MainWindow> self) {
                bool created = false;
                TabId id = self->tabs_->FindOrCreateSingleton(
                    TabKind::kAgent, &created);
                if (!id.empty()) self->tabs_->Activate(id);
              },
              CefRefPtr<MainWindow>(this)));
        }),
        "\xE2\x9A\x99 Settings");
    btn->SetTextColor(CEF_BUTTON_STATE_NORMAL, kTitleBarBtnFg);
    btn->SetTextColor(CEF_BUTTON_STATE_HOVERED, 0xFFFFFFFF);
    btn->SetTooltipText("Open settings");
    panel->AddChildView(btn);
    layout->SetFlexForView(btn, 0);
    btn_settings_ = btn;
  }

  // 4. Reserved Windows-controls slot (zero width on macOS).
  win_pad_ = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(kWinPadW, 1)));
  panel->AddChildView(win_pad_);
  layout->SetFlexForView(win_pad_, 0);

  return panel;
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
    id = tabs_->Open(k, OpenParams{});
    if (!id.empty()) tabs_->Activate(id);
  }
  if (id.empty()) return;

  // Mirror the existing shell.tab_created (numeric-id) shape so the
  // sidebar's BrowserTab Zod schema accepts the event.
  int numeric = 0;
  static constexpr char kPrefix[] = "tab-";
  if (id.compare(0, sizeof(kPrefix) - 1, kPrefix) == 0) {
    numeric = std::atoi(id.c_str() + sizeof(kPrefix) - 1);
  }
  std::string created = "{\"id\":";
  created += std::to_string(numeric);
  created += ",\"url\":\"";
  created += JsEsc(url_for_event);
  created += "\",\"title\":\"\",\"is_pinned\":false}";
  PushToSidebar("shell.tab_created", created);
}

void MainWindow::RefreshTitleBarDragRegion() {
#if defined(__APPLE__)
  if (!main_window_ || !titlebar_panel_) return;
  const CefRect bar = titlebar_panel_->GetBoundsInScreen();
  if (bar.width <= 0 || bar.height <= 0) return;
  const CefRect win = main_window_->GetBounds();
  // Title-bar rect in window-content coords (top-down).
  const CefRect bar_in_window(bar.x - win.x, bar.y - win.y, bar.width, bar.height);

  // Collect button rects so the overlay's hitTest punches holes for them
  // (clicks pass through to the CEF-rendered buttons).
  std::vector<CefRect> nodrag;
  auto add = [&](const CefRefPtr<CefLabelButton>& b) {
    if (!b) return;
    CefRect r = b->GetBoundsInScreen();
    if (r.width <= 0 || r.height <= 0) return;
    nodrag.emplace_back(r.x - win.x, r.y - win.y, r.width, r.height);
  };
  add(btn_web_);
  add(btn_term_);
  add(btn_chat_);
  add(btn_settings_);
  InstallTitleBarDragOverlay(main_window_->GetWindowHandle(),
                             bar_in_window,
                             nodrag.empty() ? nullptr : nodrag.data(),
                             nodrag.size());
#endif
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
  if (!browser) return;
  auto frame = browser->GetMainFrame();
  if (!frame) return;
  const std::string js =
      "window.__aiDesktopDispatch && window.__aiDesktopDispatch('" +
      event_name + "'," + json_payload + ");";
  frame->ExecuteJavaScript(js, frame->GetURL(), 0);
}

}  // namespace

void MainWindow::PushToSidebar(const std::string& event_name,
                               const std::string& json_payload) {
  PushToView(sidebar_view_, event_name, json_payload);
}

void MainWindow::BroadcastToAllPanels(const std::string& event_name,
                                      const std::string& json_payload) {
  PushToView(sidebar_view_, event_name, json_payload);
  // Phase 9: per-kind *_view_ singletons are gone. Broadcast to every
  // tab's content browser via the TabManager.
  if (!tabs_) return;
  for (const auto& s : tabs_->Snapshot()) {
    Tab* t = tabs_->Get(s.id);
    if (!t || !t->behavior()) continue;
    const int bid = t->browser_id();
    if (bid == 0) continue;
    // Find the corresponding CefBrowserView through whichever behavior
    // exposes one. Both WebTabBehavior and SimpleTabBehavior expose
    // browser_view().
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
  Space* sp = space_manager_.ActiveSpace();
  if (!sp) return;
  BrowserTabRow row;
  row.space_id = sp->id;
  row.url = url;
  row.title = title;
  row.is_pinned = false;
  row.last_accessed = 0;
  const int64_t db_id = space_manager_.store().CreateTab(row);
  if (db_id > 0) {
    tab_db_ids_[tab_id] = db_id;
    tab_persisted_titles_[tab_id] = title;
  }
}

void MainWindow::PersistTabTitlesIfChanged() {
  if (!tabs_) return;
  for (const auto& s : tabs_->Snapshot()) {
    if (s.kind != TabKind::kWeb) continue;
    auto it = tab_db_ids_.find(s.id);
    if (it == tab_db_ids_.end()) continue;
    Tab* t = tabs_->Get(s.id);
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
    space_manager_.store().UpdateTab(row);
    tab_persisted_titles_[s.id] = title;
  }
}

void MainWindow::PersistTabClosed(const std::string& tab_id) {
  auto it = tab_db_ids_.find(tab_id);
  if (it == tab_db_ids_.end()) return;
  space_manager_.store().DeleteTab(it->second);
  tab_db_ids_.erase(it);
  tab_persisted_titles_.erase(tab_id);
}

}  // namespace cronymax
