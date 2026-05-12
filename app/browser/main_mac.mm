// Copyright (c) 2026.

#import <Cocoa/Cocoa.h>

#include "browser/app.h"
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

    CefRefPtr<cronymax::App> app(new cronymax::App());

    if (!CefInitialize(main_args, settings, app.get(), nullptr)) {
      return CefGetExitCode();
    }

    AiDesktopAppDelegate* delegate = [[AiDesktopAppDelegate alloc] init];
    NSApp.delegate = delegate;

    // Install a minimal main menu so macOS routes standard Edit key
    // equivalents (Cmd+C/X/V/A/Z) through the responder chain into the
    // focused CEF BrowserView / NSTextView / HTML input element.
    // Without this menu the actions are never looked up and copy/paste
    // silently do nothing.
    {
      NSMenu* mainMenu = [[NSMenu alloc] initWithTitle:@""];

      // App menu (index 0 — required by AppKit, title is ignored)
      NSMenuItem* appItem = [[NSMenuItem alloc] initWithTitle:@"App"
                                                       action:nil
                                                keyEquivalent:@""];
      NSMenu* appMenu = [[NSMenu alloc] initWithTitle:@"App"];
      [appMenu addItemWithTitle:@"Quit"
                         action:@selector(terminate:)
                  keyEquivalent:@"q"];
      appItem.submenu = appMenu;
      [mainMenu addItem:appItem];

      // Edit menu — supplies the key equivalents for copy/paste/etc.
      NSMenuItem* editItem = [[NSMenuItem alloc] initWithTitle:@"Edit"
                                                        action:nil
                                                 keyEquivalent:@""];
      NSMenu* editMenu = [[NSMenu alloc] initWithTitle:@"Edit"];
      [editMenu addItemWithTitle:@"Undo"
                          action:@selector(undo:)
                   keyEquivalent:@"z"];
      [editMenu addItemWithTitle:@"Redo"
                          action:@selector(redo:)
                   keyEquivalent:@"Z"];  // Shift+Cmd+Z
      [editMenu addItem:[NSMenuItem separatorItem]];
      [editMenu addItemWithTitle:@"Cut"
                          action:@selector(cut:)
                   keyEquivalent:@"x"];
      [editMenu addItemWithTitle:@"Copy"
                          action:@selector(copy:)
                   keyEquivalent:@"c"];
      [editMenu addItemWithTitle:@"Paste"
                          action:@selector(paste:)
                   keyEquivalent:@"v"];
      [editMenu addItemWithTitle:@"Select All"
                          action:@selector(selectAll:)
                   keyEquivalent:@"a"];
      editItem.submenu = editMenu;
      [mainMenu addItem:editItem];

      NSApp.mainMenu = mainMenu;
    }

    CefRunMessageLoop();
    CefShutdown();

#if !__has_feature(objc_arc)
    [delegate release];
#endif
    delegate = nil;
  }

  return 0;
}
