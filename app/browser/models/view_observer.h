// app/browser/models/view_observer.h
//
// ViewObserver<EventT> / ViewObserverList<EventT> — the typed observer bus
// used by the MVC views layer to subscribe to model-state changes.
//
// ViewObserver<EventT>    — single-event observer interface; views inherit from
//                           one (or more) instantiations.
// ViewObserverList<EventT>— observer list backed by common::ObserverList<T>;
//                           convenience Notify(event) collapses the dispatch.
//
// Event structs are defined here so all subscribers include a single header:
//   ThemeChanged, SpaceChanged, TabsChanged, ActiveTabChanged.

#pragma once

#include <string>

#include "common/observer_list.h"
#include "include/cef_base.h" // cef_color_t

// ---------------------------------------------------------------------------
// ThemeChrome — resolved colour token set sent with ThemeChanged events.
// Defined here so views that include only this header get the complete type.
// ---------------------------------------------------------------------------
struct ThemeChrome {
  cef_color_t bg_body = 0;
  cef_color_t bg_base = 0;
  cef_color_t bg_float = 0;
  cef_color_t bg_mask = 0;
  cef_color_t border = 0;
  cef_color_t primary = 0;  // brand accent: teal
  cef_color_t text_title = 0;
  cef_color_t text_caption = 0;
};

// ---------------------------------------------------------------------------
// Event structs
// ---------------------------------------------------------------------------

struct ThemeChanged {
  ThemeChrome chrome;
};
struct SpaceChanged {
  std::string new_id;
  std::string new_name;
};
struct TabsChanged {};
struct ActiveTabChanged {
  std::string url;
  int browser_id = 0;
};

namespace cronymax {

// ---------------------------------------------------------------------------
// ViewObserver<EventT>
// ---------------------------------------------------------------------------

template <typename EventT> class ViewObserver {
public:
  virtual void OnViewObserved(const EventT &event) = 0;

protected:
  virtual ~ViewObserver() = default;
};

// ---------------------------------------------------------------------------
// ViewObserverList<EventT>
//
// Thin wrapper around ObserverList<ViewObserver<EventT>> that adds a one-arg
// Notify(event) convenience method so call sites stay concise.
// ---------------------------------------------------------------------------

template <typename EventT> class ViewObserverList {
public:
  ViewObserverList() = default;

  void AddObserver(ViewObserver<EventT> *obs) { list_.AddObserver(obs); }
  void RemoveObserver(ViewObserver<EventT> *obs) { list_.RemoveObserver(obs); }
  bool HasObserver(const ViewObserver<EventT> *obs) const {
    return list_.HasObserver(obs);
  }
  bool empty() const { return list_.empty(); }

  // Dispatch the event to all registered observers.
  void Notify(const EventT &event) {
    list_.Notify(&ViewObserver<EventT>::OnViewObserved, event);
  }

private:
  ObserverList<ViewObserver<EventT>> list_;
};

} // namespace cronymax
