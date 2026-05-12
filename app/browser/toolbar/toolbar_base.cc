// Copyright (c) 2026.

#include "browser/toolbar/toolbar_base.h"

#include "include/views/cef_button_delegate.h"
#include "include/views/cef_fill_layout.h"
#include "include/views/cef_panel_delegate.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

class SizedPanelDelegate : public CefPanelDelegate {
 public:
  explicit SizedPanelDelegate(CefSize sz) : sz_(sz) {}
  CefSize GetPreferredSize(CefRefPtr<CefView>) override { return sz_; }

 private:
  CefSize sz_;
  IMPLEMENT_REFCOUNTING(SizedPanelDelegate);
  DISALLOW_COPY_AND_ASSIGN(SizedPanelDelegate);
};

class FnButtonDelegate : public CefButtonDelegate {
 public:
  explicit FnButtonDelegate(std::function<void()> fn) : fn_(std::move(fn)) {}
  void OnButtonPressed(CefRefPtr<CefButton>) override {
    if (fn_)
      fn_();
  }

 private:
  std::function<void()> fn_;
  IMPLEMENT_REFCOUNTING(FnButtonDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnButtonDelegate);
};

constexpr int kBtnSz = 28;

}  // namespace

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

CefRefPtr<CefPanel> ToolbarBase::Build(ThemeContext* ctx,
                                       CefRefPtr<CefPanel> parent) {
  if (parent) {
    root_ = parent;
  } else {
    root_ = CefPanel::CreatePanel(nullptr);
  }

  CefBoxLayoutSettings root_box;
  root_box.horizontal = true;
  root_box.inside_border_insets = {4, 8, 4, 8};
  root_box.between_child_spacing = 6;
  root_box.cross_axis_alignment = CEF_AXIS_ALIGNMENT_CENTER;
  root_layout_ = root_->SetToBoxLayout(root_box);

  CefBoxLayoutSettings slot_box;
  slot_box.horizontal = true;
  slot_box.between_child_spacing = 4;
  slot_box.cross_axis_alignment = CEF_AXIS_ALIGNMENT_CENTER;

  leading_ = CefPanel::CreatePanel(nullptr);
  leading_->SetToBoxLayout(slot_box);
  root_->AddChildView(leading_);
  root_layout_->SetFlexForView(leading_, 0);

  middle_ = CefPanel::CreatePanel(nullptr);
  middle_->SetToBoxLayout(slot_box);
  root_->AddChildView(middle_);
  root_layout_->SetFlexForView(middle_, 1);

  trailing_ = CefPanel::CreatePanel(nullptr);
  trailing_->SetToBoxLayout(slot_box);
  root_->AddChildView(trailing_);
  root_layout_->SetFlexForView(trailing_, 0);

  // Create middle widget; it will be colored properly when Register→ApplyTheme
  // fires below, but we need a placeholder chrome to size/create it now.
  ThemeChrome placeholder_chrome;
  if (ctx)
    placeholder_chrome = ctx->GetCurrentChrome();
  CefRefPtr<CefView> mid_widget = CreateMiddleWidget(placeholder_chrome);
  if (mid_widget) {
    middle_->AddChildView(mid_widget);
    if (auto layout = middle_->GetLayout()) {
      if (auto box = layout->AsBoxLayout()) {
        box->SetFlexForView(mid_widget, 1);
      }
    }
  }

  // Subscribe to theme; seeds ApplyTheme immediately.
  if (ctx)
    Register(ctx);

  return root_;
}

// ---------------------------------------------------------------------------
// Action management
// ---------------------------------------------------------------------------

ToolbarBase::ActionHandle ToolbarBase::AddAction(
    CefRefPtr<CefPanel> slot,
    IconId icon,
    std::string_view tooltip,
    std::function<void()> callback) {
  auto btn =
      MakeIconButton(new FnButtonDelegate(std::move(callback)), icon, tooltip);
  IconRegistry::ApplyToButton(btn, icon, dark_mode_);

  auto wrapper =
      CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(kBtnSz, kBtnSz)));
  wrapper->SetToFillLayout();
  wrapper->AddChildView(btn);
  slot->AddChildView(wrapper);

  if (auto layout = slot->GetLayout()) {
    if (auto box = layout->AsBoxLayout()) {
      box->SetFlexForView(wrapper, 0);
    }
  }

  return static_cast<int>(leading_actions_.size() +
                          trailing_actions_.size());  // temporary index
}

ToolbarBase::ActionHandle ToolbarBase::AddLeadingAction(
    IconId icon,
    std::string_view tooltip,
    std::function<void()> callback) {
  if (!leading_)
    return kInvalidHandle;

  auto btn =
      MakeIconButton(new FnButtonDelegate(std::move(callback)), icon, tooltip);
  if (current_bg_)
    btn->SetBackgroundColor(current_bg_);
  IconRegistry::ApplyToButton(btn, icon, dark_mode_);

  auto wrapper =
      CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(kBtnSz, kBtnSz)));
  wrapper->SetToFillLayout();
  if (current_bg_)
    wrapper->SetBackgroundColor(current_bg_);
  wrapper->AddChildView(btn);
  leading_->AddChildView(wrapper);

  if (auto layout = leading_->GetLayout()) {
    if (auto box = layout->AsBoxLayout()) {
      box->SetFlexForView(wrapper, 0);
    }
  }

  ActionHandle handle = static_cast<int>(leading_actions_.size());
  leading_actions_.push_back({icon, wrapper, btn});
  return handle;
}

ToolbarBase::ActionHandle ToolbarBase::AddTrailingAction(
    IconId icon,
    std::string_view tooltip,
    std::function<void()> callback) {
  if (!trailing_)
    return kInvalidHandle;

  auto btn =
      MakeIconButton(new FnButtonDelegate(std::move(callback)), icon, tooltip);
  if (current_bg_)
    btn->SetBackgroundColor(current_bg_);
  IconRegistry::ApplyToButton(btn, icon, dark_mode_);

  auto wrapper =
      CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(kBtnSz, kBtnSz)));
  wrapper->SetToFillLayout();
  if (current_bg_)
    wrapper->SetBackgroundColor(current_bg_);
  wrapper->AddChildView(btn);
  trailing_->AddChildView(wrapper);

  if (auto layout = trailing_->GetLayout()) {
    if (auto box = layout->AsBoxLayout()) {
      box->SetFlexForView(wrapper, 0);
    }
  }

  ActionHandle handle =
      kTrailingBase + static_cast<int>(trailing_actions_.size());
  trailing_actions_.push_back({icon, wrapper, btn});
  return handle;
}

ToolbarBase::ActionEntry* ToolbarBase::EntryForHandle(ActionHandle handle) {
  if (handle == kInvalidHandle)
    return nullptr;
  if (handle >= kTrailingBase) {
    int idx = handle - kTrailingBase;
    if (idx >= 0 && idx < static_cast<int>(trailing_actions_.size()))
      return &trailing_actions_[idx];
  } else {
    if (handle >= 0 && handle < static_cast<int>(leading_actions_.size()))
      return &leading_actions_[handle];
  }
  return nullptr;
}

void ToolbarBase::SetActionEnabled(ActionHandle handle, bool enabled) {
  if (auto* e = EntryForHandle(handle)) {
    if (e->btn)
      e->btn->SetEnabled(enabled);
  }
}

void ToolbarBase::UpdateActionIcon(ActionHandle handle, IconId new_icon) {
  if (auto* e = EntryForHandle(handle)) {
    e->icon = new_icon;
    if (e->btn)
      IconRegistry::ApplyToButton(e->btn, new_icon, dark_mode_);
  }
}

// ---------------------------------------------------------------------------
// UpdateActionBackgrounds
// ---------------------------------------------------------------------------

void ToolbarBase::UpdateActionBackgrounds(cef_color_t bg, bool dark_mode) {
  current_bg_ = bg;
  dark_mode_ = dark_mode;
  for (auto& e : leading_actions_) {
    if (e.btn) {
      e.btn->SetBackgroundColor(bg);
      IconRegistry::ApplyToButton(e.btn, e.icon, dark_mode_);
    }
    if (e.wrapper)
      e.wrapper->SetBackgroundColor(bg);
  }
  for (auto& e : trailing_actions_) {
    if (e.btn) {
      e.btn->SetBackgroundColor(bg);
      IconRegistry::ApplyToButton(e.btn, e.icon, dark_mode_);
    }
    if (e.wrapper)
      e.wrapper->SetBackgroundColor(bg);
  }
}

// ---------------------------------------------------------------------------
// ApplyTheme
// ---------------------------------------------------------------------------

void ToolbarBase::ApplyTheme(const ThemeChrome& chrome) {
  const cef_color_t bg = chrome.bg_float;
  current_bg_ = bg;
  dark_mode_ = ((chrome.text_title >> 8) & 0xFF) > 0x80;

  if (root_)
    root_->SetBackgroundColor(bg);
  if (leading_)
    leading_->SetBackgroundColor(bg);
  if (middle_)
    middle_->SetBackgroundColor(bg);
  if (trailing_)
    trailing_->SetBackgroundColor(bg);

  UpdateActionBackgrounds(bg, dark_mode_);

  ApplyMiddleTheme(chrome);
}

}  // namespace cronymax
