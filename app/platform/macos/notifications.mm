// Cronymax — macOS notifications + dock badge implementation.
//
// Built only on Apple platforms; CMake adds this file conditionally with
// Objective-C++ flags and links UserNotifications.framework + AppKit.framework.
// Non-Apple targets compile `notifications_stub.cc` instead.

#include "platform/macos/notifications.h"

#if defined(__APPLE__)

#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#import <UserNotifications/UserNotifications.h>

#include <atomic>
#include <mutex>

namespace cronymax::platform::macos {

namespace {

std::atomic<bool> g_authorized{false};
std::mutex g_handler_mutex;
std::function<void(const std::string&)> g_click_handler;

// Bridge AppKit's UNUserNotificationCenterDelegate to our C++ click handler.
}  // namespace
}  // namespace cronymax::platform::macos

@interface CronymaxNotificationDelegate
    : NSObject <UNUserNotificationCenterDelegate>
@end

@implementation CronymaxNotificationDelegate

- (void)userNotificationCenter:(UNUserNotificationCenter*)center
       willPresentNotification:(UNNotification*)notification
         withCompletionHandler:
             (void (^)(UNNotificationPresentationOptions))completionHandler {
  // Show banner + play sound when app is foregrounded.
  completionHandler(UNNotificationPresentationOptionBanner |
                    UNNotificationPresentationOptionSound);
}

- (void)userNotificationCenter:(UNUserNotificationCenter*)center
    didReceiveNotificationResponse:(UNNotificationResponse*)response
             withCompletionHandler:(void (^)(void))completionHandler {
  NSString* deeplink =
      response.notification.request.content.userInfo[@"deeplink"];
  std::string deeplinkStr = deeplink ? deeplink.UTF8String : "";
  std::function<void(const std::string&)> cb;
  {
    std::lock_guard<std::mutex> lock(
        cronymax::platform::macos::g_handler_mutex);
    cb = cronymax::platform::macos::g_click_handler;
  }
  if (cb) {
    dispatch_async(dispatch_get_main_queue(), ^{
      cb(deeplinkStr);
    });
  }
  completionHandler();
}

@end

namespace cronymax::platform::macos {

namespace {

CronymaxNotificationDelegate* SharedDelegate() {
  static CronymaxNotificationDelegate* instance =
      [[CronymaxNotificationDelegate alloc] init];
  return instance;
}

}  // namespace

void RequestNotificationAuth(std::function<void(bool granted)> cb) {
  UNUserNotificationCenter* center =
      [UNUserNotificationCenter currentNotificationCenter];
  center.delegate = SharedDelegate();
  UNAuthorizationOptions opts = UNAuthorizationOptionAlert |
                                UNAuthorizationOptionSound |
                                UNAuthorizationOptionBadge;
  // Capture cb by value into the block.
  auto cb_copy = std::make_shared<std::function<void(bool)>>(std::move(cb));
  [center
      requestAuthorizationWithOptions:opts
                    completionHandler:^(BOOL granted, NSError* /*error*/) {
                      g_authorized.store(granted, std::memory_order_release);
                      if (cb_copy && *cb_copy)
                        (*cb_copy)(granted);
                    }];
}

bool IsNotificationAuthorized() {
  return g_authorized.load(std::memory_order_acquire);
}

void PostNotification(const std::string& title,
                      const std::string& body,
                      const std::string& deeplink) {
  if (!IsNotificationAuthorized())
    return;
  UNUserNotificationCenter* center =
      [UNUserNotificationCenter currentNotificationCenter];
  UNMutableNotificationContent* content =
      [[UNMutableNotificationContent alloc] init];
  content.title = [NSString stringWithUTF8String:title.c_str()];
  content.body = [NSString stringWithUTF8String:body.c_str()];
  content.userInfo = @{
    @"deeplink" : [NSString stringWithUTF8String:deeplink.c_str()],
  };
  // Identifier: timestamp-based; UNUserNotificationCenter dedups by identifier
  // within a short window if the same id is posted twice.
  NSString* ident =
      [NSString stringWithFormat:@"cronymax-%@", [[NSUUID UUID] UUIDString]];
  UNNotificationRequest* req =
      [UNNotificationRequest requestWithIdentifier:ident
                                           content:content
                                           trigger:nil];
  [center addNotificationRequest:req withCompletionHandler:nil];
}

void SetDockBadgeCount(int count) {
  dispatch_async(dispatch_get_main_queue(), ^{
    NSDockTile* tile = [NSApp dockTile];
    if (count <= 0) {
      tile.badgeLabel = nil;
    } else {
      tile.badgeLabel = [NSString stringWithFormat:@"%d", count];
    }
  });
}

void SetStatusDotState(StatusDotState /*state*/) {
  // No native dot indicator on macOS — the renderer paints the dot in-window.
  // This stub is a no-op; reserved for future tray-icon integration.
}

void SetNotificationClickHandler(std::function<void(const std::string&)> cb) {
  std::lock_guard<std::mutex> lock(g_handler_mutex);
  g_click_handler = std::move(cb);
}

}  // namespace cronymax::platform::macos

#endif  // __APPLE__
