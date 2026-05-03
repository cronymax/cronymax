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
#include "browser/icon_registry.h"
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
[[maybe_unused]] void RoundContentCorners(CefRefPtr<CefWindow> win,
                                         CefRefPtr<CefBrowserView> bv,
                                         cef_color_t window_bg) {
  if (!win || !bv) return;
  CefPostTask(TID_UI, base::BindOnce(
      [](CefRefPtr<CefWindow> w, CefRefPtr<CefBrowserView> view,
         cef_color_t bg) {
        // Walk up: BrowserView -> content_host_ (FillLayout) -> card_
        // (BoxLayout that contains [toolbar, content_host_]).
        // We want the card_ bounds so the punch views land at the card's
        // four corners, covering both the toolbar top and content bottom.
        CefRefPtr<CefView> content_host = view->GetParentView();
        CefRefPtr<CefView> card =
            content_host ? content_host->GetParentView() : nullptr;
        // Fallback: if the hierarchy is shallower, use whatever we found.
        CefRefPtr<CefView> target = card ? card : (content_host ? content_host : view);
        CefPoint cardOrigin{0, 0};
        target->ConvertPointToWindow(cardOrigin);
        CefRect cardBounds = target->GetBounds();
        CefRect rel;
        rel.x      = cardOrigin.x;
        rel.y      = cardOrigin.y;
        rel.width  = cardBounds.width;
        rel.height = cardBounds.height;
        StyleContentBrowserView(w->GetWindowHandle(),
                                kContentCornerRadius,
                                bg,
                                rel);
        // Add a soft drop shadow around the card so it appears to float
        // above the window background, creating clear depth from titlebar.
        if (auto br = view->GetBrowser()) {
          AddContentCardShadow(br->GetHost()->GetWindowHandle());
        }
      }, win, bv, window_bg));
}
#else
inline void RoundContentCorners(CefRefPtr<CefWindow>, CefRefPtr<CefBrowserView>,
                                cef_color_t) {}
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

MainWindow::MainWindow() : client_handler_(new ClientHandler(&space_manager_)) {}

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
  tabs_->RegisterSingletonKind(TabKind::kSettings);
  tabs_->SetKindContentUrl(TabKind::kChat,
                           ResourceUrl("panels/chat/index.html"));
  tabs_->SetKindContentUrl(TabKind::kTerminal,
                           ResourceUrl("panels/terminal/index.html"));
  tabs_->SetKindContentUrl(TabKind::kSettings,
                           ResourceUrl("panels/settings/index.html"));

  // refine-ui-theme-layout: load persisted theme mode (defaults to
  // "system") and seed current_chrome_ before BuildChrome so the title
  // bar paints with the correct color on first frame.
  {
    std::string persisted = space_manager_.store().GetKv("ui.theme");
    if (persisted == "light" || persisted == "dark" || persisted == "system") {
      theme_mode_ = persisted;
    }
    current_chrome_ = ChromeFor(ResolveAppearance());
  }

  BuildChrome(window);

  // Open a default web tab so the window has visible content on startup.
  OpenWebTab("https://www.google.com");

#if defined(__APPLE__)
  // Arc-style: translucent NSWindow with hidden title bar. Posted onto the
  // UI runner so the NSWindow is fully realized first.
  CefPostTask(TID_UI, base::BindOnce([](CefRefPtr<CefWindow> w,
                                         cef_color_t bg) {
                StyleMainWindowTranslucent(w->GetWindowHandle(), bg);
              }, window, current_chrome_.bg_body));
  // refine-ui-theme-layout: install rounded 12 px frame + initial border
  // colour around the content panel. Posted so the NSView is realized.
  CefPostTask(TID_UI, base::BindOnce([](CefRefPtr<MainWindow> self) {
                self->ApplyThemeChrome(self->current_chrome_);
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

  // ── Content host: outer box providing Arc-style insets, then a
  //    `content_frame_` panel that paints a 12 px rounded chrome border
  //    around the inner FillLayout panel that hosts each tab's card.
  content_outer_ = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings content_box;
  content_box.horizontal = false;
  // refine-ui-theme-layout: 8 px breathing room around the rounded card on
  // all sides (including top) so the card floats inside the window chrome
  // and the titlebar/content boundary is visually distinct.
  content_box.inside_border_insets = {8, 8, 8, 8};
  auto content_outer_layout = content_outer_->SetToBoxLayout(content_box);
  body_panel_->AddChildView(content_outer_);
  body_layout->SetFlexForView(content_outer_, 1);

  content_frame_ = CefPanel::CreatePanel(nullptr);
  content_frame_->SetToFillLayout();
  content_outer_->AddChildView(content_frame_);
  content_outer_layout->SetFlexForView(content_frame_, 1);

  content_panel_ = CefPanel::CreatePanel(nullptr);
  content_panel_->SetToFillLayout();
  content_frame_->AddChildView(content_panel_);

  // ── Shell callbacks (TabManager-backed) ────────────────────────────────
  ShellCallbacks sh;

  // Build a TabManager-backed JSON snapshot of the web tab list using the
  // legacy {id, url, title, is_pinned} shape. The id is now a string;
  // sidebar parsers rely on the broadened TabIdPayloadSchema (Phase 2)
  // to accept it. Phase 10 replaces this with shell.tabs_list/TabSummary.
  // refine-ui-theme-layout: emit the unified TabsListSnapshot shape
  // {tabs:[{kind,id,displayName,...}], activeTabId} for both the request
  // and the broadcast so the sidebar can show every tab kind, not just
  // web. The legacy shape (id:int + is_pinned + url/title) is gone.
  sh.list_tabs = [this]() -> std::string {
    std::string out = "{\"tabs\":[";
    bool first = true;
    for (const auto& s : tabs_->Snapshot()) {
      if (!first) out += ",";
      first = false;
      out += "{\"kind\":\"";
      out += TabKindToString(s.kind);
      out += "\",\"id\":\"";
      out += JsEsc(s.id);
      out += "\",\"displayName\":\"";
      out += JsEsc(s.display_name);
      out += "\"";
      if (s.kind == TabKind::kWeb) {
        Tab* t = tabs_->Get(s.id);
        auto* wb = t ? static_cast<WebTabBehavior*>(t->behavior()) : nullptr;
        if (wb) {
          out += ",\"url\":\"";
          out += JsEsc(wb->current_url());
          out += "\"";
        }
      }
      out += "}";
    }
    out += "],\"activeTabId\":";
    if (tabs_->active_tab_id().empty()) {
      out += "null";
    } else {
      out += "\"";
      out += JsEsc(tabs_->active_tab_id());
      out += "\"";
    }
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
  // refine-ui-theme-layout: open the Settings panel as a popover.
  sh.settings_popover_open = [this]() {
    OpenPopover(ResourceUrl("panels/settings/index.html"));
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

  sh.popover_navigate = [this](const std::string& url) {
    if (popover_view_ && popover_view_->GetBrowser())
      popover_view_->GetBrowser()->GetMainFrame()->LoadURL(url);
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
    if (s == "chat")     { *out = TabKind::kChat; return true; }
    if (s == "terminal") { *out = TabKind::kTerminal; return true; }
    if (s == "settings")    { *out = TabKind::kSettings; return true; }
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
    if (Tab* tab = tabs_->Get(id)) {
      tab->ApplyTheme(current_chrome_.bg_base, current_chrome_.bg_float,
                      current_chrome_.text_title);
    }
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
      if (Tab* tab = tabs_->Get(id)) {
        tab->ApplyTheme(current_chrome_.bg_base, current_chrome_.bg_float,
                        current_chrome_.text_title);
      }
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
      js += "\"";
      if (snap[i].kind == TabKind::kWeb) {
        Tab* t = tabs_->Get(snap[i].id);
        auto* wb = t ? static_cast<WebTabBehavior*>(t->behavior()) : nullptr;
        if (wb) {
          js += ",\"url\":\"";
          js += JsEsc(wb->current_url());
          js += "\"";
        }
      }
      js += "}";
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

  // refine-ui-theme-layout: register theme.* bridge callbacks. get_mode
  // is synchronous (returns the current ThemeStateJson without chrome);
  // set_mode persists, recomputes, applies, and broadcasts.
  ThemeCallbacks theme_cbs;
  theme_cbs.get_mode = [this]() -> std::string {
    return ThemeStateJson(/*include_chrome=*/false);
  };
  theme_cbs.set_mode = [this](const std::string& mode) {
    CefPostTask(TID_UI, base::BindOnce(
        [](CefRefPtr<MainWindow> self, std::string m) {
          self->HandleThemeModeChange(m);
        },
        CefRefPtr<MainWindow>(this), mode));
  };
  client_handler_->SetThemeCallbacks(std::move(theme_cbs));

  // ── Browser event callbacks (Phase 4: TabManager-routed) ──────────────
  // WebTabBehavior already registers per-browser listeners with
  // ClientHandler when its browser is realized, so it owns the toolbar UI
  // updates. The callbacks below are kept for cross-cutting concerns:
  // sidebar event mirroring and popover URL display.
  client_handler_->on_browser_created = [this](int browser_id) {
    // Pair the chrome-strip BrowserView so its address-change events are
    // filtered out and not mistaken for content-browser navigation.
    if (popover_chrome_view_ && popover_chrome_view_->GetBrowser() &&
        popover_chrome_view_->GetBrowser()->GetIdentifier() == browser_id) {
      popover_chrome_browser_id_ = browser_id;
    }
    // Round the content corners now that the browser (and its NSView tree)
    // is fully initialized. ShowActiveTab posts the same call but
    // GetBrowser() is null on a brand-new tab at that point.
    Tab* t = tabs_ ? tabs_->FindByBrowserId(browser_id) : nullptr;
    if (t) {
      CefRefPtr<CefBrowserView> bv;
      if (t->kind() == TabKind::kWeb) {
        if (auto* wb = static_cast<WebTabBehavior*>(t->behavior()))
          bv = wb->browser_view();
      } else {
        if (auto* sb = static_cast<SimpleTabBehavior*>(t->behavior()))
          bv = sb->browser_view();
      }
      RoundContentCorners(main_window_, bv, current_chrome_.bg_body);
    }
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
        // Ignore address changes from the chrome-strip BrowserView itself.
        if (browser_id == popover_chrome_browser_id_) return;
        // Mirror popover content URL into the HTML chrome strip.
        if (popover_view_ && popover_view_->GetBrowser() &&
            popover_view_->GetBrowser()->GetIdentifier() == browser_id) {
          popover_content_browser_id_ = browser_id;
          PushToView(popover_chrome_view_,
                     "popover_chrome.url_changed",
                     "{\"url\":\"" + JsEsc(url) + "\"}");
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
  if (Tab* tab = tabs_->Get(id)) {
    tab->ApplyTheme(current_chrome_.bg_base, current_chrome_.bg_float,
                    current_chrome_.text_title);
  }
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

  // Round content BrowserView corners on every activation — CEF may
  // re-parent the NSView when SetVisible changes, so we re-assert.
  {
    CefRefPtr<CefBrowserView> bv;
    if (active->kind() == TabKind::kWeb) {
      if (auto* wb = static_cast<WebTabBehavior*>(active->behavior()))
        bv = wb->browser_view();
    } else {
      if (auto* sb = static_cast<SimpleTabBehavior*>(active->behavior()))
        bv = sb->browser_view();
    }
    RoundContentCorners(main_window_, bv, current_chrome_.bg_body);
  }

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

void StylePopoverChrome(CefRefPtr<CefBrowserView> v) {
  if (!v) return;
  auto b = v->GetBrowser();
  if (!b) return;
  // No shadow on the chrome strip — shadow comes from the content view below.
  StyleOverlayBrowserView(b->GetHost()->GetWindowHandle(),
                          kPopoverCornerRadius,
                          kCornerTop,
                          /*with_shadow=*/false);
}

void StylePopoverContent(CefRefPtr<CefBrowserView> v, int corner_mask) {
  if (!v) return;
  auto b = v->GetBrowser();
  if (!b) return;
  StyleOverlayBrowserView(b->GetHost()->GetWindowHandle(),
                          kPopoverCornerRadius,
                          corner_mask,
                          /*with_shadow=*/true);
}
#else
inline void StylePopoverChrome(CefRefPtr<CefBrowserView>) {}
inline void StylePopoverContent(CefRefPtr<CefBrowserView>, int) {}
#endif

// Returns true for bundled panel URLs (file:// + /panels/ path). Builtin
// panels supply their own title/close bar so the URL-bar chrome strip is
// suppressed; only web-page popovers (non-file URLs) show the full chrome.
static bool IsBuiltinPanel(const std::string& url) {
  return url.find("file://") == 0 &&
         url.find("/panels/") != std::string::npos;
}

}  // namespace

void MainWindow::OpenPopover(const std::string& url, int owner_browser_id) {
  if (!main_window_) return;

  const bool is_builtin = IsBuiltinPanel(url);

  // If a popover already exists, just navigate it and re-pair owner.
  if (popover_overlay_ && popover_overlay_->IsValid()) {
    popover_owner_browser_id_ = owner_browser_id;
    popover_is_builtin_ = is_builtin;
    if (popover_view_) {
      auto b = popover_view_->GetBrowser();
      if (b) b->GetMainFrame()->LoadURL(url);
    }
    if (!is_builtin && popover_chrome_view_) {
      PushToView(popover_chrome_view_,
                 "popover_chrome.url_changed",
                 "{\"url\":\"" + JsEsc(url) + "\"}");
    }
    LayoutPopover();
    UpdatePopoverVisibility();
    if (popover_view_) popover_view_->RequestFocus();
    SetContentOuterVInsets(24, 24);
    return;
  }

  popover_owner_browser_id_ = owner_browser_id;
  popover_is_builtin_ = is_builtin;

  CefBrowserSettings bs;
  // Give the popover an opaque initial background so it doesn't flash
  // transparent while the page HTML hasn't rendered its body background.
  bs.background_color = current_chrome_.bg_float != 0
                            ? current_chrome_.bg_float
                            : static_cast<cef_color_t>(0xFF1C1C1F);
  popover_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, url, bs, nullptr, nullptr,
      new AlloyBrowserViewDelegate());
  popover_overlay_ = main_window_->AddOverlayView(
      popover_view_, CEF_DOCKING_MODE_CUSTOM, /*can_activate=*/true);

  // 2) Native chrome strip (URL textfield + action buttons) — overlay #2,
  //    added last so it sits above the content overlay.
  //    Suppressed for builtin panels: they render their own title bar.
  if (!is_builtin) {
    popover_chrome_view_ = BuildPopoverChromeView(url);
    popover_chrome_overlay_ = main_window_->AddOverlayView(
        popover_chrome_view_, CEF_DOCKING_MODE_CUSTOM, /*can_activate=*/true);
  }

  LayoutPopover();
  UpdatePopoverVisibility();
  if (popover_view_) popover_view_->RequestFocus();

  // Arc-style: visually recess the main content card when a popover is
  // in front. Equal top/bottom insets so the card is vertically centered.
  SetContentOuterVInsets(24, 24);
  const bool builtin_for_style = is_builtin;
  CefPostTask(TID_UI, base::BindOnce(
      [](CefRefPtr<CefBrowserView> content,
         CefRefPtr<CefBrowserView> chrome_view,
         bool builtin) {
        // Builtin panels fill the entire popover → all 4 corners rounded.
        // Web-page popovers have a chrome strip on top → only bottom corners
        // on the content view; the chrome strip gets the top corners.
        const int content_mask = builtin ? kCornerAll : kCornerBottom;
        StylePopoverContent(content, content_mask);
        if (!builtin && chrome_view) StylePopoverChrome(chrome_view);
      },
      popover_view_, popover_chrome_view_, builtin_for_style));
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
  popover_chrome_view_ = nullptr;
  popover_root_ = nullptr;
  popover_owner_browser_id_ = 0;
  popover_content_browser_id_ = 0;
  popover_chrome_browser_id_ = 0;
  popover_is_builtin_ = false;
  // Restore the normal content-panel insets now that the popover is gone.
  SetContentOuterVInsets(8, 8);
}

void MainWindow::UpdatePopoverVisibility() {
  if (!popover_overlay_ || !popover_overlay_->IsValid()) return;
  // refine-ui-theme-layout: a popover with `owner_browser_id == 0` is a
  // *global* popover (currently: Settings) that floats over whatever is
  // active. Per-tab popovers (e.g. address-bar suggest) only show while
  // their owning web tab is foreground.
  bool visible = false;
  if (popover_owner_browser_id_ == 0) {
    visible = true;
  } else {
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
  // Builtin panels supply their own title bar — no separate chrome strip.
  const int chrome_h = popover_is_builtin_ ? 0 : kChromeH;
  const int content_x = kSidebarW;
  const int content_y = kTitleBarH;
  const int content_w = std::max(320, bounds.width  - kSidebarW);
  const int content_h = std::max(360, bounds.height - kTitleBarH);
  // Arc-style popover sizing: match the full content-panel height so the
  // popup feels like it replaces the card rather than floating as a tiny
  // modal. Width stays at 80% centered. Leave an 8 px gap at the bottom
  // so the popover edge doesn't sit flush against the window frame.
  const int w = std::min(1280, std::max(560, content_w * 80 / 100));
  const int h = std::max(80, content_h - 8);
  const int x = content_x + (content_w - w) / 2;
  const int y = content_y;
  // Address bar on top (web-page popovers only), content below.
  if (!popover_is_builtin_ && popover_chrome_overlay_ &&
      popover_chrome_overlay_->IsValid()) {
    popover_chrome_overlay_->SetBounds(CefRect(x, y, w, kChromeH));
  }
  popover_overlay_->SetBounds(
      CefRect(x, y + chrome_h, w, std::max(80, h - chrome_h)));

  // Re-assert corner mask + shadow after CEF lays out / re-parents.
  const int content_mask = popover_is_builtin_ ? kCornerAll : kCornerBottom;
  StylePopoverContent(popover_view_, content_mask);
  if (!popover_is_builtin_ && popover_chrome_view_)
    StylePopoverChrome(popover_chrome_view_);
}

void MainWindow::OnWindowBoundsChanged(CefRefPtr<CefWindow> window,
                                       const CefRect& new_bounds) {
  (void)window; (void)new_bounds;
  LayoutPopover();
  RefreshTitleBarDragRegion();
}

// ---------------------------------------------------------------------------
// Popover chrome (HTML BrowserView)
// ---------------------------------------------------------------------------
// A CefPanel overlay does not paint SetBackgroundColor on macOS overlay
// NSViews — the NSView layer stays transparent regardless of the logical
// color. Using a CefBrowserView backed by an HTML page gives us a solid
// dark background via CSS and the same bridge mechanism used by every
// other panel in the app.

CefRefPtr<CefBrowserView> MainWindow::BuildPopoverChromeView(
    const std::string& initial_url) {
  CefBrowserSettings bs;
  // Send the initial URL to the HTML page once it finishes loading via the
  // popover_chrome.url_changed event (pushed from on_address_change when
  // the content browser fires its first OnAddressChange). We'll also push
  // it directly once the chrome browser is created.
  (void)initial_url;
  auto view = CefBrowserView::CreateBrowserView(
      client_handler_,
      ResourceUrl("panels/popover/index.html"),
      bs, nullptr, nullptr,
      new AlloyBrowserViewDelegate());
  return view;
}

// ---------------------------------------------------------------------------
// Native title bar (CefPanel)
// ---------------------------------------------------------------------------

namespace {
constexpr int kTitleBarH = 38;
// refine-ui-theme-layout: the chrome fill is sourced from
// MainWindow::current_chrome_.window_bg via ApplyThemeChrome().
// kTitleBarBg below is the fallback used during the very first paint
// before ApplyThemeChrome runs (matches the legacy Dark default).
constexpr cef_color_t kTitleBarBgFallback = 0xFF14141A;
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
  // refine-ui-theme-layout: use the resolved chrome if it has been
  // computed, otherwise fall back to the legacy dark constant. The
  // first ApplyThemeChrome() call (right after BuildChrome) overwrites
  // this with the persisted choice.
  panel->SetBackgroundColor(
      current_chrome_.bg_body == 0 ? kTitleBarBgFallback
                     : current_chrome_.bg_body);

  CefBoxLayoutSettings box;
  box.horizontal = true;
  box.inside_border_insets = {6, 8, 6, 8};
  box.between_child_spacing = 6;
  auto layout = panel->SetToBoxLayout(box);

  // 1. macOS traffic-light reservation.
  lights_pad_ = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(kLightsPadW, kTitleBarH - 12)));
  lights_pad_->SetBackgroundColor(
      current_chrome_.bg_body == 0 ? kTitleBarBgFallback
                     : current_chrome_.bg_body);
  panel->AddChildView(lights_pad_);
  layout->SetFlexForView(lights_pad_, 0);

  // 2. Drag spacer (drag overlay attaches here on macOS).
  spacer_ = CefPanel::CreatePanel(nullptr);
  spacer_->SetBackgroundColor(
      current_chrome_.bg_body == 0 ? kTitleBarBgFallback
                     : current_chrome_.bg_body);
  panel->AddChildView(spacer_);
  layout->SetFlexForView(spacer_, 1);

  // 3. New-tab buttons. (unified-icons: text label kept, glyph replaced
  // with a registry-backed CefImage via MakeIconLabelButton.)
  auto add_btn = [&](CefRefPtr<CefLabelButton>* slot, IconId icon,
                     const std::string& label, const std::string& tooltip,
                     const std::string& kind) {
    auto btn = MakeIconLabelButton(
        new FnButtonDelegate([this, kind]() {
          // Defer to a UI tick so click handler unwinds before any tab
          // mutation walks the view tree.
          CefPostTask(TID_UI, base::BindOnce(
              [](CefRefPtr<MainWindow> self, std::string k) {
                self->OpenNewTabKind(k);
              },
              CefRefPtr<MainWindow>(this), kind));
        }),
        icon, label, tooltip);
    btn->SetTextColor(CEF_BUTTON_STATE_NORMAL, kTitleBarBtnFg);
    btn->SetTextColor(CEF_BUTTON_STATE_HOVERED, 0xFFFFFFFF);
    btn->SetBackgroundColor(
        current_chrome_.bg_body == 0 ? kTitleBarBgFallback
                                     : current_chrome_.bg_body);
    panel->AddChildView(btn);
    layout->SetFlexForView(btn, 0);
    *slot = btn;
  };
  add_btn(&btn_web_,  IconId::kTabWeb,      "Web",      "New web tab",  "web");
  add_btn(&btn_term_, IconId::kTabTerminal, "Terminal", "New terminal", "terminal");
  add_btn(&btn_chat_, IconId::kTabChat,     "Chat",     "New chat",     "chat");

  // Settings: opens the Settings popover (refine-ui-theme-layout).
  // Replaces the legacy "activate Agent singleton" path so settings now
  // float over the active tab regardless of which tab is focused.
  // (unified-icons: glyph replaced with kSettings icon.)
  {
    auto btn = MakeIconLabelButton(
        new FnButtonDelegate([this]() {
          CefPostTask(TID_UI, base::BindOnce(
              [](CefRefPtr<MainWindow> self) {
                self->OpenPopover(
                    self->ResourceUrl("panels/settings/index.html"));
              },
              CefRefPtr<MainWindow>(this)));
        }),
        IconId::kSettings, "Settings", "Open settings");
    btn->SetTextColor(CEF_BUTTON_STATE_NORMAL, kTitleBarBtnFg);
    btn->SetTextColor(CEF_BUTTON_STATE_HOVERED, 0xFFFFFFFF);
    btn->SetBackgroundColor(
        current_chrome_.bg_body == 0 ? kTitleBarBgFallback
                                     : current_chrome_.bg_body);
    btn->SetTooltipText("Open settings");
    panel->AddChildView(btn);
    layout->SetFlexForView(btn, 0);
    btn_settings_ = btn;
  }

  // 4. Reserved Windows-controls slot (zero width on macOS).
  win_pad_ = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(kWinPadW, 1)));
  win_pad_->SetBackgroundColor(
      current_chrome_.bg_body == 0 ? kTitleBarBgFallback
                     : current_chrome_.bg_body);
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
    if (Tab* tab = tabs_->Get(id)) {
      tab->ApplyTheme(current_chrome_.bg_base, current_chrome_.bg_float,
                      current_chrome_.text_title);
    }
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
  // Traffic-light reservation: exclude the entire lights_pad area so
  // macOS close/minimize/zoom buttons receive mouse events (the drag
  // overlay sits above them in the themeFrame z-order and would otherwise
  // swallow those clicks).
  if (lights_pad_) {
    CefRect lr = lights_pad_->GetBoundsInScreen();
    if (lr.width > 0 && lr.height > 0) {
      nodrag.emplace_back(lr.x - win.x, lr.y - win.y, lr.width, lr.height);
    }
  }
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

// ---------------------------------------------------------------------------
// refine-ui-theme-layout: theme application + persistence
// ---------------------------------------------------------------------------

// static
MainWindow::ThemeChrome MainWindow::ChromeFor(const std::string& resolved) {
  // Tokens mirror the shell-relevant semantic subset in theme.css.
  // ARGB encodes 0xFF<rrggbb> for opaque colors.
  ThemeChrome c{};
  if (resolved == "light") {
    c.bg_body      = 0xFFF3F7F6;  // bg_body
    c.bg_base      = 0xFFFCFEFD;  // bg_base
    c.bg_float     = 0xFFFFFFFF;  // bg_float
    c.bg_mask      = 0x290E1817;  // bg_mask rgba(14, 24, 23, 0.16)
    c.border       = 0xFFD5E2DE;  // border
    c.text_title   = 0xFF13201E;  // text_title
    c.text_caption = 0xFF5A6E69;  // text_caption
  } else {
    c.bg_body      = 0xFF0E1716;  // bg_body
    c.bg_base      = 0xFF131F1D;  // bg_base
    c.bg_float     = 0xFF182625;  // bg_float
    c.bg_mask      = 0x85020808;  // bg_mask rgba(2, 8, 8, 0.52)
    c.border       = 0xFF29403D;  // border
    c.text_title   = 0xFFE8F2F0;  // text_title
    c.text_caption = 0xFF9DB2AD;  // text_caption
  }
  return c;
}

std::string MainWindow::ResolveAppearance() const {
  if (theme_mode_ == "light") return "light";
  if (theme_mode_ == "dark")  return "dark";
#if defined(__APPLE__)
  return CurrentSystemAppearance();
#else
  return "dark";
#endif
}

namespace {
std::string ArgbToCssHex(cef_color_t argb) {
  char buf[8];
  std::snprintf(buf, sizeof(buf), "#%06X", static_cast<unsigned>(argb & 0x00FFFFFF));
  return std::string(buf);
}
}  // namespace

std::string MainWindow::ThemeStateJson(bool include_chrome) const {
  std::string resolved = ResolveAppearance();
  std::string out = "{\"mode\":\"";
  out += theme_mode_;
  out += "\",\"resolved\":\"";
  out += resolved;
  out += "\"";
  if (include_chrome) {
    out += ",\"chrome\":{\"bg_body\":\"";
    out += ArgbToCssHex(current_chrome_.bg_body);
    out += "\",\"bg_base\":\"";
    out += ArgbToCssHex(current_chrome_.bg_base);
    out += "\",\"bg_float\":\"";
    out += ArgbToCssHex(current_chrome_.bg_float);
    out += "\",\"bg_mask\":\"";
    out += ArgbToCssHex(current_chrome_.bg_mask);
    out += "\",\"border\":\"";
    out += ArgbToCssHex(current_chrome_.border);
    out += "\",\"text_title\":\"";
    out += ArgbToCssHex(current_chrome_.text_title);
    out += "\",\"text_caption\":\"";
    out += ArgbToCssHex(current_chrome_.text_caption);
    out += "\"}";
  }
  out += "}";
  return out;
}

void MainWindow::ApplyThemeChrome(const ThemeChrome& chrome) {
  current_chrome_ = chrome;
  // Native panels: titlebar + body share the chrome fill so the visual
  // chrome is one continuous region (per refine-ui-theme-layout design).
  if (titlebar_panel_) titlebar_panel_->SetBackgroundColor(chrome.bg_body);
  if (body_panel_)     body_panel_->SetBackgroundColor(chrome.bg_body);
  if (content_outer_)  content_outer_->SetBackgroundColor(chrome.bg_body);
  if (content_frame_)  content_frame_->SetBackgroundColor(chrome.bg_base);
  // Titlebar action buttons must use a readable foreground against bg_body.
  // dark_mode = true when text is light (dark background), false otherwise.
  const bool title_dark = ((chrome.text_title >> 8) & 0xFF) > 0x80;
  constexpr IconId kTitleBtnIcons[] = {
      IconId::kTabWeb, IconId::kTabTerminal, IconId::kTabChat, IconId::kSettings};
  CefRefPtr<CefLabelButton>* kTitleBtns[] = {
      &btn_web_, &btn_term_, &btn_chat_, &btn_settings_};
  for (int i = 0; i < 4; ++i) {
    auto* b = kTitleBtns[i]->get();
    if (!b) continue;
    b->SetTextColor(CEF_BUTTON_STATE_NORMAL,  chrome.text_title);
    b->SetTextColor(CEF_BUTTON_STATE_HOVERED, chrome.text_title);
    b->SetBackgroundColor(chrome.bg_body);
    IconRegistry::ApplyToButton(*kTitleBtns[i], kTitleBtnIcons[i], title_dark);
  }
  if (tabs_) {
    for (const auto& summary : tabs_->Snapshot()) {
      if (Tab* tab = tabs_->Get(summary.id)) {
        tab->ApplyTheme(chrome.bg_base, chrome.bg_float, chrome.text_title);
      }
    }
  }
  // Title-bar child panels also need the fill — they paint with their own
  // bg color, so without this they'd show CefPanel's default (white) and
  // visually break the title bar into bands.
  if (lights_pad_) lights_pad_->SetBackgroundColor(chrome.bg_body);
  if (spacer_)     spacer_->SetBackgroundColor(chrome.bg_body);
  if (win_pad_)    win_pad_->SetBackgroundColor(chrome.bg_body);
#if defined(__APPLE__)
  if (main_window_) {
    SetMainWindowBackgroundColor(main_window_->GetWindowHandle(),
                                 chrome.bg_body);
  }
  // Refresh the AppKit corner-punch views for the active tab so they match
  // the new bg_body. Without this re-call, old punch views keep the previous
  // theme's color and create visually wrong corners on theme switch.
  if (tabs_) {
    Tab* active = tabs_->Active();
    if (active) {
      CefRefPtr<CefBrowserView> bv;
      if (active->kind() == TabKind::kWeb) {
        if (auto* wb = static_cast<WebTabBehavior*>(active->behavior()))
          bv = wb->browser_view();
      } else {
        if (auto* sb = static_cast<SimpleTabBehavior*>(active->behavior()))
          bv = sb->browser_view();
      }
      RoundContentCorners(main_window_, bv, chrome.bg_body);
    }
  }
#endif
  // Broadcast to renderers so each panel's installThemeMirror flips the
  // <html data-theme="…"> attribute and React listeners refresh.
  BroadcastToAllPanels("theme.changed", ThemeStateJson(/*include_chrome=*/true));
}

void MainWindow::HandleThemeModeChange(const std::string& mode) {
  if (mode != "system" && mode != "light" && mode != "dark") return;
  theme_mode_ = mode;
  space_manager_.store().SetKv("ui.theme", mode);
  ApplyThemeChrome(ChromeFor(ResolveAppearance()));
}

void MainWindow::OnSystemAppearanceChanged() {
  // Only react in `system` mode; explicit Light/Dark pins ignore the OS.
  if (theme_mode_ != "system") return;
  ApplyThemeChrome(ChromeFor(ResolveAppearance()));
}

void MainWindow::SetContentOuterVInsets(int top, int bottom) {
  if (!content_outer_ || !content_frame_) return;
  CefBoxLayoutSettings box;
  box.horizontal = false;
  box.inside_border_insets = {top, 8, bottom, 8};
  auto layout = content_outer_->SetToBoxLayout(box);
  layout->SetFlexForView(content_frame_, 1);
  content_outer_->Layout();
  // The card has moved — refresh the corner-punch views so the rounded
  // corners track the new card position (same pattern as ApplyThemeChrome).
#if defined(__APPLE__)
  if (tabs_ && main_window_) {
    Tab* active = tabs_->Active();
    if (active) {
      CefRefPtr<CefBrowserView> bv;
      if (active->kind() == TabKind::kWeb) {
        if (auto* wb = static_cast<WebTabBehavior*>(active->behavior()))
          bv = wb->browser_view();
      } else {
        if (auto* sb = static_cast<SimpleTabBehavior*>(active->behavior()))
          bv = sb->browser_view();
      }
      RoundContentCorners(main_window_, bv, current_chrome_.bg_body);
    }
  }
#endif
}

}  // namespace cronymax
