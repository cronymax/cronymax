#include "browser/views/content_view.h"

#include "browser/platform/view_style.h"
#include "include/base/cef_callback.h"
#include "include/cef_task.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_fill_layout.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {

namespace {
constexpr double kContentCornerRadius = 10.0;
}  // namespace

ContentView::ContentView(TabsContext* tabs,
                         ThemeContext* theme_ctx,
                         CefRefPtr<CefWindow> main_win,
                         Host host)
    : tabs_(tabs),
      theme_ctx_(theme_ctx),
      main_win_(std::move(main_win)),
      host_(std::move(host)) {
  tabs_->AddActiveTabObserver(this);
}

ContentView::~ContentView() {
  tabs_->RemoveActiveTabObserver(this);
}

CefRefPtr<CefPanel> ContentView::Build() {
  content_outer_ = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings content_box;
  content_box.horizontal = false;
  // refine-ui-theme-layout: breathing room around the rounded card on the
  // sides and bottom.  Top is 0 so the card sits flush against the titlebar.
  content_box.inside_border_insets = {0, 8, 8, 8};
  auto content_outer_layout = content_outer_->SetToBoxLayout(content_box);

  content_frame_ = CefPanel::CreatePanel(nullptr);
  content_frame_->SetToFillLayout();
  content_outer_->AddChildView(content_frame_);
  content_outer_layout->SetFlexForView(content_frame_, 1);

  content_panel_ = CefPanel::CreatePanel(nullptr);
  content_panel_->SetToFillLayout();
  content_frame_->AddChildView(content_panel_);

  Register(theme_ctx_);
  return content_outer_;
}

void ContentView::RemoveCard(const std::string& tab_id) {
  auto it = mounted_cards_.find(tab_id);
  if (it == mounted_cards_.end())
    return;
  if (content_panel_ && it->second)
    content_panel_->RemoveChildView(it->second);
  mounted_cards_.erase(it);
}

void ContentView::ApplyTheme(const ThemeChrome& chrome) {
  bg_body_ = chrome.bg_body;
  if (content_outer_)
    content_outer_->SetBackgroundColor(chrome.bg_body);
  if (content_frame_)
    content_frame_->SetBackgroundColor(chrome.bg_base);
  // Re-round active tab's corners with the new bg (theme switch can alter
  // the color used for the shadow sibling).
  auto [tab_id, card, bv] = host_.active_tab();
  (void)tab_id;
  (void)card;
  if (bv)
    RoundCornersFor(bv, main_win_, bg_body_);
}

void ContentView::SetVInsets(int top, int bottom) {
  if (!content_outer_ || !content_frame_)
    return;
  CefBoxLayoutSettings box;
  box.horizontal = false;
  box.inside_border_insets = {top, 8, bottom, 8};
  auto layout = content_outer_->SetToBoxLayout(box);
  layout->SetFlexForView(content_frame_, 1);
  content_outer_->Layout();

  // The card has moved — re-assert corner punch views so they track the new
  // card bounds (same pattern as ApplyThemeChrome).
  auto [tab_id, card, bv] = host_.active_tab();
  (void)tab_id;
  (void)card;
  if (bv)
    RoundCornersFor(bv, main_win_, bg_body_);
}

/*static*/
void ContentView::RoundCornersFor(CefRefPtr<CefBrowserView> bv,
                                  CefRefPtr<CefWindow> win,
                                  cef_color_t bg_body) {
  if (!bv || !win)
    return;
  CefPostTask(
      TID_UI,
      base::BindOnce(
          [](CefRefPtr<CefBrowserView> v, CefRefPtr<CefWindow> w,
             cef_color_t bg) {
            if (!v || !w)
              return;
            // Walk the CEF view tree (not NSView tree) to find the card panel:
            //   BrowserView -> Tab::content_host_ -> Tab::card_
            // We use the CEF view API because GetWindowHandle() returns the
            // window root NSView (BridgedContentView), not the BrowserView's
            // NSView.
            CefRefPtr<CefView> content_host = v->GetParentView();
            CefRefPtr<CefView> card =
                content_host ? content_host->GetParentView() : nullptr;
            CefRefPtr<CefView> target =
                card ? card
                     : (content_host ? content_host : CefRefPtr<CefView>(v));
            if (!target)
              return;

            CefPoint origin{0, 0};
            target->ConvertPointToWindow(origin);
            CefRect bounds = target->GetBounds();
            CefRect card_rect{origin.x, origin.y, bounds.width, bounds.height};

            // w->GetWindowHandle() = BridgedContentView (window root NSView).
            // StyleContentBrowserView places CronymaxCornerPunchView instances
            // at the card's four corners, painting bg with a quarter-circle
            // cutout so both the card AND content_frame_ appear rounded (the
            // card fills content_frame_ exactly via FillLayout).
            void* window_nsview = w->GetWindowHandle();
            StyleContentBrowserView(window_nsview, kContentCornerRadius, bg,
                                    card_rect);
          },
          bv, win, bg_body));
}

void ContentView::OnViewObserved(const ActiveTabChanged& /*e*/) {
  ShowActiveCard();
}

void ContentView::ShowActiveCard() {
  auto [tab_id, card, bv] = host_.active_tab();

  // Hide every mounted card that is NOT the active one.
  for (auto& [id, c] : mounted_cards_) {
    if (id != tab_id && c)
      c->SetVisible(false);
  }

  if (tab_id.empty() || !card) {
    if (content_panel_)
      content_panel_->Layout();
    host_.update_popover_visibility();
    return;
  }

  // Mount the card into content_panel_ if this is the first activation.
  if (mounted_cards_.find(tab_id) == mounted_cards_.end()) {
    content_panel_->AddChildView(card);
    mounted_cards_[tab_id] = card;
  }
  card->SetVisible(true);
  content_panel_->Layout();

  // Apply corner-punch overlays. Posted so the underlying NSView is
  // realized (no-op on non-Apple via platform stub).
  RoundCornersFor(bv, main_win_, bg_body_);

  // Re-install the AppKit drag overlay after NSView re-parenting.
  host_.refresh_drag_region();

  // Request keyboard focus (web tabs only; Host handles the TabKind check).
  host_.request_focus();

  // Update popover visibility for the newly-active tab.
  host_.update_popover_visibility();
}

}  // namespace cronymax
