// Copyright (c) 2026.

#include "browser/tab.h"

#include <cassert>
#include <utility>

#include "browser/tab_behavior.h"
#include "browser/tab_toolbar.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_fill_layout.h"

namespace cronymax {

const char* TabKindToString(TabKind kind) {
  switch (kind) {
    case TabKind::kWeb:      return "web";
    case TabKind::kTerminal: return "terminal";
    case TabKind::kChat:     return "chat";
    case TabKind::kAgent:    return "agent";
    case TabKind::kGraph:    return "graph";
  }
  return "unknown";
}

Tab::Tab(TabId id, TabKind kind, std::unique_ptr<TabBehavior> behavior)
    : id_(std::move(id)), kind_(kind), behavior_(std::move(behavior)) {
  assert(behavior_ != nullptr);
  assert(behavior_->Kind() == kind_);
}

Tab::~Tab() = default;

namespace {
// Same base dark chrome color used by TabToolbar (0xFF0E0E10).
// The card panel adopts this so any gap area matches the toolbar background,
// making the toolbar + content appear as one unified floating card.
constexpr cef_color_t kCardBgArgb = 0xFF0E0E10;
}  // namespace

void Tab::Build() {
  assert(!built_ && "Tab::Build called twice");
  built_ = true;

  card_ = CefPanel::CreatePanel(nullptr);
  card_->SetBackgroundColor(kCardBgArgb);
  CefBoxLayoutSettings card_box;
  card_box.horizontal = false;
  card_layout_ = card_->SetToBoxLayout(card_box);

  // Toolbar.
  toolbar_ = std::make_unique<TabToolbar>();
  CefRefPtr<CefPanel> toolbar_root = toolbar_->Build();
  card_->AddChildView(toolbar_root);
  card_layout_->SetFlexForView(toolbar_root, 0);
  behavior_->BuildToolbar(toolbar_.get(), this);

  // Content host (FillLayout, behavior populates exactly one child).
  content_host_ = CefPanel::CreatePanel(nullptr);
  content_host_->SetToFillLayout();
  card_->AddChildView(content_host_);
  card_layout_->SetFlexForView(content_host_, 1);

  CefRefPtr<CefView> content = behavior_->BuildContent(this);
  if (content) {
    content_host_->AddChildView(content);
  }
}

void Tab::OnToolbarState(const ToolbarState& state) {
  if (state.kind != kind_) {
    // Caller is responsible for the kind/tab mismatch check; reject silently.
    return;
  }
  if (behavior_) {
    behavior_->ApplyToolbarState(state);
  }
}

void Tab::SetToolbarState(const ToolbarState& state) {
  OnToolbarState(state);
}

void Tab::SetChromeTheme(const std::string& css_color_or_empty) {
  if (toolbar_) {
    toolbar_->SetChromeColor(css_color_or_empty);
  }
}

void Tab::RequestClose() {
  // Hooked up by TabManager in a later phase. Intentional no-op for now.
}

int Tab::browser_id() const {
  return behavior_ ? behavior_->BrowserId() : 0;
}

}  // namespace cronymax
