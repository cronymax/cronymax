// Cronymax — macOS native notifications + dock badge shim.
//
// Pure C-style API to keep the C++ side free of Objective-C headers. The
// implementation in `notifications.mm` is built only on Apple platforms;
// non-Apple builds get the no-op stubs below.

#pragma once

#include <cstdint>
#include <functional>
#include <string>

namespace cronymax::platform::macos {

// Status-dot mirror of the renderer-side `useStatusDotState()` enum.
// 0 = off, 1 = activity, 2 = attention, 3 = error.
enum class StatusDotState : int {
  kOff = 0,
  kActivity = 1,
  kAttention = 2,
  kError = 3,
};

// Request notification authorization from UNUserNotificationCenter.
// `cb` is invoked exactly once on the system queue with `granted=true|false`.
// On non-Apple platforms `cb(false)` is called synchronously.
void RequestNotificationAuth(std::function<void(bool granted)> cb);

// True if the user has already granted (and not later revoked) authorization.
// Returns the cached answer; call after `RequestNotificationAuth` resolves.
bool IsNotificationAuthorized();

// Post a banner notification. `deeplink` is opaquely round-tripped via the
// notification's userInfo so the click handler can route back to a panel.
// Title and body should be plain UTF-8 (no markup).
void PostNotification(const std::string& title,
                      const std::string& body,
                      const std::string& deeplink);

// Set the dock-tile badge label. Pass 0 to clear.
void SetDockBadgeCount(int count);

// Mirror the renderer status dot to the dock-tile colour ring.
// On macOS this is a best-effort visual hint via the dock icon.
void SetStatusDotState(StatusDotState state);

// Register a handler invoked on the main thread when the user clicks a
// notification. Receives the `deeplink` string that was passed to
// `PostNotification`. Setting an empty handler clears the registration.
void SetNotificationClickHandler(
    std::function<void(const std::string& deeplink)> cb);

}  // namespace cronymax::platform::macos
