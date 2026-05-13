// app/browser/views/profile_picker_overlay.h
//
// workspace-with-profile D9: ProfilePickerOverlay — pure CefPanel native
// dialog for opening a folder with a selected sandbox profile.
//
// Pre-allocates two overlay slots on the main window:
//   scrim  — full-window semi-transparent dimmer
//   card   — 440 × 220 dialog panel centered in the window
//
// MainWindow constructs the overlay in BuildOverlaySlots(), stores it as
// profile_picker_overlay_, and wires the Host callbacks.
// TitleBarView::Host::show_profile_picker() calls Show().

#pragma once

#include <functional>
#include <string>
#include <vector>

#include "browser/models/theme_aware_view.h"
#include "browser/models/view_context.h"
#include "include/views/cef_label_button.h"
#include "include/views/cef_menu_button.h"
#include "include/views/cef_overlay_controller.h"
#include "include/views/cef_panel.h"
#include "include/views/cef_textfield.h"
#include "include/views/cef_window.h"
#include "runtime/profile_store.h"

namespace cronymax {

class ProfilePickerOverlay : public ThemeAwareView {
 public:
  struct Host {
    // Runs the native folder-picker dialog; calls callback with the chosen
    // path (empty string if cancelled).
    std::function<void(std::function<void(const std::string&)>)>
        run_file_dialog;
    // Returns all available profiles.
    std::function<std::vector<ProfileRecord>()> get_profiles;
    // Creates a new Space at root_path with the given profile_id.
    // Returns the new Space JSON string on success, empty string on failure.
    std::function<std::string(const std::string& path,
                              const std::string& profile_id)>
        create_space;
    // Broadcast the space.created event to all panels.
    std::function<void(const std::string& space_json)> send_space_created_event;
  };

  ProfilePickerOverlay(ThemeContext* theme_ctx,
                       CefRefPtr<CefWindow> main_win,
                       Host host);
  ~ProfilePickerOverlay() override = default;

  // Pre-allocate the two overlay slots and build the card view tree.
  // Must be called once during window construction (inside BuildOverlaySlots).
  void Build();

  // Show / hide the dialog. If prefill_path is non-empty the path field is
  // pre-populated and the Open button is immediately enabled.
  void Show(const std::string& prefill_path = {});
  void Hide();

  // ThemeAwareView
  void ApplyTheme(const ThemeChrome& chrome) override;

 private:
  // Center card_oc_ over the main window.
  void LayoutCard();
  // Enable / disable the Open button.
  void SetOpenEnabled(bool enabled);
  // Reload the profile CefMenuButton label from selected_profile_id_.
  void RefreshProfileButton();
  // Show an error message; pass empty string to hide.
  void ShowError(const std::string& msg);

  ThemeContext* theme_ctx_;
  CefRefPtr<CefWindow> main_win_;
  Host host_;

  // Overlay controllers (pre-allocated in Build).
  CefRefPtr<CefOverlayController> scrim_oc_;
  CefRefPtr<CefOverlayController> card_oc_;

  // Card root panel (owned by the overlay view tree).
  CefRefPtr<CefPanel> card_panel_;

  // Interactive child views inside the card.
  CefRefPtr<CefLabelButton> title_btn_;  // static title — themed in ApplyTheme
  CefRefPtr<CefTextfield> path_field_;
  CefRefPtr<CefMenuButton> profile_btn_;
  CefRefPtr<CefLabelButton> browse_btn_;
  CefRefPtr<CefLabelButton> open_btn_;
  CefRefPtr<CefLabelButton> cancel_btn_;
  CefRefPtr<CefLabelButton> error_label_;

  // Cached theme colors so SetOpenEnabled can restore correct button state.
  cef_color_t btn_bg_ = 0xFF3A3A3C;
  cef_color_t btn_fg_ = 0xFFE5E5EA;
  cef_color_t primary_ = 0xFF22B8A7;

  // Selected profile state (reset to "default" on Hide).
  std::string selected_profile_id_ = "default";
  std::string selected_profile_name_ = "Default";

  // Cached card dimensions — see LayoutCard().
  static constexpr int kCardW = 440;
  static constexpr int kCardH = 222;

  // True after StylePickerCard has been applied (done once on first Show).
  bool card_styled_ = false;
};

}  // namespace cronymax
