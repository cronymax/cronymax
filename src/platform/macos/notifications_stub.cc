// Non-Apple no-op fallback for cronymax::platform::macos notification API.
// Compiled when not building for __APPLE__ so cronymax_common always exposes
// the same symbols regardless of platform.

#include "platform/macos/notifications.h"

#if !defined(__APPLE__)

namespace cronymax::platform::macos {

void RequestNotificationAuth(std::function<void(bool)> cb) {
  if (cb) cb(false);
}

bool IsNotificationAuthorized() { return false; }

void PostNotification(const std::string&,
                      const std::string&,
                      const std::string&) {}

void SetDockBadgeCount(int) {}

void SetStatusDotState(StatusDotState) {}

void SetNotificationClickHandler(std::function<void(const std::string&)>) {}

}  // namespace cronymax::platform::macos

#endif
