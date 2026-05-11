// app/browser/views/profile_picker_overlay.cc
//
// workspace-with-profile D9: ProfilePickerOverlay implementation.

#include "browser/views/profile_picker_overlay.h"

#include "browser/platform/view_style.h"
#include "browser/views/view_helpers.h"
#include "include/base/cef_callback.h"
#include "include/cef_menu_model.h"
#include "include/cef_task.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_fill_layout.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {

namespace {

// Card visual constants.
constexpr cef_color_t kScrimColor       = 0x99000000;  // 60% black
constexpr cef_color_t kCardBgFallback   = 0xFF2C2C2E;
constexpr cef_color_t kTextFallback     = 0xFFE5E5EA;
constexpr cef_color_t kCaptionFallback  = 0xFF8E8E93;

}  // namespace

// ---------------------------------------------------------------------------
// Constructor / Build
// ---------------------------------------------------------------------------

ProfilePickerOverlay::ProfilePickerOverlay(ThemeContext* theme_ctx,
                                           CefRefPtr<CefWindow> main_win,
                                           Host host)
    : theme_ctx_(theme_ctx),
      main_win_(std::move(main_win)),
      host_(std::move(host)) {}

void ProfilePickerOverlay::Build() {
  CEF_REQUIRE_UI_THREAD();

  // ── Slot A: scrim (full-window dimmer) ──────────────────────────────────
  auto scrim_panel = CefPanel::CreatePanel(nullptr);
  scrim_panel->SetBackgroundColor(kScrimColor);
  scrim_oc_ = main_win_->AddOverlayView(
      scrim_panel, CEF_DOCKING_MODE_CUSTOM, /*can_activate=*/false);
  scrim_oc_->SetVisible(false);

  // ── Slot B: card (dialog) ───────────────────────────────────────────────
  card_panel_ = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(kCardW, kCardH)));
  card_panel_->SetBackgroundColor(kCardBgFallback);
  card_oc_ = main_win_->AddOverlayView(
      card_panel_, CEF_DOCKING_MODE_CUSTOM, /*can_activate=*/true);
  card_oc_->SetVisible(false);

  // Card: vertical box layout with inner padding.
  CefBoxLayoutSettings vbox;
  vbox.horizontal = false;
  vbox.inside_border_insets = {16, 20, 16, 20};
  vbox.between_child_spacing = 10;
  auto vlayout = card_panel_->SetToBoxLayout(vbox);

  // ── Title label ─────────────────────────────────────────────────────────
  title_btn_ = CefLabelButton::CreateLabelButton(
      new FnButtonDelegate([]() {}), "Open Folder");
  title_btn_->SetEnabled(false);
  title_btn_->SetTextColor(CEF_BUTTON_STATE_DISABLED, kTextFallback);
  title_btn_->SetFontList("Sans Bold, 13px");
  title_btn_->SetBackgroundColor(0x00000000);
  card_panel_->AddChildView(title_btn_);
  vlayout->SetFlexForView(title_btn_, 0);

  // ── Path row (textfield + Browse button) ────────────────────────────────
  auto path_row = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(0, 28)));
  path_row->SetBackgroundColor(0x00000000);
  CefBoxLayoutSettings hbox_path;
  hbox_path.horizontal = true;
  hbox_path.between_child_spacing = 8;
  auto hlayout_path = path_row->SetToBoxLayout(hbox_path);

  path_field_ = CefTextfield::CreateTextfield(nullptr);
  path_field_->SetPlaceholderText("/path/to/folder");
  path_field_->SetTextColor(kTextFallback);
  path_field_->SetBackgroundColor(0xFF1C1C1E);
  path_row->AddChildView(path_field_);
  hlayout_path->SetFlexForView(path_field_, 1);

  browse_btn_ = CefLabelButton::CreateLabelButton(
      new FnButtonDelegate([this]() {
        if (!host_.run_file_dialog) return;
        host_.run_file_dialog([this](const std::string& path) {
          if (path.empty()) return;
          path_field_->SetText(path);
          SetOpenEnabled(true);
        });
      }),
      "Browse\u2026");
  browse_btn_->SetBackgroundColor(0xFF3A3A3C);
  browse_btn_->SetEnabledTextColors(kTextFallback);
  browse_btn_->SetMinimumSize(CefSize(72, 26));
  path_row->AddChildView(browse_btn_);
  hlayout_path->SetFlexForView(browse_btn_, 0);

  card_panel_->AddChildView(path_row);
  vlayout->SetFlexForView(path_row, 0);

  // ── Profile row (CefMenuButton) ─────────────────────────────────────────
  auto profile_row = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(0, 28)));
  profile_row->SetBackgroundColor(0x00000000);
  CefBoxLayoutSettings hbox_prof;
  hbox_prof.horizontal = true;
  hbox_prof.between_child_spacing = 8;
  auto hlayout_prof = profile_row->SetToBoxLayout(hbox_prof);

  auto profile_label_btn = CefLabelButton::CreateLabelButton(
      new FnButtonDelegate([]() {}), "Profile:");
  profile_label_btn->SetEnabled(false);
  profile_label_btn->SetTextColor(CEF_BUTTON_STATE_DISABLED, kCaptionFallback);
  profile_label_btn->SetBackgroundColor(0x00000000);
  profile_label_btn->SetMinimumSize(CefSize(52, 26));
  profile_row->AddChildView(profile_label_btn);
  hlayout_prof->SetFlexForView(profile_label_btn, 0);

  profile_btn_ = CefMenuButton::CreateMenuButton(
      new FnMenuButtonDelegate(
          [this](CefRefPtr<CefMenuButton> btn, const CefPoint& pt,
                 CefRefPtr<CefMenuButtonPressedLock> /*lock*/) {
            if (!host_.get_profiles) return;
            const auto profiles = host_.get_profiles();
            auto menu = CefMenuModel::CreateMenuModel(
                new FnMenuModelDelegate([this, profiles](int cmd) {
                  if (cmd < 0 || cmd >= static_cast<int>(profiles.size()))
                    return;
                  selected_profile_id_   = profiles[cmd].id;
                  selected_profile_name_ = profiles[cmd].name;
                  profile_btn_->SetText(selected_profile_name_ + " \u25BE");
                }));
            for (int i = 0; i < static_cast<int>(profiles.size()); ++i) {
              menu->AddItem(i, profiles[i].name);
              if (profiles[i].id == selected_profile_id_)
                menu->SetChecked(i, true);
            }
            btn->ShowMenu(menu, pt, CEF_MENU_ANCHOR_TOPLEFT);
          }),
      selected_profile_name_ + " \u25BE");
  profile_btn_->SetBackgroundColor(0xFF3A3A3C);
  profile_btn_->SetEnabledTextColors(kTextFallback);
  profile_row->AddChildView(profile_btn_);
  hlayout_prof->SetFlexForView(profile_btn_, 1);

  card_panel_->AddChildView(profile_row);
  vlayout->SetFlexForView(profile_row, 0);

  // ── Error label (hidden until an error occurs) ───────────────────────────
  error_label_ = CefLabelButton::CreateLabelButton(
      new FnButtonDelegate([]() {}), "");
  error_label_->SetEnabled(false);
  error_label_->SetTextColor(CEF_BUTTON_STATE_DISABLED, 0xFFFF453A);
  error_label_->SetBackgroundColor(0x00000000);
  error_label_->SetVisible(false);
  card_panel_->AddChildView(error_label_);
  vlayout->SetFlexForView(error_label_, 0);

  // ── Button row (Cancel  Open) ───────────────────────────────────────────
  auto btn_row = CefPanel::CreatePanel(
      new SizedPanelDelegate(CefSize(0, 32)));
  btn_row->SetBackgroundColor(0x00000000);
  CefBoxLayoutSettings hbox_btns;
  hbox_btns.horizontal = true;
  hbox_btns.between_child_spacing = 8;
  auto hlayout_btns = btn_row->SetToBoxLayout(hbox_btns);

  // Flexible spacer pushes Cancel + Open to the right.
  auto spacer = CefPanel::CreatePanel(nullptr);
  spacer->SetBackgroundColor(0x00000000);
  btn_row->AddChildView(spacer);
  hlayout_btns->SetFlexForView(spacer, 1);

  cancel_btn_ = CefLabelButton::CreateLabelButton(
      new FnButtonDelegate([this]() { Hide(); }),
      "Cancel");
  cancel_btn_->SetBackgroundColor(0xFF3A3A3C);
  cancel_btn_->SetEnabledTextColors(kTextFallback);
  cancel_btn_->SetMinimumSize(CefSize(72, 28));
  btn_row->AddChildView(cancel_btn_);
  hlayout_btns->SetFlexForView(cancel_btn_, 0);

  open_btn_ = CefLabelButton::CreateLabelButton(
      new FnButtonDelegate([this]() {
        if (!open_btn_->IsEnabled()) return;
        const std::string path = path_field_->GetText().ToString();
        if (path.empty()) return;
        open_btn_->SetEnabled(false);
        ShowError("");
        const std::string json =
            host_.create_space ? host_.create_space(path, selected_profile_id_)
                               : std::string{};
        if (json.empty()) {
          ShowError("Could not open folder. Check that the path exists.");
          open_btn_->SetEnabled(true);
          return;
        }
        if (host_.send_space_created_event) {
          host_.send_space_created_event(json);
        }
        Hide();
      }),
      "Open");
  open_btn_->SetEnabled(false);
  open_btn_->SetBackgroundColor(0xFF0A84FF);
  open_btn_->SetEnabledTextColors(0xFFFFFFFF);
  open_btn_->SetTextColor(CEF_BUTTON_STATE_DISABLED, 0xFF636366);
  open_btn_->SetMinimumSize(CefSize(72, 28));
  btn_row->AddChildView(open_btn_);
  hlayout_btns->SetFlexForView(open_btn_, 0);

  card_panel_->AddChildView(btn_row);
  vlayout->SetFlexForView(btn_row, 0);

  // Subscribe to theme changes and apply the current chrome immediately.
  Register(theme_ctx_);
}

// ---------------------------------------------------------------------------
// Show / Hide
// ---------------------------------------------------------------------------

void ProfilePickerOverlay::Show(const std::string& prefill_path) {
  CEF_REQUIRE_UI_THREAD();

  // Reset state.
  path_field_->SelectAll(false);
  path_field_->ExecuteCommand(CEF_TFC_DELETE);
  if (!prefill_path.empty()) {
    path_field_->SetText(prefill_path);
  }
  SetOpenEnabled(!prefill_path.empty());
  ShowError("");
  selected_profile_id_   = "default";
  selected_profile_name_ = "Default";
  RefreshProfileButton();

  // Position and reveal.
  LayoutCard();

  const CefRect wb = main_win_->GetBounds();
  if (scrim_oc_->IsValid()) {
    scrim_oc_->SetBounds(CefRect(0, 0, wb.width, wb.height));
    scrim_oc_->SetVisible(true);
  }
  if (card_oc_->IsValid()) {
    card_oc_->SetVisible(true);
  }

  // First Show(): style the card overlay after CEF has had one event-loop
  // tick to attach the overlay NSWindow (addChildWindow: is deferred by CEF
  // until the overlay is made visible, so we must call this here — not in
  // Build — and defer by one additional tick to ensure the child window is
  // fully parented before we walk the NSWindow hierarchy).
  if (!card_styled_) {
    card_styled_ = true;
    CefPostTask(TID_UI, base::BindOnce(
        [](CefRefPtr<CefWindow> w) {
          void* nsv = CaptureLastChildNSView(
              reinterpret_cast<void*>(w->GetWindowHandle()));
          if (nsv) StylePickerCard(nsv, 0xFF2C2C2E);
        },
        main_win_));
  }
}

void ProfilePickerOverlay::Hide() {
  CEF_REQUIRE_UI_THREAD();

  if (scrim_oc_->IsValid()) scrim_oc_->SetVisible(false);
  if (card_oc_->IsValid())  card_oc_->SetVisible(false);

  // Reset text state so reopening is clean.
  path_field_->SelectAll(false);
  path_field_->ExecuteCommand(CEF_TFC_DELETE);
  ShowError("");
  SetOpenEnabled(false);
  selected_profile_id_   = "default";
  selected_profile_name_ = "Default";
}

// ---------------------------------------------------------------------------
// Layout helpers
// ---------------------------------------------------------------------------

void ProfilePickerOverlay::LayoutCard() {
  const CefRect wb = main_win_->GetBounds();
  const int x = (wb.width  - kCardW) / 2;
  const int y = (wb.height - kCardH) / 2;
  if (card_oc_->IsValid()) {
    card_oc_->SetBounds(CefRect(std::max(0, x), std::max(0, y), kCardW, kCardH));
  }
}

void ProfilePickerOverlay::SetOpenEnabled(bool enabled) {
  if (!open_btn_) return;
  open_btn_->SetEnabled(enabled);
  if (enabled) {
    // Active state: brand primary background, white text.
    open_btn_->SetBackgroundColor(primary_);
    open_btn_->SetEnabledTextColors(0xFFFFFFFF);
  } else {
    // Disabled state: same muted background as other buttons.
    open_btn_->SetBackgroundColor(btn_bg_);
    open_btn_->SetTextColor(CEF_BUTTON_STATE_DISABLED, btn_fg_);
  }
}

void ProfilePickerOverlay::RefreshProfileButton() {
  if (profile_btn_) {
    profile_btn_->SetText(selected_profile_name_ + " \u25BE");
  }
}

void ProfilePickerOverlay::ShowError(const std::string& msg) {
  if (!error_label_) return;
  if (msg.empty()) {
    error_label_->SetVisible(false);
  } else {
    error_label_->SetText(msg);
    error_label_->SetVisible(true);
  }
}

// ---------------------------------------------------------------------------
// ApplyTheme
// ---------------------------------------------------------------------------

void ProfilePickerOverlay::ApplyTheme(const ThemeChrome& chrome) {
  if (!card_panel_) return;  // Called before Build() on some platforms.

  card_panel_->SetBackgroundColor(chrome.bg_float ? chrome.bg_float
                                                   : kCardBgFallback);

  // Cache theme colors for use in SetOpenEnabled.
  btn_bg_ = chrome.border  ? chrome.border  : 0xFF3A3A3C;
  btn_fg_ = chrome.text_title ? chrome.text_title : kTextFallback;
  primary_ = chrome.primary ? chrome.primary : 0xFF22B8A7;

  // Update title label text color (was hardcoded, invisible in light mode).
  if (title_btn_) {
    title_btn_->SetTextColor(CEF_BUTTON_STATE_DISABLED,
                             chrome.text_title ? chrome.text_title
                                               : kTextFallback);
  }

  if (path_field_) {
    path_field_->SetBackgroundColor(chrome.bg_base ? chrome.bg_base
                                                    : 0xFF1C1C1E);
    path_field_->SetTextColor(chrome.text_title ? chrome.text_title
                                                 : kTextFallback);
  }

  // Use chrome.border as button background: it has real contrast against
  // bg_float in both modes.
  //   Light: border=0xFFD5E2DE on card bg_float=0xFFFFFFFF → visible gray-green
  //   Dark:  border=0xFF29403D on card bg_float=0xFF182625 → lighter teal
  const cef_color_t btn_bg = chrome.border ? chrome.border : 0xFF3A3A3C;
  const cef_color_t fg     = chrome.text_title ? chrome.text_title
                                                : kTextFallback;
  if (browse_btn_) {
    browse_btn_->SetBackgroundColor(btn_bg);
    browse_btn_->SetEnabledTextColors(fg);
  }
  if (profile_btn_) {
    profile_btn_->SetBackgroundColor(btn_bg);
    profile_btn_->SetEnabledTextColors(fg);
  }
  if (cancel_btn_) {
    cancel_btn_->SetBackgroundColor(btn_bg);
    cancel_btn_->SetEnabledTextColors(fg);
  }
  // Re-apply open button state with refreshed theme colors.
  if (open_btn_) {
    SetOpenEnabled(open_btn_->IsEnabled());
  }
}

}  // namespace cronymax
