// Copyright (c) 2026.

#include "browser/tab/web_tab_behavior.h"

#include <optional>
#include <utility>

#include "browser/client_handler.h"
#include "browser/icon_registry.h"
#include "browser/models/view_observer.h"
#include "browser/toolbar/tab_toolbar.h"
#include "include/base/cef_callback.h"
#include "include/cef_browser.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/wrapper/cef_closure_task.h"

namespace cronymax {
namespace {

// Forces Alloy runtime style — required when this BrowserView lives in a
// window that already hosts other Alloy browsers (every cronymax window).
class WebTabBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  WebTabBrowserViewDelegate() = default;
  cef_runtime_style_t GetBrowserRuntimeStyle() override {
    return CEF_RUNTIME_STYLE_ALLOY;
  }

 private:
  IMPLEMENT_REFCOUNTING(WebTabBrowserViewDelegate);
  DISALLOW_COPY_AND_ASSIGN(WebTabBrowserViewDelegate);
};

// Virtual key codes (Chromium VKEY_* mapping; same on all platforms).
constexpr int kVkReturn = 0x0D;
constexpr int kVkEscape = 0x1B;

}  // namespace

WebTabBehavior::WebTabBehavior(ClientHandler* client_handler,
                               std::string initial_url,
                               ThemeContext* theme_ctx)
    : theme_ctx_(theme_ctx),
      client_handler_(client_handler),
      initial_url_(std::move(initial_url)),
      current_url_(initial_url_),
      weak_factory_(this) {}

WebTabBehavior::~WebTabBehavior() {
  if (client_handler_ && browser_id_ != 0) {
    client_handler_->UnregisterBrowserListener(browser_id_);
  }
}

void WebTabBehavior::BuildToolbar(TabToolbar* toolbar,
                                  TabContext* /*context*/) {
  toolbar_ = toolbar;

  // Leading: back / forward / refresh.
  h_back_ = toolbar->AddLeadingAction(IconId::kBack, "Back", [this]() {
    if (browser_view_ && browser_view_->GetBrowser())
      browser_view_->GetBrowser()->GoBack();
  });
  toolbar->SetActionEnabled(h_back_, false);

  h_fwd_ = toolbar->AddLeadingAction(IconId::kForward, "Forward", [this]() {
    if (browser_view_ && browser_view_->GetBrowser())
      browser_view_->GetBrowser()->GoForward();
  });
  toolbar->SetActionEnabled(h_fwd_, false);

  h_refresh_ = toolbar->AddLeadingAction(IconId::kRefresh, "Refresh", [this]() {
    auto br = browser_view_ ? browser_view_->GetBrowser() : nullptr;
    if (!br)
      return;
    if (is_loading_)
      br->StopLoad();
    else
      br->Reload();
  });

  // Trailing: new-tab placeholder.
  h_new_tab_ = toolbar->AddTrailingAction(IconId::kNewTab, "New Tab", [this]() {
    if (browser_view_ && browser_view_->GetBrowser())
      browser_view_->GetBrowser()->GetMainFrame()->LoadURL("about:blank");
  });

  // Textfield event callbacks wired after Build (which created url_field_).
  toolbar->SetKeyCallback([this](int vk) -> bool {
    OnUrlFieldKeyEvent(vk);
    return false;
  });
  toolbar->SetFocusCallback([this]() { OnUrlFieldFocused(); });

  // Seed the URL field with the initial URL.
  toolbar->SetUrl(current_url_);
}

CefRefPtr<CefView> WebTabBehavior::BuildContent(TabContext* context) {
  context_ = context;
  CefBrowserSettings settings;
  if (theme_ctx_)
    settings.background_color = theme_ctx_->GetCurrentChrome().bg_body;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, initial_url_, settings, nullptr, nullptr,
      new WebTabBrowserViewDelegate());

  // Hook a per-browser listener once the browser is realized. We can't get
  // a stable browser_id_ until OnAfterCreated has fired, so poll lazily on
  // the first navigation/title event by reading from GetBrowser(). To avoid
  // a deferred-registration dance, we register at the next idle task: by
  // then OnAfterCreated has fired and GetBrowser()->GetIdentifier() is valid.
  CefPostTask(TID_UI, base::BindOnce(&WebTabBehavior::RegisterBrowserListener,
                                     weak_factory_.GetWeakPtr()));

  if (theme_ctx_)
    Register(theme_ctx_);
  return browser_view_;
}

void WebTabBehavior::RegisterBrowserListener() {
  if (!browser_view_)
    return;
  auto br = browser_view_->GetBrowser();
  if (!br)
    return;
  browser_id_ = br->GetIdentifier();
  if (!client_handler_)
    return;
  ClientHandler::BrowserListener listener;
  listener.on_address_change = [this](const std::string& u) {
    OnAddressChange(u);
  };
  listener.on_title_change = [this](const std::string& t) { OnTitleChange(t); };
  listener.on_loading_state_change = [this](bool il, bool cb, bool cf) {
    OnLoadingStateChange(il, cb, cf);
  };
  listener.on_load_end = [this](const std::string& url) { OnLoadEnd(url); };
  client_handler_->RegisterBrowserListener(browser_id_, std::move(listener));
}

void WebTabBehavior::ApplyToolbarState(const ToolbarState& /*state*/) {
  // Web tabs author their own toolbar state from native browser events; no
  // renderer push is expected on this channel for web kind. (The schema
  // permits it for symmetry; we just no-op.)
}

void WebTabBehavior::ApplyTheme(const ThemeChrome& /*chrome*/) {
  // TabToolbar is a ThemeAwareView; it subscribes to the ThemeContext directly
  // via ToolbarBase::Build → Register(ctx). No manual propagation needed here.
  // SetChromeColor overrides still flow via Tab →
  // WebTabBehavior::SetChromeColor which calls toolbar_->SetChromeColor.
}

void WebTabBehavior::FocusUrlField() {
  if (toolbar_)
    toolbar_->FocusUrlField();
}

void WebTabBehavior::Navigate(const std::string& url) {
  std::string final_url = url;
  if (final_url.find("://") == std::string::npos) {
    final_url = "https://" + final_url;
  }
  current_url_ = final_url;
  if (toolbar_)
    toolbar_->SetUrl(final_url);
  if (browser_view_ && browser_view_->GetBrowser()) {
    browser_view_->GetBrowser()->GetMainFrame()->LoadURL(final_url);
  }
}

void WebTabBehavior::GoBack() {
  if (browser_view_ && browser_view_->GetBrowser()) {
    browser_view_->GetBrowser()->GoBack();
  }
}

void WebTabBehavior::GoForward() {
  if (browser_view_ && browser_view_->GetBrowser()) {
    browser_view_->GetBrowser()->GoForward();
  }
}

void WebTabBehavior::Reload() {
  if (browser_view_ && browser_view_->GetBrowser()) {
    browser_view_->GetBrowser()->Reload();
  }
}

void WebTabBehavior::OnAddressChange(const std::string& url) {
  current_url_ = url;
  if (toolbar_)
    toolbar_->SetUrl(url);
}

void WebTabBehavior::OnTitleChange(const std::string& title) {
  current_title_ = title;
}

void WebTabBehavior::OnLoadingStateChange(bool is_loading,
                                          bool can_go_back,
                                          bool can_go_forward) {
  is_loading_ = is_loading;
  can_go_back_ = can_go_back;
  can_go_forward_ = can_go_forward;
  if (toolbar_) {
    toolbar_->SetActionEnabled(h_back_, can_go_back);
    toolbar_->SetActionEnabled(h_fwd_, can_go_forward);
  }
  UpdateRefreshStopGlyph();
}

void WebTabBehavior::OnLoadEnd(const std::string& url) {
  // Skip in-app panels (file:// URLs) and blank pages — they set their own
  // chrome theme via the tab.set_chrome_theme bridge.
  if (url.rfind("file://", 0) == 0)
    return;
  if (url.rfind("about:", 0) == 0)
    return;
  if (!browser_view_)
    return;
  auto br = browser_view_->GetBrowser();
  if (!br)
    return;
  auto frame = br->GetMainFrame();
  if (!frame)
    return;
  const std::string tab_id = context_ ? context_->tab_id() : std::string();
  if (tab_id.empty())
    return;

  // Inject JS that detects the page's background color from either the
  // <meta name="theme-color"> tag or the computed body background, then
  // reports it back via cefQuery → tab.set_chrome_theme so the toolbar tint
  // automatically matches the loaded page.
  // Using a raw string literal to keep the JS readable.
  const std::string js_template = R"JS(
(function(){
  var c='';
  try{
    var m=document.querySelector('meta[name="theme-color"]');
    if(m&&m.content)c=m.content.trim();
  }catch(e){}
  if(!c){
    try{
      var b=window.getComputedStyle(document.documentElement).backgroundColor;
      if(!b||b==='rgba(0, 0, 0, 0)'||b==='transparent')
        b=window.getComputedStyle(document.body).backgroundColor;
      var r=b.match(/rgb[a]?\((\d+),\s*(\d+),\s*(\d+)/);
      if(r)c='#'+('0'+parseInt(r[1]).toString(16)).slice(-2)
                +('0'+parseInt(r[2]).toString(16)).slice(-2)
                +('0'+parseInt(r[3]).toString(16)).slice(-2);
    }catch(e){}
  }
  if(c&&c.charCodeAt(0)===35&&window.cefQuery){
    window.cefQuery({
      request:'tab.set_chrome_theme\n{"tabId":"__TAB_ID__","color":"'+c+'"}',
      onSuccess:function(){},
      onFailure:function(){}
    });
  }
})();
)JS";

  std::string js = js_template;
  const std::string kPh = "__TAB_ID__";
  const auto pos = js.find(kPh);
  if (pos != std::string::npos)
    js.replace(pos, kPh.size(), tab_id);
  frame->ExecuteJavaScript(js, frame->GetURL(), 0);
}

void WebTabBehavior::OnUrlFieldKeyEvent(int windows_key_code) {
  if (windows_key_code == kVkReturn) {
    NavigateToCurrentField();
  } else if (windows_key_code == kVkEscape) {
    if (toolbar_)
      toolbar_->SetUrl(current_url_);
  }
}

void WebTabBehavior::OnUrlFieldFocused() {
  // Best-effort: select-all on focus. OnAfterUserAction fires on every key
  // press too, so guard so we don't re-select while typing.
  if (toolbar_ && toolbar_->GetUrl() == current_url_) {
    toolbar_->FocusUrlField();
  }
}

void WebTabBehavior::NavigateToCurrentField() {
  if (!toolbar_)
    return;
  std::string typed = toolbar_->GetUrl();
  if (typed.empty())
    return;
  if (typed.find("://") == std::string::npos)
    typed = "https://" + typed;
  if (browser_view_ && browser_view_->GetBrowser()) {
    browser_view_->GetBrowser()->GetMainFrame()->LoadURL(typed);
  }
  current_url_ = typed;
}

void WebTabBehavior::UpdateRefreshStopGlyph() {
  if (toolbar_ && h_refresh_ != ToolbarBase::kInvalidHandle) {
    toolbar_->UpdateActionIcon(h_refresh_,
                               is_loading_ ? IconId::kStop : IconId::kRefresh);
  }
}

}  // namespace cronymax
