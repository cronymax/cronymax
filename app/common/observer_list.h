// app/common/observer_list.h
//
// Generic observer-list infrastructure, inspired by Chromium's
// base::ObserverList<T>.
//
// Provides a reusable ObserverList<ObserverType> that:
//   - stores typed observer pointers (raw, non-owning)
//   - takes a snapshot before iteration so removals inside notifications
//     are safe (the removed pointer is skipped if it no longer appears in
//     the live list)
//   - DCHECKs on double-add, remove-of-unknown, and destruction with
//     live observers
//
// Usage:
//   class FooObserver {
//    public:
//     virtual void OnFoo(const FooEvent& e) = 0;
//    protected:
//     virtual ~FooObserver() = default;
//   };
//
//   ObserverList<FooObserver> observers;
//   observers.AddObserver(&my_obs);
//   observers.Notify(&FooObserver::OnFoo, event);
//   observers.RemoveObserver(&my_obs);
//
// The ObserverType does NOT need to inherit from any special base class.

#pragma once

#include <algorithm>
#include <vector>

#include "include/wrapper/cef_helpers.h"  // DCHECK

namespace cronymax {

template <typename ObserverType>
class ObserverList {
 public:
  ObserverList() = default;

  ~ObserverList() {
    DCHECK(observers_.empty())
        << "ObserverList destroyed with live observers. "
           "All observers must remove themselves before the list is destroyed.";
  }

  // Non-copyable, non-movable — observer pointers must remain stable.
  ObserverList(const ObserverList&) = delete;
  ObserverList& operator=(const ObserverList&) = delete;

  void AddObserver(ObserverType* obs) {
    DCHECK(obs);
    DCHECK(!HasObserver(obs)) << "Observer already registered.";
    observers_.push_back(obs);
  }

  void RemoveObserver(ObserverType* obs) {
    auto it = std::find(observers_.begin(), observers_.end(), obs);
    DCHECK(it != observers_.end()) << "Observer not registered.";
    if (it != observers_.end())
      observers_.erase(it);
  }

  bool HasObserver(const ObserverType* obs) const {
    return std::find(observers_.cbegin(), observers_.cend(), obs) !=
           observers_.cend();
  }

  bool empty() const { return observers_.empty(); }

  // Dispatch a notification via a member-function pointer with zero or more
  // arguments.  A snapshot of the observer list is taken before iterating so
  // removals (or additions) inside an OnXxx handler do not invalidate the
  // iteration.
  template <typename Method, typename... Args>
  void Notify(Method method, Args&&... args) {
    const std::vector<ObserverType*> snapshot = observers_;
    for (ObserverType* obs : snapshot)
      (obs->*method)(std::forward<Args>(args)...);
  }

 private:
  std::vector<ObserverType*> observers_;
};

}  // namespace cronymax
