// Copyright (c) 2026.

#include "browser/tab/web_tab_behavior.h"

#include <utility>

#include "browser/client_handler.h"
#include "browser/icon_registry.h"
#include "browser/tab/tab_toolbar.h"
#include "include/base/cef_callback.h"
#include "include/cef_browser.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/views/cef_button_delegate.h"
#include "include/views/cef_textfield_delegate.h"
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

// Refcounted CefButtonDelegate that forwards OnButtonPressed to a
// std::function. Used so WebTabBehavior (which is unique_ptr-owned) can
// participate in the CefRefPtr-based delegate world.
class FunctionButtonDelegate : public CefButtonDelegate {
 public:
  explicit FunctionButtonDelegate(std::function<void()> on_press)
      : on_press_(std::move(on_press)) {}
  void OnButtonPressed(CefRefPtr<CefButton> /*button*/) override {
    if (on_press_) on_press_();
  }

 private:
  std::function<void()> on_press_;
  IMPLEMENT_REFCOUNTING(FunctionButtonDelegate);
  DISALLOW_COPY_AND_ASSIGN(FunctionButtonDelegate);
};

class FunctionTextfieldDelegate : public CefTextfieldDelegate {
 public:
  FunctionTextfieldDelegate(std::function<bool(int)> on_key,
                             std::function<void()> on_focus_change)
      : on_key_(std::move(on_key)),
        on_focus_change_(std::move(on_focus_change)) {}
  bool OnKeyEvent(CefRefPtr<CefTextfield> /*textfield*/,
                   const CefKeyEvent& event) override {
    // Only react on key-down so Enter/Escape fire once.
    if (event.type != KEYEVENT_RAWKEYDOWN) return false;
    if (on_key_) return on_key_(event.windows_key_code);
    return false;
  }
  // Best-effort focus-on-click signal: OnAfterUserAction fires on every user
  // action (click/typing). The behavior debounces with a "last selected URL"
  // check so we don't re-select on every keystroke.
  void OnAfterUserAction(CefRefPtr<CefTextfield> /*textfield*/) override {
    if (on_focus_change_) on_focus_change_();
  }

 private:
  std::function<bool(int)> on_key_;
  std::function<void()> on_focus_change_;
  IMPLEMENT_REFCOUNTING(FunctionTextfieldDelegate);
  DISALLOW_COPY_AND_ASSIGN(FunctionTextfieldDelegate);
};

// Default chrome ARGB / pill colors. Mirror tab_toolbar.cc defaults.
constexpr cef_color_t kPillBg = 0xFF1A1A1F;
constexpr cef_color_t kPillFg = 0xFFE6E6EA;
constexpr cef_color_t kBtnFg  = 0xFFE6E6EA;

// Virtual key codes (Chromium VKEY_* mapping; same on all platforms).
constexpr int kVkReturn = 0x0D;
constexpr int kVkEscape = 0x1B;

}  // namespace

WebTabBehavior::WebTabBehavior(ClientHandler* client_handler,
                                 std::string initial_url)
    : client_handler_(client_handler),
      initial_url_(std::move(initial_url)),
      current_url_(initial_url_) {}

WebTabBehavior::~WebTabBehavior() {
  if (client_handler_ && browser_id_ != 0) {
    client_handler_->UnregisterBrowserListener(browser_id_);
  }
}

void WebTabBehavior::BuildToolbar(TabToolbar* toolbar, TabContext* /*context*/) {
  // Leading: back / forward / refresh. (unified-icons: registry-backed
  // CefImage replaces the previous Unicode glyph text.)
  back_btn_ = MakeIconButton(
      new FunctionButtonDelegate([this]() {
        if (browser_view_ && browser_view_->GetBrowser()) {
          browser_view_->GetBrowser()->GoBack();
        }
      }),
      IconId::kBack, "Back");
  back_btn_->SetEnabled(false);
  back_btn_->SetTextColor(CEF_BUTTON_STATE_NORMAL, kBtnFg);
  back_btn_->SetBackgroundColor(0);
  toolbar->leading()->AddChildView(back_btn_);

  fwd_btn_ = MakeIconButton(
      new FunctionButtonDelegate([this]() {
        if (browser_view_ && browser_view_->GetBrowser()) {
          browser_view_->GetBrowser()->GoForward();
        }
      }),
      IconId::kForward, "Forward");
  fwd_btn_->SetEnabled(false);
  fwd_btn_->SetTextColor(CEF_BUTTON_STATE_NORMAL, kBtnFg);
  fwd_btn_->SetBackgroundColor(0);
  toolbar->leading()->AddChildView(fwd_btn_);

  refresh_btn_ = MakeIconButton(
      new FunctionButtonDelegate([this]() {
        auto br = browser_view_ ? browser_view_->GetBrowser() : nullptr;
        if (!br) return;
        if (is_loading_) {
          br->StopLoad();
        } else {
          br->Reload();
        }
      }),
      IconId::kRefresh, "Refresh");
  refresh_btn_->SetTextColor(CEF_BUTTON_STATE_NORMAL, kBtnFg);
  refresh_btn_->SetBackgroundColor(0);
  toolbar->leading()->AddChildView(refresh_btn_);

  // Middle: URL pill.
  url_field_ = CefTextfield::CreateTextfield(
      new FunctionTextfieldDelegate(
          [this](int vk) -> bool { OnUrlFieldKeyEvent(vk); return false; },
          [this]() { OnUrlFieldFocused(); }));
  url_field_->SetText(current_url_);
  url_field_->SetBackgroundColor(kPillBg);
  url_field_->SetTextColor(kPillFg);
  // Explicit preferred height keeps the pill compact; the toolbar root uses
  // CEF_AXIS_ALIGNMENT_CENTER so the field is vertically centred at this size
  // rather than being stretched to the full toolbar inner height.
  toolbar->middle()->AddChildView(url_field_);
  // Make the URL textfield consume all available middle-slot width.
  if (auto middle_layout = toolbar->middle()->GetLayout()) {
    if (auto box = middle_layout->AsBoxLayout()) {
      box->SetFlexForView(url_field_, 1);
    }
  }

  // Trailing: new-tab placeholder. Phase 10 wires the dock button properly;
  // this is a click target so the toolbar feels live in Phase 3 smoke-tests.
  new_btn_ = MakeIconButton(
      new FunctionButtonDelegate([this]() {
        if (browser_view_ && browser_view_->GetBrowser()) {
          browser_view_->GetBrowser()->GetMainFrame()->LoadURL("about:blank");
        }
      }),
      IconId::kNewTab, "New Tab");
  new_btn_->SetTextColor(CEF_BUTTON_STATE_NORMAL, kBtnFg);
  new_btn_->SetBackgroundColor(0);
  toolbar->trailing()->AddChildView(new_btn_);
}

CefRefPtr<CefView> WebTabBehavior::BuildContent(TabContext* context) {
  context_ = context;
  CefBrowserSettings settings;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, initial_url_, settings, nullptr, nullptr,
      new WebTabBrowserViewDelegate());

  // Hook a per-browser listener once the browser is realized. We can't get
  // a stable browser_id_ until OnAfterCreated has fired, so poll lazily on
  // the first navigation/title event by reading from GetBrowser(). To avoid
  // a deferred-registration dance, we register at the next idle task: by
  // then OnAfterCreated has fired and GetBrowser()->GetIdentifier() is valid.
  CefPostTask(TID_UI, base::BindOnce([](WebTabBehavior* self) {
                if (!self->browser_view_) return;
                auto br = self->browser_view_->GetBrowser();
                if (!br) return;
                self->browser_id_ = br->GetIdentifier();
                if (!self->client_handler_) return;
                ClientHandler::BrowserListener listener;
                listener.on_address_change = [self](const std::string& u) {
                  self->OnAddressChange(u);
                };
                listener.on_title_change = [self](const std::string& t) {
                  self->OnTitleChange(t);
                };
                listener.on_loading_state_change =
                    [self](bool il, bool cb, bool cf) {
                      self->OnLoadingStateChange(il, cb, cf);
                    };
                listener.on_load_end = [self](const std::string& url) {
                  self->OnLoadEnd(url);
                };
                self->client_handler_->RegisterBrowserListener(
                    self->browser_id_, std::move(listener));
              }, this));

  return browser_view_;
}

void WebTabBehavior::ApplyToolbarState(const ToolbarState& /*state*/) {
  // Web tabs author their own toolbar state from native browser events; no
  // renderer push is expected on this channel for web kind. (The schema
  // permits it for symmetry; we just no-op.)
}

void WebTabBehavior::ApplyThemeColors(cef_color_t text_fg,
                                      cef_color_t surface_bg,
                                      cef_color_t toolbar_bg) {
  // dark_mode = true when text is light (dark background), false otherwise.
  const bool dark = ((text_fg >> 8) & 0xFF) > 0x80;
  current_dark_mode_ = dark;
  // Update every toolbar widget built in BuildToolbar so they use the
  // current theme's foreground / surface colors instead of hardcoded values.
  const struct { CefRefPtr<CefLabelButton>* btn; IconId id; } kFixedBtns[] = {
    {&back_btn_,    IconId::kBack},
    {&fwd_btn_,     IconId::kForward},
    {&new_btn_,     IconId::kNewTab},
  };
  for (const auto& e : kFixedBtns) {
    if (!e.btn->get()) continue;
    e.btn->get()->SetTextColor(CEF_BUTTON_STATE_NORMAL,  text_fg);
    e.btn->get()->SetTextColor(CEF_BUTTON_STATE_HOVERED, text_fg);
    if (toolbar_bg != 0) e.btn->get()->SetBackgroundColor(toolbar_bg);
    IconRegistry::ApplyToButton(*e.btn, e.id, dark);
  }
  // refresh_btn_ alternates between kRefresh and kStop depending on load state.
  if (refresh_btn_) {
    refresh_btn_->SetTextColor(CEF_BUTTON_STATE_NORMAL,  text_fg);
    refresh_btn_->SetTextColor(CEF_BUTTON_STATE_HOVERED, text_fg);
    if (toolbar_bg != 0) refresh_btn_->SetBackgroundColor(toolbar_bg);
    IconRegistry::ApplyToButton(
        refresh_btn_, is_loading_ ? IconId::kStop : IconId::kRefresh, dark);
  }
  if (url_field_) {
    url_field_->SetBackgroundColor(surface_bg);
    url_field_->SetTextColor(text_fg);
  }
}

void WebTabBehavior::FocusUrlField() {
  if (url_field_) {
    url_field_->RequestFocus();
    url_field_->SelectAll(/*reversed=*/false);
  }
}

void WebTabBehavior::Navigate(const std::string& url) {
  std::string final_url = url;
  if (final_url.find("://") == std::string::npos) {
    final_url = "https://" + final_url;
  }
  current_url_ = final_url;
  if (url_field_) url_field_->SetText(final_url);
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
  if (url_field_) url_field_->SetText(url);
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
  if (back_btn_) back_btn_->SetEnabled(can_go_back);
  if (fwd_btn_) fwd_btn_->SetEnabled(can_go_forward);
  UpdateRefreshStopGlyph();
}

void WebTabBehavior::OnLoadEnd(const std::string& url) {
  // Skip in-app panels (file:// URLs) and blank pages — they set their own
  // chrome theme via the tab.set_chrome_theme bridge.
  if (url.rfind("file://", 0) == 0) return;
  if (url.rfind("about:", 0) == 0) return;
  if (!browser_view_) return;
  auto br = browser_view_->GetBrowser();
  if (!br) return;
  auto frame = br->GetMainFrame();
  if (!frame) return;
  const std::string tab_id = context_ ? context_->tab_id() : std::string();
  if (tab_id.empty()) return;

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
  if (pos != std::string::npos) js.replace(pos, kPh.size(), tab_id);
  frame->ExecuteJavaScript(js, frame->GetURL(), 0);
}

void WebTabBehavior::OnUrlFieldKeyEvent(int windows_key_code) {
  if (windows_key_code == kVkReturn) {
    NavigateToCurrentField();
  } else if (windows_key_code == kVkEscape) {
    if (url_field_) url_field_->SetText(current_url_);
  }
}

void WebTabBehavior::OnUrlFieldFocused() {
  // Best-effort: select-all on focus. OnAfterUserAction fires on every key
  // press too, so guard with HasSelection so we don't re-select while
  // typing.
  if (url_field_ && !url_field_->HasSelection() &&
      url_field_->GetText() == current_url_) {
    url_field_->SelectAll(/*reversed=*/false);
  }
}

void WebTabBehavior::NavigateToCurrentField() {
  if (!url_field_) return;
  std::string typed = url_field_->GetText().ToString();
  if (typed.empty()) return;
  if (typed.find("://") == std::string::npos) typed = "https://" + typed;
  if (browser_view_ && browser_view_->GetBrowser()) {
    browser_view_->GetBrowser()->GetMainFrame()->LoadURL(typed);
  }
  current_url_ = typed;
}

void WebTabBehavior::UpdateRefreshStopGlyph() {
  if (!refresh_btn_) return;
  // unified-icons: swap the registry image instead of the text glyph.
  // Use current_dark_mode_ so the tint matches the active theme even when
  // this fires from OnLoadingStateChange (outside of ApplyThemeColors).
  IconRegistry::ApplyToButton(
      refresh_btn_, is_loading_ ? IconId::kStop : IconId::kRefresh,
      current_dark_mode_);
  refresh_btn_->SetAccessibleName(is_loading_ ? "Stop" : "Refresh");
  refresh_btn_->SetTooltipText(is_loading_ ? "Stop" : "Refresh");
}

}  // namespace cronymax
