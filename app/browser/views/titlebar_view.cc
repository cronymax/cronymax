// app/browser/views/titlebar_view.cc
//
// native-views-mvc Phase 9: TitleBarView implementation.

#include "browser/views/titlebar_view.h"
#include <optional>

#include "browser/icon_registry.h"
#include "browser/models/view_observer.h"
#include "browser/platform/view_style.h"
#include "browser/views/view_helpers.h"
#include "include/base/cef_callback.h"
#include "include/cef_menu_model.h"
#include "include/cef_parser.h"
#include "include/cef_task.h"
#include "include/views/cef_box_layout.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {

namespace {
constexpr int kTitleBarH = 38;
constexpr cef_color_t kTitleBarBgFallback = 0xFF14141A;
constexpr cef_color_t kTitleBarBtnFg = 0xFFE5E5EA;
#if defined(__APPLE__)
constexpr int kLightsPadW = 78;
constexpr int kWinPadW = 0;
#else
constexpr int kLightsPadW = 0;
constexpr int kWinPadW = 138;
#endif
}  // namespace

TitleBarView::TitleBarView(SpaceContext* space,
                           WindowActionContext* window_ctx,
                           OverlayActionContext* overlay,
                           ResourceContext* resources,
                           ThemeContext* theme_ctx,
                           CefRefPtr<CefWindow> main_win,
                           Host host)
    : space_ctx_(space),
      window_ctx_(window_ctx),
      overlay_ctx_(overlay),
      resource_ctx_(resources),
      theme_ctx_(theme_ctx),
      main_win_(std::move(main_win)),
      host_(std::move(host)) {
  space_ctx_->AddSpaceObserver(this);
}

TitleBarView::~TitleBarView() {
  space_ctx_->RemoveSpaceObserver(this);
}

CefRefPtr<CefPanel> TitleBarView::Build() {
  const std::optional<ThemeChrome> theme =
      theme_ctx_
          ? std::make_optional<ThemeChrome>(theme_ctx_->GetCurrentChrome())
          : std::nullopt;
  auto panel =
      CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(0, kTitleBarH)));
  panel->SetBackgroundColor(theme ? theme->bg_body : kTitleBarBgFallback);

  CefBoxLayoutSettings box;
  box.horizontal = true;
  box.inside_border_insets = {6, 8, 6, 8};
  box.between_child_spacing = 6;
  auto layout = panel->SetToBoxLayout(box);

  // 1. macOS traffic-light reservation.
  lights_pad_ = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(kLightsPadW, kTitleBarH - 12)));
  lights_pad_->SetBackgroundColor(theme ? theme->bg_body : kTitleBarBgFallback);
  panel->AddChildView(lights_pad_);
  layout->SetFlexForView(lights_pad_, 0);

  // 1a. Space selector.
  {
    static constexpr int kNewSpaceCmd = 9000;
    const std::string init_label =
        space_ctx_->GetCurrentSpaceName().empty()
            ? "Default \u25BE"
            : space_ctx_->GetCurrentSpaceName() + " \u25BE";

    auto delegate = new FnMenuButtonDelegate(
        [this](CefRefPtr<CefMenuButton> btn, const CefPoint& pt,
               CefRefPtr<CefMenuButtonPressedLock> /*lock*/) {
          const auto spaces = space_ctx_->GetSpaces();
          const std::string active_id = space_ctx_->GetCurrentSpaceId();
          auto menu = CefMenuModel::CreateMenuModel(
              new FnMenuModelDelegate([this, spaces](int cmd) {
                if (cmd == kNewSpaceCmd) {
                  // Open the native folder picker first; only show the profile
                  // picker card once the user has chosen a folder.
                  if (host_.run_file_dialog) {
                    host_.run_file_dialog([this](const std::string& path) {
                      if (path.empty())
                        return;
                      if (host_.show_profile_picker) {
                        host_.show_profile_picker(path);
                      }
                    });
                  }
                } else if (cmd >= 0 && cmd < static_cast<int>(spaces.size())) {
                  space_ctx_->SwitchSpace(spaces[cmd].first);
                }
              }));
          for (int i = 0; i < static_cast<int>(spaces.size()); ++i) {
            menu->AddItem(i, spaces[i].second);
            if (spaces[i].first == active_id)
              menu->SetChecked(i, true);
          }
          menu->AddSeparator();
          menu->AddItem(kNewSpaceCmd, "Open Folder\u2026");
          btn->ShowMenu(menu, pt, CEF_MENU_ANCHOR_TOPLEFT);
        });
    btn_space_ = CefMenuButton::CreateMenuButton(delegate, init_label);
    btn_space_->SetTextColor(CEF_BUTTON_STATE_NORMAL,
                             theme ? theme->text_title : kTitleBarBtnFg);
    btn_space_->SetTextColor(CEF_BUTTON_STATE_HOVERED,
                             theme ? theme->text_title : kTitleBarBtnFg);
    btn_space_->SetTextColor(CEF_BUTTON_STATE_PRESSED,
                             theme ? theme->text_title : kTitleBarBtnFg);
    btn_space_->SetBackgroundColor(theme ? theme->bg_body
                                         : kTitleBarBgFallback);
    panel->AddChildView(btn_space_);
    layout->SetFlexForView(btn_space_, 0);
  }

  // 1b. Sidebar toggle.
  {
    btn_sidebar_toggle_ = MakeIconLabelButton(
        new FnButtonDelegate([this]() {
          CefPostTask(TID_UI, base::BindOnce(
                                  [](WindowActionContext* ctx) {
                                    ctx->ToggleSidebar();
                                  },
                                  window_ctx_));
        }),
        IconId::kSidebarToggle, "", "Toggle sidebar");
    btn_sidebar_toggle_->SetTextColor(
        CEF_BUTTON_STATE_NORMAL, theme ? theme->text_title : kTitleBarBtnFg);
    btn_sidebar_toggle_->SetTextColor(CEF_BUTTON_STATE_HOVERED, 0xFFFFFFFF);
    btn_sidebar_toggle_->SetBackgroundColor(theme ? theme->bg_body
                                                  : kTitleBarBgFallback);
    panel->AddChildView(btn_sidebar_toggle_);
    layout->SetFlexForView(btn_sidebar_toggle_, 0);
  }

  // 1c. New-tab buttons.
  auto add_new_tab_btn = [&](CefRefPtr<CefLabelButton>* slot, IconId icon,
                             const std::string& label,
                             const std::string& tooltip,
                             const std::string& kind) {
    auto btn = MakeIconLabelButton(
        new FnButtonDelegate([this, kind]() {
          CefPostTask(TID_UI, base::BindOnce(
                                  [](TitleBarView* self, std::string k) {
                                    if (self->host_.open_new_tab)
                                      self->host_.open_new_tab(k);
                                  },
                                  this, kind));
        }),
        icon, label, tooltip);
    btn->SetTextColor(CEF_BUTTON_STATE_NORMAL,
                      theme ? theme->text_title : kTitleBarBtnFg);
    btn->SetTextColor(CEF_BUTTON_STATE_HOVERED, 0xFFFFFFFF);
    btn->SetBackgroundColor(theme ? theme->bg_body : kTitleBarBgFallback);
    panel->AddChildView(btn);
    layout->SetFlexForView(btn, 0);
    *slot = btn;
  };
  add_new_tab_btn(&btn_term_, IconId::kTabTerminal, "", "New terminal",
                  "terminal");
  add_new_tab_btn(&btn_web_, IconId::kTabWeb, "", "New web tab", "web");
  add_new_tab_btn(&btn_chat_, IconId::kTabChat, "", "New chat", "chat");

  // 2. Drag spacer.
  spacer_ = CefPanel::CreatePanel(nullptr);
  spacer_->SetBackgroundColor(theme ? theme->bg_body : kTitleBarBgFallback);
  panel->AddChildView(spacer_);
  layout->SetFlexForView(spacer_, 1);

  // 3. Popover buttons.
  auto add_popover_btn = [&](CefRefPtr<CefLabelButton>* slot, IconId icon,
                             const std::string& label,
                             const std::string& tooltip,
                             const std::string& resource) {
    auto btn = MakeIconLabelButton(
        new FnButtonDelegate([this, resource]() {
          CefPostTask(TID_UI, base::BindOnce(
                                  [](TitleBarView* self, std::string r) {
                                    self->overlay_ctx_->OpenPopover(
                                        self->resource_ctx_->ResourceUrl(r));
                                  },
                                  this, resource));
        }),
        icon, label, tooltip);
    btn->SetTextColor(CEF_BUTTON_STATE_NORMAL,
                      theme ? theme->text_title : kTitleBarBtnFg);
    btn->SetTextColor(CEF_BUTTON_STATE_HOVERED, 0xFFFFFFFF);
    btn->SetBackgroundColor(theme ? theme->bg_body : kTitleBarBgFallback);
    panel->AddChildView(btn);
    layout->SetFlexForView(btn, 0);
    *slot = btn;
  };
  add_popover_btn(&btn_flows_, IconId::kFlows, "Flows", "Open Flows",
                  "panels/flows/index.html");
  add_popover_btn(&btn_settings_, IconId::kSettings, "Settings",
                  "Open Settings", "panels/settings/index.html");

  // 4. Windows-controls slot (zero width on macOS).
  win_pad_ =
      CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(kWinPadW, 1)));
  win_pad_->SetBackgroundColor(theme ? theme->bg_body : kTitleBarBgFallback);
  panel->AddChildView(win_pad_);
  layout->SetFlexForView(win_pad_, 0);

  titlebar_panel_ = panel;
  Register(theme_ctx_);
  return panel;
}

void TitleBarView::RefreshDragRegion() {
#if defined(__APPLE__)
  if (!main_win_ || !titlebar_panel_)
    return;
  const CefRect bar = titlebar_panel_->GetBoundsInScreen();
  if (bar.width <= 0 || bar.height <= 0)
    return;
  const CefRect win = main_win_->GetBounds();
  const CefRect bar_in_window(bar.x - win.x, bar.y - win.y, bar.width,
                              bar.height);

  std::vector<CefRect> nodrag;
  if (lights_pad_) {
    CefRect lr = lights_pad_->GetBoundsInScreen();
    if (lr.width > 0 && lr.height > 0)
      nodrag.emplace_back(lr.x - win.x, lr.y - win.y, lr.width, lr.height);
  }
  auto add = [&](const CefRefPtr<CefLabelButton>& b) {
    if (!b)
      return;
    CefRect r = b->GetBoundsInScreen();
    if (r.width <= 0 || r.height <= 0)
      return;
    nodrag.emplace_back(r.x - win.x, r.y - win.y, r.width, r.height);
  };
  auto add_view = [&](const CefRefPtr<CefView>& b) {
    if (!b)
      return;
    CefRect r = b->GetBoundsInScreen();
    if (r.width <= 0 || r.height <= 0)
      return;
    nodrag.emplace_back(r.x - win.x, r.y - win.y, r.width, r.height);
  };
  add(btn_sidebar_toggle_);
  add_view(btn_space_);
  add(btn_web_);
  add(btn_term_);
  add(btn_chat_);
  add(btn_flows_);
  add(btn_settings_);
  InstallTitleBarDragOverlay(main_win_->GetWindowHandle(), bar_in_window,
                             nodrag.empty() ? nullptr : nodrag.data(),
                             nodrag.size());
#endif
}

void TitleBarView::ApplyTheme(const ThemeChrome& chrome) {
  if (titlebar_panel_)
    titlebar_panel_->SetBackgroundColor(chrome.bg_body);
  if (lights_pad_)
    lights_pad_->SetBackgroundColor(chrome.bg_body);
  if (spacer_)
    spacer_->SetBackgroundColor(chrome.bg_body);
  if (win_pad_)
    win_pad_->SetBackgroundColor(chrome.bg_body);
  if (btn_space_) {
    btn_space_->SetTextColor(CEF_BUTTON_STATE_NORMAL, chrome.text_title);
    btn_space_->SetTextColor(CEF_BUTTON_STATE_HOVERED, chrome.text_title);
    btn_space_->SetTextColor(CEF_BUTTON_STATE_PRESSED, chrome.text_title);
    btn_space_->SetBackgroundColor(chrome.bg_body);
  }
  const bool title_dark = ((chrome.text_title >> 8) & 0xFF) > 0x80;
  constexpr IconId kIcons[] = {IconId::kSidebarToggle, IconId::kTabWeb,
                               IconId::kTabTerminal,   IconId::kTabChat,
                               IconId::kFlows,         IconId::kSettings};
  CefRefPtr<CefLabelButton>* kBtns[] = {&btn_sidebar_toggle_, &btn_web_,
                                        &btn_term_,           &btn_chat_,
                                        &btn_flows_,          &btn_settings_};
  for (int i = 0; i < 6; ++i) {
    auto* b = kBtns[i]->get();
    if (!b)
      continue;
    b->SetTextColor(CEF_BUTTON_STATE_NORMAL, chrome.text_title);
    b->SetTextColor(CEF_BUTTON_STATE_HOVERED, chrome.text_title);
    b->SetBackgroundColor(chrome.bg_body);
    IconRegistry::ApplyToButton(*kBtns[i], kIcons[i], title_dark);
  }
#if defined(__APPLE__)
  if (main_win_) {
    SetMainWindowBackgroundColor(main_win_->GetWindowHandle(), chrome.bg_body);
    SetAppAppearance(chrome.text_title > 0x80808080);
  }
#endif
}

void TitleBarView::UpdateSpaceName(const std::string& name) {
  if (btn_space_)
    btn_space_->SetText(name + " \u25BE");
}

void TitleBarView::OnViewObserved(const SpaceChanged& e) {
  UpdateSpaceName(e.new_name);
}

}  // namespace cronymax
