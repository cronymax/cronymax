// Copyright (c) 2026.

#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>

#include "browser/mac_view_style.h"

namespace cronymax {

namespace {

CACornerMask ToCACornerMask(int mask) {
  CACornerMask out = 0;
  if (mask & kCornerTopLeft)     out |= kCALayerMinXMaxYCorner;
  if (mask & kCornerTopRight)    out |= kCALayerMaxXMaxYCorner;
  if (mask & kCornerBottomLeft)  out |= kCALayerMinXMinYCorner;
  if (mask & kCornerBottomRight) out |= kCALayerMaxXMinYCorner;
  return out;
}

}  // namespace
}  // namespace cronymax

// Subclass of NSVisualEffectView whose entire area is a window-drag region.
// Used as the chrome backdrop — the strip exposed around the chrome panels
// (top + left + right insets configured by the root CefBoxLayout) becomes a
// native drag handle.
@interface CronymaxDragVisualEffectView : NSVisualEffectView
@end

@implementation CronymaxDragVisualEffectView
- (BOOL)mouseDownCanMoveWindow { return YES; }
@end

namespace cronymax {

void StyleOverlayBrowserView(void* nsview_ptr,
                             double radius,
                             int corner_mask,
                             bool with_shadow) {
  if (!nsview_ptr) return;
  NSView* view = (__bridge NSView*)nsview_ptr;

  // Round the requested corners on the BrowserView's NSView itself.
  view.wantsLayer = YES;
  if (CALayer* layer = view.layer) {
    layer.cornerRadius = radius;
    layer.maskedCorners = ToCACornerMask(corner_mask);
    layer.masksToBounds = YES;
  }

  // Drop shadow lives on the parent overlay container so it can render
  // outside the rounded child. CEF places each overlay BrowserView into its
  // own host NSView; that's what we want to shadow. Walk up one level so we
  // don't paint into the clipped layer above.
  if (!with_shadow) return;
  NSView* host = view.superview;
  if (!host) return;
  host.wantsLayer = YES;
  if (CALayer* hl = host.layer) {
    hl.masksToBounds = NO;
    hl.shadowColor = [NSColor blackColor].CGColor;
    hl.shadowOpacity = 0.35f;
    hl.shadowRadius = 28.0f;
    hl.shadowOffset = CGSizeMake(0, -10);
  }
}

void ApplyCardStyle(void* nsview_ptr) {
  if (!nsview_ptr) return;
  NSView* view = (__bridge NSView*)nsview_ptr;

  view.wantsLayer = YES;
  if (CALayer* layer = view.layer) {
    layer.cornerRadius = 10.0;
    layer.maskedCorners = kCALayerMinXMinYCorner | kCALayerMaxXMinYCorner |
                          kCALayerMinXMaxYCorner | kCALayerMaxXMaxYCorner;
    layer.masksToBounds = YES;
    layer.borderWidth = 1.0;
    // Default dark border; chrome theme phase will retint via a getter.
    NSColor* border = [NSColor colorWithSRGBRed:0.06 green:0.06 blue:0.07
                                          alpha:1.0];
    layer.borderColor = border.CGColor;
  }

  NSView* host = view.superview;
  if (!host) return;
  host.wantsLayer = YES;
  if (CALayer* hl = host.layer) {
    hl.masksToBounds = NO;
    hl.shadowColor = [NSColor blackColor].CGColor;
    hl.shadowOpacity = 0.30f;
    hl.shadowRadius = 18.0f;
    hl.shadowOffset = CGSizeMake(0, -6);
  }
}

void StyleMainWindowTranslucent(void* nswindow_ptr) {
  if (!nswindow_ptr) return;
  // CEF returns the NSView* of the window's content view as the window
  // handle, not the NSWindow itself. Walk up to the hosting NSWindow.
  NSView* content = (__bridge NSView*)nswindow_ptr;
  NSWindow* window = content.window;
  if (!window) return;

  // Title bar disappears into the content; traffic lights still render and
  // the top edge remains a drag region.
  window.styleMask |= NSWindowStyleMaskFullSizeContentView;
  window.titlebarAppearsTransparent = YES;
  window.titleVisibility = NSWindowTitleHidden;
  window.movableByWindowBackground = YES;

  // Solid opaque dark window — NO NSVisualEffectView. Vibrancy under the
  // AppKit titlebar zone reads visibly different from vibrancy under the
  // body region; a flat opaque color guarantees a single uniform chrome.
  static NSColor* const kChromeColor =
      [NSColor colorWithSRGBRed:0x14 / 255.0
                          green:0x14 / 255.0
                           blue:0x1A / 255.0
                          alpha:1.0];
  window.opaque = YES;
  window.backgroundColor = kChromeColor;
  window.hasShadow = YES;

  content.wantsLayer = YES;
  if (CALayer* cl = content.layer) {
    cl.cornerRadius = 12.0;
    cl.masksToBounds = YES;
    cl.backgroundColor = kChromeColor.CGColor;
  }
}

void StyleContentBrowserView(void* nsview_ptr,
                             double radius,
                             bool with_shadow) {
  if (!nsview_ptr) return;
  NSView* view = (__bridge NSView*)nsview_ptr;
  view.wantsLayer = YES;
  if (CALayer* layer = view.layer) {
    layer.cornerRadius = radius;
    layer.maskedCorners =
        kCALayerMinXMinYCorner | kCALayerMaxXMinYCorner |
        kCALayerMinXMaxYCorner | kCALayerMaxXMaxYCorner;
    layer.masksToBounds = YES;
    layer.borderWidth = 0.5;
    layer.borderColor = [NSColor colorWithWhite:1.0 alpha:0.08].CGColor;
  }
  if (!with_shadow) return;
  NSView* host = view.superview;
  if (!host) return;
  host.wantsLayer = YES;
  if (CALayer* hl = host.layer) {
    hl.masksToBounds = NO;
    hl.shadowColor = [NSColor blackColor].CGColor;
    hl.shadowOpacity = 0.25f;
    hl.shadowRadius = 14.0f;
    hl.shadowOffset = CGSizeMake(0, -2);
  }
}

void MakeBrowserViewTransparent(void* nsview_ptr) {
  if (!nsview_ptr) return;
  NSView* view = (__bridge NSView*)nsview_ptr;
  view.wantsLayer = YES;
  if (CALayer* l = view.layer) {
    l.backgroundColor = [NSColor clearColor].CGColor;
    l.opaque = NO;
  }
  // Recurse so any AppKit/Chromium child NSView (compositor host) is also
  // cleared. Some of these views paint solid white otherwise.
  for (NSView* sub in view.subviews) {
    sub.wantsLayer = YES;
    if (CALayer* sl = sub.layer) {
      sl.backgroundColor = [NSColor clearColor].CGColor;
      sl.opaque = NO;
    }
  }
}

void PerformWindowDrag(void* nswindow_ptr) {
  if (!nswindow_ptr) return;
  NSView* content = (__bridge NSView*)nswindow_ptr;
  NSWindow* window = content.window;
  if (!window) return;
  NSEvent* ev = [NSApp currentEvent];
  if (!ev) return;
  // performWindowDragWithEvent: must be called from a mouseDown event.
  if (ev.type == NSEventTypeLeftMouseDown ||
      ev.type == NSEventTypeLeftMouseDragged) {
    [window performWindowDragWithEvent:ev];
  }
}

}  // namespace cronymax

// View installed above a chrome BrowserView's NSView. Its hit-test path is
// recomputed from the most recent set of draggable regions; pixels inside
// the path become a window-drag handle, all others fall through to the
// underlying CEF browser view.
@interface CronymaxDragHitView : NSView
@property(nonatomic, strong) NSBezierPath* dragPath;
@property(nonatomic, unsafe_unretained) NSView* trackedHost;
@end

@implementation CronymaxDragHitView {
  NSInteger _tag;
}
- (void)setTag:(NSInteger)tag { _tag = tag; }
- (NSInteger)tag { return _tag; }
- (BOOL)mouseDownCanMoveWindow { return YES; }
- (BOOL)acceptsFirstMouse:(NSEvent*)event { return YES; }
- (NSView*)hitTest:(NSPoint)pointInSuperview {
  if (!self.dragPath) return nil;
  NSPoint local = [self convertPoint:pointInSuperview fromView:self.superview];
  if (![self.dragPath containsPoint:local]) return nil;
  return self;
}
- (void)mouseDown:(NSEvent*)event {
  // Hard fallback in case mouseDownCanMoveWindow isn't honoured for any
  // reason (e.g. window is non-movable, vibrancy quirks, etc.).
  NSWindow* w = self.window;
  if (w) [w performWindowDragWithEvent:event];
}
- (void)hostFrameDidChange:(NSNotification*)note {
  NSView* host = self.trackedHost;
  if (!host || !host.window || !self.superview) return;
  NSRect r = [host convertRect:host.bounds toView:self.superview];
  self.frame = r;
}
@end

namespace cronymax {

static constexpr NSInteger kDragOverlayTag = 0x44524147;  // 'DRAG'

void ApplyDraggableRegions(void* nsview_ptr,
                           const DragRegion* regions,
                           size_t count) {
  if (!nsview_ptr) return;
  NSView* host = (__bridge NSView*)nsview_ptr;
  NSWindow* window = host.window;
  if (!window) return;
  NSView* contentView = window.contentView;
  if (!contentView) return;

  // Find an existing overlay tracking this host (one per chrome panel).
  CronymaxDragHitView* overlay = nil;
  for (NSView* sv in contentView.subviews) {
    if (sv.tag == kDragOverlayTag &&
        [sv isKindOfClass:[CronymaxDragHitView class]]) {
      CronymaxDragHitView* candidate = (CronymaxDragHitView*)sv;
      if (candidate.trackedHost == host) {
        overlay = candidate;
        break;
      }
    }
  }

  // Frame of the chrome panel in window-content coordinates.
  NSRect frameInContent = [host convertRect:host.bounds toView:contentView];

  if (!overlay) {
    overlay = [[CronymaxDragHitView alloc] initWithFrame:frameInContent];
    overlay.tag = kDragOverlayTag;
    overlay.trackedHost = host;
    overlay.autoresizingMask = NSViewNotSizable;
    [contentView addSubview:overlay
                 positioned:NSWindowAbove
                 relativeTo:nil];
    host.postsFrameChangedNotifications = YES;
    [[NSNotificationCenter defaultCenter]
        addObserver:overlay
           selector:@selector(hostFrameDidChange:)
               name:NSViewFrameDidChangeNotification
             object:host];
  } else {
    overlay.frame = frameInContent;
    // Re-raise to the topmost subview so any later-added CEF children sit
    // below it.
    [overlay removeFromSuperview];
    [contentView addSubview:overlay
                 positioned:NSWindowAbove
                 relativeTo:nil];
  }

  // Build path = union(draggable) − union(no-drag). Web rects use
  // top-left origin; AppKit overlay (flipped=NO by default) uses bottom-left.
  const CGFloat H = overlay.bounds.size.height;
  NSBezierPath* drag = [NSBezierPath bezierPath];
  NSBezierPath* nodrag = [NSBezierPath bezierPath];
  for (size_t i = 0; i < count; ++i) {
    const auto& r = regions[i];
    NSRect rect = NSMakeRect(r.x, H - r.y - r.height, r.width, r.height);
    if (r.draggable) [drag appendBezierPathWithRect:rect];
    else             [nodrag appendBezierPathWithRect:rect];
  }
  drag.windingRule = NSWindingRuleEvenOdd;
  [drag appendBezierPath:nodrag];
  overlay.dragPath = drag;
}

}  // namespace cronymax

// native-title-bar: dedicated drag-handle NSView for the title-bar.
// mouseDownCanMoveWindow=YES so AppKit treats clicks here as window drags.
// hitTest: returns nil for points inside any `noDragRects` (the title-bar
// buttons) so clicks pass through to the CEF browser view that paints them.
// One singleton per contentView identified by tag.
@interface CronymaxTitleBarDragView : NSView
@property(nonatomic, assign) CGFloat barHeight;             // top strip height (AppKit pts)
@property(nonatomic, copy) NSArray<NSValue*>* noDragRects;  // NSRect, AppKit (bottom-up) overlay-local coords
@end

@implementation CronymaxTitleBarDragView {
  NSInteger _tag;
}
- (void)setTag:(NSInteger)tag { _tag = tag; }
- (NSInteger)tag { return _tag; }
// Return NO so AppKit delivers mouseDown: to us; we then explicitly call
// performWindowDragWithEvent:. (Returning YES would let AppKit consume the
// click, but inside an NSTitlebarAccessoryViewController it does not actually
// initiate a window drag.)
- (BOOL)mouseDownCanMoveWindow { return NO; }
- (BOOL)acceptsFirstMouse:(NSEvent*)event { (void)event; return YES; }
- (NSView*)hitTest:(NSPoint)pointInSuperview {
  NSPoint local = [self convertPoint:pointInSuperview fromView:self.superview];
  if (!NSPointInRect(local, self.bounds)) return nil;
  for (NSValue* v in self.noDragRects) {
    if (NSPointInRect(local, v.rectValue)) return nil;
  }
  return self;
}
- (void)mouseDown:(NSEvent*)event {
  NSWindow* w = self.window;
  if (w) [w performWindowDragWithEvent:event];
}

// Stay topmost across CEF subview reorderings.
- (void)viewDidMoveToWindow {
  [super viewDidMoveToWindow];
  NSView* parent = self.superview;
  if (!parent) return;
  [[NSNotificationCenter defaultCenter] removeObserver:self];
  [[NSNotificationCenter defaultCenter]
      addObserver:self
         selector:@selector(parentSubviewsDidChange:)
             name:NSViewFrameDidChangeNotification
           object:parent];
  if (self.window) {
    [[NSNotificationCenter defaultCenter]
        addObserver:self
           selector:@selector(parentSubviewsDidChange:)
               name:NSWindowDidUpdateNotification
             object:self.window];
    // Catch any AppKit event tick — far more aggressive than NSWindowDidUpdate.
    [[NSNotificationCenter defaultCenter]
        addObserver:self
           selector:@selector(parentSubviewsDidChange:)
               name:NSWindowDidBecomeKeyNotification
             object:self.window];
  }
  // KVO on the parent's subviews array catches every insertion / removal /
  // reorder that CEF performs as it mounts browser views.
  [parent addObserver:self
           forKeyPath:@"subviews"
              options:0
              context:NULL];
}
- (void)observeValueForKeyPath:(NSString*)keyPath
                      ofObject:(id)object
                        change:(NSDictionary<NSKeyValueChangeKey,id>*)change
                       context:(void*)context {
  (void)change; (void)context;
  if ([keyPath isEqualToString:@"subviews"] && object == self.superview) {
    [self parentSubviewsDidChange:nil];
  }
}
- (void)parentSubviewsDidChange:(NSNotification*)note {
  (void)note;
  NSView* parent = self.superview;
  if (!parent) return;
  if (parent.subviews.lastObject == self) return;
  [self retain];
  [self removeFromSuperview];
  [parent addSubview:self positioned:NSWindowAbove relativeTo:nil];
  [self release];
}
- (void)dealloc {
  if (self.superview) {
    @try { [self.superview removeObserver:self forKeyPath:@"subviews"]; }
    @catch (NSException*) {}
  }
  [[NSNotificationCenter defaultCenter] removeObserver:self];
  [super dealloc];
}
@end

namespace cronymax {

static constexpr NSInteger kTitleBarDragTag = 0x54424452;  // 'TBDR'

void InstallTitleBarDragOverlay(void* nswindow_handle,
                                const CefRect& bar_rect_window_coords,
                                const CefRect* nodrag_rects,
                                size_t nodrag_count) {
  if (!nswindow_handle) return;
  NSView* content = (__bridge NSView*)nswindow_handle;
  NSWindow* window = content.window;
  if (!window) return;
  // The window's contentView.superview is the AppKit "themeFrame". Subviews
  // installed there sit ABOVE the contentView (and therefore above any
  // CefBrowserView/CefPanel NSViews) and receive titlebar clicks even with
  // NSWindowStyleMaskFullSizeContentView + titlebarAppearsTransparent.
  NSView* themeFrame = content.superview;
  if (!themeFrame) return;

  CronymaxTitleBarDragView* overlay = nil;
  for (NSView* sv in themeFrame.subviews) {
    if (sv.tag == kTitleBarDragTag &&
        [sv isKindOfClass:[CronymaxTitleBarDragView class]]) {
      overlay = (CronymaxTitleBarDragView*)sv;
      break;
    }
  }

  const CGFloat W = themeFrame.bounds.size.width;
  const CGFloat H = themeFrame.bounds.size.height;
  const CGFloat barH = (CGFloat)bar_rect_window_coords.height;
  // themeFrame is NOT flipped (AppKit bottom-up). Title bar occupies top.
  const NSRect frame = NSMakeRect(0, H - barH, W, barH);

  if (!overlay) {
    overlay = [[CronymaxTitleBarDragView alloc] initWithFrame:frame];
    overlay.tag = kTitleBarDragTag;
    overlay.autoresizingMask = NSViewWidthSizable | NSViewMinYMargin;
    [themeFrame addSubview:overlay
                positioned:NSWindowAbove
                relativeTo:nil];
  } else {
    overlay.frame = frame;
    [overlay removeFromSuperview];
    [themeFrame addSubview:overlay
                positioned:NSWindowAbove
                relativeTo:nil];
  }
  overlay.barHeight = barH;

  // Button rects come in window top-down coords. Convert to overlay-local
  // (also flipped relative to AppKit, but since the overlay is non-flipped,
  // local.y = barH - window.y - h).
  NSMutableArray<NSValue*>* nodrag =
      [NSMutableArray arrayWithCapacity:nodrag_count];
  for (size_t i = 0; i < nodrag_count; ++i) {
    const auto& r = nodrag_rects[i];
    const CGFloat lx = r.x;
    const CGFloat ly = barH - r.y - r.height;
    [nodrag addObject:[NSValue valueWithRect:NSMakeRect(lx, ly, r.width, r.height)]];
  }
  overlay.noDragRects = nodrag;
}

}  // namespace cronymax
