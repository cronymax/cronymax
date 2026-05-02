// Copyright (c) 2026.

#import <Cocoa/Cocoa.h>

#include "browser/desktop_app.h"
#include "include/cef_application_mac.h"
#include "include/cef_command_line.h"
#include "include/wrapper/cef_helpers.h"
#include "include/wrapper/cef_library_loader.h"

@interface AiDesktopAppDelegate : NSObject <NSApplicationDelegate>
@end

@interface AiDesktopApplication : NSApplication <CefAppProtocol> {
 @private
  BOOL handlingSendEvent_;
}
@end

@implementation AiDesktopApplication
- (BOOL)isHandlingSendEvent {
  return handlingSendEvent_;
}

- (void)setHandlingSendEvent:(BOOL)handlingSendEvent {
  handlingSendEvent_ = handlingSendEvent;
}

- (void)sendEvent:(NSEvent*)event {
  CefScopedSendingEvent sendingEventScoper;
  [super sendEvent:event];
}

- (void)terminate:(id)sender {
  CefQuitMessageLoop();
}
@end

@implementation AiDesktopAppDelegate
- (BOOL)applicationSupportsSecureRestorableState:(NSApplication*)app {
  return YES;
}

- (NSApplicationTerminateReply)applicationShouldTerminate:
    (NSApplication*)sender {
  CefQuitMessageLoop();
  return NSTerminateCancel;
}
@end

int main(int argc, char* argv[]) {
  CefScopedLibraryLoader library_loader;
  if (!library_loader.LoadInMain()) {
    return 1;
  }

  CefMainArgs main_args(argc, argv);

  @autoreleasepool {
    [AiDesktopApplication sharedApplication];
    CHECK([NSApp isKindOfClass:[AiDesktopApplication class]]);

    CefSettings settings;
#if !defined(CEF_USE_SANDBOX)
    settings.no_sandbox = true;
#endif
    // Default browser background is transparent so panels (sidebar, etc.)
    // composite over the NSVisualEffectView vibrancy without a white flash
    // and without an opaque GPU clear color showing through.
    settings.background_color = 0x00000000;

    CefRefPtr<cronymax::DesktopApp> app(
        new cronymax::DesktopApp());

    if (!CefInitialize(main_args, settings, app.get(), nullptr)) {
      return CefGetExitCode();
    }

    AiDesktopAppDelegate* delegate = [[AiDesktopAppDelegate alloc] init];
    NSApp.delegate = delegate;

    CefRunMessageLoop();
    CefShutdown();

#if !__has_feature(objc_arc)
    [delegate release];
#endif
    delegate = nil;
  }

  return 0;
}

