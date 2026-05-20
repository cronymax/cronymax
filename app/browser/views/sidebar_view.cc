// app/browser/views/sidebar_view.cc
//
// native-views-mvc Phase 10: SidebarView implementation.
//
// The sidebar column is a vertical CefPanel split into two parts:
//
//   [top]    CefBrowserView  — flex 1 — hosts panels/sidebar/index.html
//   [bottom] CEF-views panel — flex 0 — Activities + Flows action buttons

#include "browser/views/sidebar_view.h"

#include "browser/client_handler.h"
#include "browser/icon_registry.h"
#include "browser/models/resource_context.h"
#include "browser/models/view_context.h"
#include "browser/models/view_observer.h"
#include "browser/views/view_helpers.h"
#include "include/base/cef_callback.h"
#include "include/cef_task.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {

namespace {

constexpr int kSidebarW = 240;
// Height of the bottom CEF-views panel (two rows of icon buttons).
constexpr int kCefPanelH = 80;

// Fixed-width delegate for the sidebar browser view.
class SidebarBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  SidebarBrowserViewDelegate() = default;
  CefSize GetPreferredSize(CefRefPtr<CefView> /*view*/) override {
    return CefSize(kSidebarW, 900);
  }
  cef_runtime_style_t GetBrowserRuntimeStyle() override {
    return CEF_RUNTIME_STYLE_ALLOY;
  }

 private:
  IMPLEMENT_REFCOUNTING(SidebarBrowserViewDelegate);
  DISALLOW_COPY_AND_ASSIGN(SidebarBrowserViewDelegate);
};

}  // namespace

SidebarView::SidebarView(ResourceContext* resource_ctx,
                         ThemeContext* theme_ctx,
                         CefRefPtr<ClientHandler> client_handler,
                         Host host)
    : resource_ctx_(resource_ctx),
      theme_ctx_(theme_ctx),
      client_handler_(std::move(client_handler)),
      host_(std::move(host)) {}

SidebarView::~SidebarView() = default;

CefRefPtr<CefPanel> SidebarView::Build() {
  const ThemeChrome chrome =
      theme_ctx_ ? theme_ctx_->GetCurrentChrome() : ThemeChrome{};

  // ── Root column (VBox) ────────────────────────────────────────────────────
  column_panel_ =
      CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(kSidebarW, 0)));
  column_panel_->SetBackgroundColor(chrome.bg_body);

  CefBoxLayoutSettings col_box;
  col_box.horizontal = false;
  col_box.between_child_spacing = 0;
  auto col_layout = column_panel_->SetToBoxLayout(col_box);

  // ── Top: sidebar webview (flex 1) ─────────────────────────────────────────
  CefBrowserSettings settings;
  settings.background_color = chrome.bg_body;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, resource_ctx_->AliasedResourceUrl("sidebar"), settings,
      nullptr, nullptr, new SidebarBrowserViewDelegate());
  column_panel_->AddChildView(browser_view_);
  col_layout->SetFlexForView(browser_view_, 1);

  // ── Bottom: CEF-views panel with Activities + Flows buttons (flex 0) ─────
  cef_views_panel_ = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(kSidebarW, kCefPanelH)));
  cef_views_panel_->SetBackgroundColor(chrome.bg_body);

  CefBoxLayoutSettings btn_box;
  btn_box.horizontal = false;
  btn_box.inside_border_insets = {6, 8, 6, 8};
  btn_box.between_child_spacing = 4;
  auto btn_layout = cef_views_panel_->SetToBoxLayout(btn_box);

  const bool title_dark = ((chrome.text_title >> 8) & 0xFF) > 0x80;

  // Activities button.
  {
    btn_activities_ = MakeIconLabelButton(
        new FnButtonDelegate([this]() {
          CefPostTask(TID_UI,
                      base::BindOnce(
                          [](SidebarView* self) {
                            if (self->host_.open_singleton_tab)
                              self->host_.open_singleton_tab("activity");
                          },
                          this));
        }),
        IconId::kActivities, "Activities", "Open Activities");
    btn_activities_->SetTextColor(CEF_BUTTON_STATE_NORMAL, chrome.text_title);
    btn_activities_->SetTextColor(CEF_BUTTON_STATE_HOVERED, 0xFFFFFFFF);
    btn_activities_->SetBackgroundColor(chrome.bg_body);
    IconRegistry::ApplyToButton(btn_activities_, IconId::kActivities,
                                title_dark);
    cef_views_panel_->AddChildView(btn_activities_);
    btn_layout->SetFlexForView(btn_activities_, 0);
  }

  // Flows button.
  {
    btn_flows_ = MakeIconLabelButton(
        new FnButtonDelegate([this]() {
          CefPostTask(TID_UI, base::BindOnce(
                                  [](SidebarView* self) {
                                    if (self->host_.open_singleton_tab)
                                      self->host_.open_singleton_tab("flows");
                                  },
                                  this));
        }),
        IconId::kFlows, "Flows", "Open Flows");
    btn_flows_->SetTextColor(CEF_BUTTON_STATE_NORMAL, chrome.text_title);
    btn_flows_->SetTextColor(CEF_BUTTON_STATE_HOVERED, 0xFFFFFFFF);
    btn_flows_->SetBackgroundColor(chrome.bg_body);
    IconRegistry::ApplyToButton(btn_flows_, IconId::kFlows, title_dark);
    cef_views_panel_->AddChildView(btn_flows_);
    btn_layout->SetFlexForView(btn_flows_, 0);
  }

  column_panel_->AddChildView(cef_views_panel_);
  col_layout->SetFlexForView(cef_views_panel_, 0);

  Register(theme_ctx_);
  return column_panel_;
}

void SidebarView::ApplyTheme(const ThemeChrome& chrome) {
  if (column_panel_)
    column_panel_->SetBackgroundColor(chrome.bg_body);
  if (browser_view_)
    browser_view_->SetBackgroundColor(chrome.bg_body);
  if (cef_views_panel_)
    cef_views_panel_->SetBackgroundColor(chrome.bg_body);

  const bool title_dark = ((chrome.text_title >> 8) & 0xFF) > 0x80;
  if (btn_activities_) {
    btn_activities_->SetTextColor(CEF_BUTTON_STATE_NORMAL, chrome.text_title);
    btn_activities_->SetTextColor(CEF_BUTTON_STATE_HOVERED, chrome.text_title);
    btn_activities_->SetBackgroundColor(chrome.bg_body);
    IconRegistry::ApplyToButton(btn_activities_, IconId::kActivities,
                                title_dark);
  }
  if (btn_flows_) {
    btn_flows_->SetTextColor(CEF_BUTTON_STATE_NORMAL, chrome.text_title);
    btn_flows_->SetTextColor(CEF_BUTTON_STATE_HOVERED, chrome.text_title);
    btn_flows_->SetBackgroundColor(chrome.bg_body);
    IconRegistry::ApplyToButton(btn_flows_, IconId::kFlows, title_dark);
  }
  // Re-apply active button highlight after theme change.
  UpdateActiveButtonState(active_kind_);
}

void SidebarView::UpdateActiveButtonState(const std::string& kind) {
  active_kind_ = kind;
  const ThemeChrome chrome =
      theme_ctx_ ? theme_ctx_->GetCurrentChrome() : ThemeChrome{};

  if (btn_activities_) {
    const bool active = (kind == "activity");
    btn_activities_->SetBackgroundColor(active ? chrome.bg_content
                                               : chrome.bg_body);
  }
  if (btn_flows_) {
    const bool active = (kind == "flows");
    btn_flows_->SetBackgroundColor(active ? chrome.bg_content : chrome.bg_body);
  }
}

void SidebarView::SetVisible(bool visible) {
  if (column_panel_)
    column_panel_->SetVisible(visible);
}

}  // namespace cronymax
