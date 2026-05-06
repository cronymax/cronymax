// Copyright (c) 2026.

#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#import <objc/runtime.h>

#include "browser/mac_view_style.h"

// ---------------------------------------------------------------------------
// Popover drop-shadow NSView
// ---------------------------------------------------------------------------
// Uses Core Animation's built-in layer shadow (shadowPath + shadowOpacity etc.)
// rather than CGContext drawing.  CA renders the shadow at WindowServer
// compositing time — ABOVE CEF's IOSurface layers, so it is always visible.
//
// Why CGContext approaches (drawRect: + CGContextSetShadowWithColor / kCGBlendModeClear)
// failed in earlier iterations:
//
//   1. CGContextSetShadowWithColor generates a SOFTWARE shadow written into
//      the layer's own bitmap backing store.  That bitmap lives in the AppKit
//      CA layer tree, which composites BELOW CEF's GPU-compositor IOSurface
//      layers.  The shadow pixels are perpetually hidden under the IOSurface.
//
//   2. kCGBlendModeClear is supposed to erase the opaque fill so only the
//      shadow ring survives.  In practice, when CA manages the context for a
//      layer-backed view, the blend mode may not write (0,0,0,0) as expected,
//      leaving the opaque black fill visible through the transparent IOSurface
//      during page load — causing the "lost background color" regression.
//
// CA shadowPath approach:
//   • Layer has NO content (backgroundColor = clearColor, no drawRect:).
//   • shadowPath is set to the popover's rounded-rect outline.
//   • CA / WindowServer renders the shadow FROM this path at compositing time,
//     spreading it outward into the margin area.
//   • Because the shadow is composited by WindowServer at the CA level (not
//     drawn into a bitmap), it sits ABOVE the CEF IOSurface in the final
//     frame — it is visible.  The layer interior is transparent, so the
//     popover's CEF content (including background_color during load) shows
//     through unmodified.
//   • masksToBounds = NO lets the shadow bleed beyond the layer's own bounds.
@interface CronymaxPopoverShadowView : NSView
@end
@implementation CronymaxPopoverShadowView
- (instancetype)initWithFrame:(NSRect)frame {
  self = [super initWithFrame:frame];
  if (self) {
    self.wantsLayer = YES;
    self.layer.backgroundColor = [NSColor clearColor].CGColor;
    self.layer.masksToBounds   = NO;
  }
  return self;
}
- (BOOL)isOpaque { return NO; }
- (void)dealloc { [self removeFromSuperview]; }
@end

// ---------------------------------------------------------------------------
// Popover scrim NSView
// ---------------------------------------------------------------------------
// Visually dims the main content panel while a popover is displayed and
// absorbs all pointer events so the underlying tab content is unreachable.
//
// WHY layer.backgroundColor DOES NOT WORK:
//   Plain NSView backing-layer fills live in the AppKit CA compositing tier.
//   CEF's GPU compositor places IOSurface-backed CALayers ABOVE the entire
//   AppKit CA layer tree, so any backgroundColor is composited below the
//   main-content IOSurface — permanently invisible.
//
// WHY CA shadowPath WORKS (same principle as CronymaxPopoverShadowView):
//   CA shadows are rasterized by WindowServer at compositing time, ABOVE
//   the IOSurface layers.  A CA shadow with:
//     shadowRadius  = 0   → no blur, sharp edges (fills exactly the path)
//     shadowOffset  = (0,0)
//     shadowOpacity = dim amount
//     shadowPath    = content-area rectangle
//   produces a solid semi-transparent dark rectangle rendered above the
//   main-tab IOSurface but below the popover overlay's IOSurface (because
//   the scrim NSView sits below the overlay root in z-order).
//
// Mouse impermeability: hitTest returns `self` for any point in bounds
// when the view has no subviews and is not hidden, regardless of the layer's
// visual content.  Events land on the scrim and are silently consumed.
@interface CronymaxPopoverScrimView : NSView
@end
@implementation CronymaxPopoverScrimView
- (instancetype)initWithFrame:(NSRect)frame {
  self = [super initWithFrame:frame];
  if (self) {
    self.wantsLayer = YES;
    self.layer.backgroundColor = [NSColor clearColor].CGColor;
    self.layer.masksToBounds   = NO;
  }
  return self;
}
- (BOOL)isOpaque { return NO; }
- (void)dealloc { [self removeFromSuperview]; }
@end

static char kPopoverShadowOwnerKey;
static char kPopoverScrimKey;
// Forward declaration so ShowPopoverScrim (defined earlier in the file) can
// reference the tag without depending on the definition order.
static constexpr NSInteger kCornerPunchTagFwd = 0x43524E58;  // "CRNX"

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

NSColor* ColorFromArgb(cef_color_t argb) {
  CGFloat a = ((argb >> 24) & 0xFF) / 255.0;
  CGFloat r = ((argb >> 16) & 0xFF) / 255.0;
  CGFloat g = ((argb >>  8) & 0xFF) / 255.0;
  CGFloat b = ((argb >>  0) & 0xFF) / 255.0;
  return [NSColor colorWithSRGBRed:r green:g blue:b alpha:a];
}

// Build a CGPath (in CALayer non-flipped coordinates) for a rectangle
// with per-corner rounding driven by a CACornerMask. Ownership: caller
// must CGPathRelease the returned path.
CGPathRef RoundedRectPathForLayer(CGRect r, CGFloat radius,
                                  CACornerMask corners) {
  // CA non-flipped: minY=bottom, maxY=top.
  const CGFloat blr = (corners & kCALayerMinXMinYCorner) ? radius : 0;  // BL
  const CGFloat brr = (corners & kCALayerMaxXMinYCorner) ? radius : 0;  // BR
  const CGFloat tlr = (corners & kCALayerMinXMaxYCorner) ? radius : 0;  // TL
  const CGFloat trr = (corners & kCALayerMaxXMaxYCorner) ? radius : 0;  // TR
  const CGFloat minX = CGRectGetMinX(r), maxX = CGRectGetMaxX(r);
  const CGFloat minY = CGRectGetMinY(r), maxY = CGRectGetMaxY(r);
  CGMutablePathRef p = CGPathCreateMutable();
  // Start: top-left after corner.
  CGPathMoveToPoint(p, NULL, minX + tlr, maxY);
  // Top edge → top-right corner.
  CGPathAddLineToPoint(p, NULL, maxX - trr, maxY);
  if (trr > 0) CGPathAddArcToPoint(p, NULL, maxX, maxY, maxX, maxY - trr, trr);
  else         CGPathAddLineToPoint(p, NULL, maxX, maxY);
  // Right edge → bottom-right corner.
  CGPathAddLineToPoint(p, NULL, maxX, minY + brr);
  if (brr > 0) CGPathAddArcToPoint(p, NULL, maxX, minY, maxX - brr, minY, brr);
  else         CGPathAddLineToPoint(p, NULL, maxX, minY);
  // Bottom edge → bottom-left corner.
  CGPathAddLineToPoint(p, NULL, minX + blr, minY);
  if (blr > 0) CGPathAddArcToPoint(p, NULL, minX, minY, minX, minY + blr, blr);
  else         CGPathAddLineToPoint(p, NULL, minX, minY);
  // Left edge → top-left corner.
  CGPathAddLineToPoint(p, NULL, minX, maxY - tlr);
  if (tlr > 0) CGPathAddArcToPoint(p, NULL, minX, maxY, minX + tlr, maxY, tlr);
  else         CGPathAddLineToPoint(p, NULL, minX, maxY);
  CGPathCloseSubpath(p);
  return p;
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

  const CACornerMask cm = ToCACornerMask(corner_mask);
  const CGFloat r = (CGFloat)radius;

  // Find the overlay root NSView — the topmost NSView in this overlay
  // widget's own view hierarchy (direct child of the main window's
  // contentView). CEF wraps the BrowserView in multiple intermediate NSViews
  // (BrowserView → ContentsView → Widget NSView) so we must find the root to
  // apply the mask there, ensuring ALL sublayers are clipped.
  NSView* windowContent = view.window ? view.window.contentView : nil;
  NSView* overlayRoot = view;
  {
    NSView* cur = view;
    while (cur.superview && cur.superview != windowContent) {
      cur = cur.superview;
    }
    overlayRoot = cur;
  }

  // Clear every intermediate CALayer background so the corner regions exposed
  // by the mask appear transparent rather than filled with opaque gray.
  for (NSView* anc = view;
       anc && anc != windowContent;
       anc = anc.superview) {
    anc.wantsLayer = YES;
    if (CALayer* al = anc.layer) {
      al.backgroundColor = [NSColor clearColor].CGColor;
      al.masksToBounds = NO;
    }
  }

  // Clip the overlay to rounded corners using a CAShapeLayer mask on the
  // overlay root layer.
  //
  // WHY NOT cornerRadius+masksToBounds:
  // Chromium's compositor manages its own IOSurface-backed CALayer subtree.
  // On macOS, cornerRadius+masksToBounds on the NSView's backing layer does
  // not reliably clip these IOSurface sublayers — they composite directly into
  // the parent bypassing the cornerRadius clip.  A layer.mask IS applied by
  // WindowServer at blend time to the full composited output of the layer
  // (including all IOSurface sublayers), so it works correctly.
  //
  // WHY masksToBounds = NO:
  // The content must be clipped to rounded corners (via the CAShapeLayer
  // mask below), but the layer's shadow must still bleed outside its bounds.
  // NOTE: CA has a known limitation — when a layer has both a `mask` AND a
  // `shadow`, the shadow is suppressed even with an explicit shadowPath.
  // We therefore apply the shadow on a SIBLING CALayer inserted behind
  // overlayRoot in the parent (windowContent.layer), so the mask and shadow
  // live on separate layers and both render correctly.
  overlayRoot.wantsLayer = YES;
  if (CALayer* rl = overlayRoot.layer) {
    rl.masksToBounds = NO;
    rl.shadowOpacity = 0.0f;  // shadow lives on the sibling layer, not here

    CAShapeLayer* shapeMask = [CAShapeLayer layer];
    CGPathRef maskPath = RoundedRectPathForLayer(rl.bounds, r, cm);
    shapeMask.path = maskPath;
    CGPathRelease(maskPath);
    rl.mask = shapeMask;
  }

  // Drop-shadow NSView: same frame as the overlay, shadow extends outward via
  // CA layer shadow properties.  See CronymaxPopoverShadowView above.
  if (with_shadow && windowContent) {
    CronymaxPopoverShadowView* shadowView =
        objc_getAssociatedObject(overlayRoot, &kPopoverShadowOwnerKey);

    if (!shadowView || shadowView.superview != windowContent) {
      if (shadowView) [shadowView removeFromSuperview];
      shadowView = [[CronymaxPopoverShadowView alloc]
                        initWithFrame:overlayRoot.frame];
      [windowContent addSubview:shadowView
                      positioned:NSWindowBelow
                      relativeTo:overlayRoot];
      objc_setAssociatedObject(overlayRoot, &kPopoverShadowOwnerKey,
                               shadowView, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }

    // Keep the shadow layer in sync with the overlay frame.
    shadowView.frame = overlayRoot.frame;

    if (CALayer* sl = shadowView.layer) {
      sl.masksToBounds = NO;
      sl.shadowOpacity = 0.55f;
      sl.shadowRadius  = 20.0f;
      sl.shadowOffset  = CGSizeMake(0, -6);
      sl.shadowColor   = [NSColor blackColor].CGColor;
      // shadowPath is in the layer's own (non-flipped) coordinate system.
      // Bounds origin is always {0,0}; size matches the frame dimensions.
      const CGRect sb = CGRectMake(0, 0,
                                   overlayRoot.frame.size.width,
                                   overlayRoot.frame.size.height);
      CGPathRef sp = CGPathCreateWithRoundedRect(sb, r, r, NULL);
      sl.shadowPath = sp;
      CGPathRelease(sp);
    }
  }
}

void ShowPopoverScrim(void* main_window_nsview_ptr,
                     int pop_x, int pop_y, int pop_w, int pop_h,
                     double corner_radius) {
  if (!main_window_nsview_ptr) return;
  NSView* root = (__bridge NSView*)main_window_nsview_ptr;
  NSView* wc = root.window ? root.window.contentView : root;
  if (!wc) return;

  CronymaxPopoverScrimView* scrim =
      objc_getAssociatedObject(wc, &kPopoverScrimKey);
  if (!scrim) {
    scrim = [[CronymaxPopoverScrimView alloc] initWithFrame:NSZeroRect];
    objc_setAssociatedObject(wc, &kPopoverScrimKey, scrim,
                             OBJC_ASSOCIATION_RETAIN_NONATOMIC);
  }

  // Insert the scrim as topmost.  Corner punch views (kCornerPunchTagFwd)
  // intentionally stay BELOW the scrim: the scrim covers the entire card
  // area, so punch views don't need to be visible while the scrim is shown.
  // Raising them above the scrim would make their bg-body-colored squares
  // visible as artifacts at the card corners on top of the overlay.
  // When the scrim is hidden (popover closed), punch views are naturally
  // the topmost non-scrim views and work correctly.
  [scrim removeFromSuperview];
  [wc addSubview:scrim positioned:NSWindowAbove relativeTo:nil];

  // Convert CEF coordinates (origin top-left, y down) to AppKit (origin
  // bottom-left, y up).  The contentView bounds height equals the full CEF
  // window height because the app uses NSWindowStyleMaskFullSizeContentView.
  NSRect wf = wc.bounds;
  NSRect f;
  f.origin.x    = (CGFloat)pop_x;
  f.size.width  = (CGFloat)pop_w;
  f.size.height = (CGFloat)pop_h;
  f.origin.y    = NSHeight(wf) - (CGFloat)pop_y - (CGFloat)pop_h;
  scrim.frame   = NSIntersectionRect(f, wf);  // clamp to visible window area
  scrim.hidden  = NO;

  if (CALayer* sl = scrim.layer) {
    sl.shadowOpacity  = 0.0f;
    sl.backgroundColor = [NSColor colorWithWhite:0 alpha:0.25f].CGColor;
    // Round the scrim corners to match the card corner radius so the
    // bg_body-colored corner punch views below show through at each corner,
    // preserving the rounded-card appearance while the overlay is visible.
    sl.cornerRadius   = (CGFloat)corner_radius;
    sl.masksToBounds  = (corner_radius > 0.0);
  }
}

void HidePopoverScrim(void* window_nsview_ptr) {
  if (!window_nsview_ptr) return;
  NSView* root = (__bridge NSView*)window_nsview_ptr;
  NSView* wc = root.window ? root.window.contentView : root;
  CronymaxPopoverScrimView* scrim =
      objc_getAssociatedObject(wc, &kPopoverScrimKey);
  if (scrim) [scrim removeFromSuperview];
}

void* CaptureLastChildNSView(void* main_nsview_ptr) {
  if (!main_nsview_ptr) return nullptr;
  NSView* contentView = (__bridge NSView*)main_nsview_ptr;
  NSWindow* mainWin = contentView.window;
  if (!mainWin) return nullptr;
  NSArray<NSWindow*>* children = mainWin.childWindows;
  if (children.count == 0) return nullptr;
  NSWindow* overlay = children.lastObject;
  NSView* overlayContent = overlay.contentView;
  if (!overlayContent) return nullptr;
  NSArray<NSView*>* subs = overlayContent.subviews;
  // Return the widget root NSView (direct subview of overlay contentView).
  // StyleOverlayBrowserView walks up from this to find the overlay root.
  return (__bridge void*)(subs.count > 0 ? subs[0] : overlayContent);
}

void StyleOverlayPanel(void* nsview_ptr,
                       double radius,
                       int corner_mask,
                       cef_color_t bg_color) {
  if (!nsview_ptr) return;
  NSView* view = (__bridge NSView*)nsview_ptr;

  // Walk up to find the overlay root (direct child of the overlay NSWindow's
  // contentView). This is the same traversal as StyleOverlayBrowserView.
  NSView* windowContent = view.window ? view.window.contentView : nil;
  NSView* overlayRoot = view;
  {
    NSView* cur = view;
    while (cur.superview && cur.superview != windowContent) {
      cur = cur.superview;
    }
    overlayRoot = cur;
  }

  // For AppKit-rendered CefPanel views the layer backgroundColor IS the
  // background — unlike BrowserView (IOSurface-backed) we must NOT clear it.
  overlayRoot.wantsLayer = YES;
  if (CALayer* rl = overlayRoot.layer) {
    const CGFloat a = ((bg_color >> 24) & 0xFF) / 255.0;
    const CGFloat r = ((bg_color >> 16) & 0xFF) / 255.0;
    const CGFloat g = ((bg_color >>  8) & 0xFF) / 255.0;
    const CGFloat b = ((bg_color      ) & 0xFF) / 255.0;
    rl.backgroundColor =
        [NSColor colorWithSRGBRed:r green:g blue:b alpha:a].CGColor;
    rl.cornerRadius   = (CGFloat)radius;
    rl.maskedCorners  = ToCACornerMask(corner_mask);
    rl.masksToBounds  = YES;
    rl.shadowOpacity  = 0.0f;  // no shadow on the chrome strip
  }
}

void SetOverlayWindowBackground(void* nsview_ptr, cef_color_t argb) {
  if (!nsview_ptr) return;
  NSView* view = (__bridge NSView*)nsview_ptr;
  NSWindow* w = view.window;
  if (!w) return;
  const CGFloat a = ((argb >> 24) & 0xFF) / 255.0;
  const CGFloat r = ((argb >> 16) & 0xFF) / 255.0;
  const CGFloat g = ((argb >>  8) & 0xFF) / 255.0;
  const CGFloat b = ((argb      ) & 0xFF) / 255.0;
  w.backgroundColor = [NSColor colorWithSRGBRed:r green:g blue:b alpha:a];
  w.opaque = (a >= 0.999);
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

void StyleMainWindowTranslucent(void* nswindow_ptr, cef_color_t argb) {
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

  // Solid opaque chrome — NO NSVisualEffectView. Vibrancy under the
  // AppKit titlebar zone reads visibly different from vibrancy under the
  // body region; a flat opaque color guarantees a single uniform chrome.
  // refine-ui-theme-layout: caller threads the active chrome color in;
  // 0 falls back to the legacy dark default.
  NSColor* chromeColor =
      argb == 0
          ? [NSColor colorWithSRGBRed:0x14 / 255.0
                                green:0x14 / 255.0
                                 blue:0x1A / 255.0
                                alpha:1.0]
          : ColorFromArgb(argb);
  window.opaque = YES;
  window.backgroundColor = chromeColor;
  window.hasShadow = YES;

  content.wantsLayer = YES;
  if (CALayer* cl = content.layer) {
    cl.cornerRadius = 12.0;
    cl.masksToBounds = YES;
    cl.backgroundColor = chromeColor.CGColor;
  }

  // Vertically center the traffic-light buttons in our 38 pt custom titlebar.
  // This must be deferred to the NEXT run-loop cycle: the style-mask changes
  // above trigger an AppKit layout pass that repositions the buttons to their
  // natural positions; if we set frames synchronously here, that layout pass
  // runs afterwards and overwrites us.
  //
  // We use ABSOLUTE positioning rather than a shift relative to the native
  // titlebar height.  The shift-based approach breaks when
  // titlebarAppearsTransparent=YES causes contentLayoutRect to span the full
  // window (natH≈0 → early return), and the shift sign is wrong for the
  // non-flipped container coordinate system used by _NSTitlebarContainerView.
  // Absolute centering: origin.y = (kTitleBarH − buttonH) / 2 sets the
  // button in the exact visual centre of the 38 pt area regardless of whether
  // the container is flipped or non-flipped.
  dispatch_async(dispatch_get_main_queue(), ^{
    // Block retains |window| in MRC — safe since the window outlives this tick.
    NSWindow* w = window;
    if (!w) return;
    const CGFloat kTitleBarH = 38.0;

    NSButton* btns[3] = {
        [w standardWindowButton:NSWindowCloseButton],
        [w standardWindowButton:NSWindowMiniaturizeButton],
        [w standardWindowButton:NSWindowZoomButton],
    };

    // Find _NSTitlebarContainerView (direct child of _NSThemeFrame that
    // owns the traffic-light buttons).  Expand it to kTitleBarH if needed
    // so the buttons have room to be centred.
    NSView* themeFrame = w.contentView.superview;
    NSView* container  = nil;
    if (btns[0]) {
      NSView* v = btns[0].superview;
      while (v && v.superview && v.superview != themeFrame) v = v.superview;
      if (v && v.superview == themeFrame) container = v;
    }
    if (container && NSHeight(container.bounds) < kTitleBarH) {
      [container setTranslatesAutoresizingMaskIntoConstraints:YES];
      NSRect cf = container.frame;
      CGFloat extra = kTitleBarH - cf.size.height;
      cf.origin.y    -= extra;
      cf.size.height  = kTitleBarH;
      container.frame = cf;
      container.autoresizingMask = NSViewWidthSizable | NSViewMinYMargin;
    }

    for (int i = 0; i < 3; ++i) {
      NSButton* btn = btns[i];
      if (!btn) continue;
      [btn setTranslatesAutoresizingMaskIntoConstraints:YES];
      NSRect f = btn.frame;
      // Centre the button vertically in the kTitleBarH container.
      // This formula is correct for both flipped and non-flipped containers:
      // it positions origin.y so that the button's centre lands at kTitleBarH/2.
      f.origin.y = (kTitleBarH - NSHeight(f)) / 2.0;
      btn.frame = f;
    }
  });
}

}  // namespace cronymax

// Solid NSView placed at each corner of the floating card.
// It paints the window chrome color, then cuts a quarter-circle via a
// CAShapeLayer mask so the card's corner appears rounded.
// Which corner: 0=BL 1=BR 2=TR 3=TL  (NSView y=0 at bottom, not flipped).
@interface CronymaxCornerPunchView : NSView {
  NSInteger _tag;
}
@property(nonatomic, assign) int punchCorner;
@property(nonatomic, assign) CGFloat punchRadius;
@property(nonatomic, strong) NSColor* punchColor;
- (void)setTag:(NSInteger)tag;
- (NSInteger)tag;
@end

@implementation CronymaxCornerPunchView
- (void)setTag:(NSInteger)t { _tag = t; }
- (NSInteger)tag { return _tag; }
- (BOOL)mouseDownCanMoveWindow { return NO; }
- (BOOL)wantsUpdateLayer { return YES; }
- (BOOL)wantsLayer { return YES; }
- (void)updateLayer {
  self.layer.backgroundColor = self.punchColor
      ? self.punchColor.CGColor
      : NSColor.blackColor.CGColor;
  // Install a circular cutout via CAShapeLayer mask.
  CGFloat s  = self.bounds.size.width;   // width == height == radius
  CGFloat r  = self.punchRadius;
  CGMutablePathRef path = CGPathCreateMutable();
  // Full square.
  CGPathAddRect(path, NULL, CGRectMake(0, 0, s, s));
  // Subtract a quarter-circle whose center is at the inward corner.
  // punchCorner: 0=BL,1=BR,2=TR,3=TL in NSView (y=0 at bottom).
  // In CALayer (y=0 at bottom, same as NSView on non-flipped view):
  CGPoint center;
  switch (self.punchCorner) {
    case 0:  center = CGPointMake(s, s); break;  // BL → arc center at BR of patch
    case 1:  center = CGPointMake(0, s); break;  // BR → arc center at BL of patch
    case 2:  center = CGPointMake(0, 0); break;  // TR → arc center at TL of patch
    case 3:  center = CGPointMake(s, 0); break;  // TL → arc center at TR of patch
    default: center = CGPointMake(0, 0); break;
  }
  CGPathAddArc(path, NULL, center.x, center.y, r,
               0, 2 * M_PI, 0);  // Full circle, but only r-sized view is clipped
  // Use even-odd fill rule to cut the circle from the square.
  CAShapeLayer* mask = [CAShapeLayer layer];
  mask.path = path;
  mask.fillRule = kCAFillRuleEvenOdd;
  self.layer.mask = mask;
  CGPathRelease(path);
}
@end

namespace cronymax {

// A tag value so we can find and remove previously installed punch views.
static constexpr NSInteger kCornerPunchTag = 0x43524E58;  // "CRNX"

void StyleContentBrowserView(void* window_nsview_ptr,
                             double radius,
                             cef_color_t bg_argb,
                             const CefRect& card_rect) {
  if (!window_nsview_ptr) return;
  NSView* root = (__bridge NSView*)window_nsview_ptr;

  // Remove any previously installed punch views.
  NSMutableArray* old = [NSMutableArray array];
  for (NSView* sv in root.subviews) {
    if (sv.tag == kCornerPunchTag) [old addObject:sv];
  }
  for (NSView* sv in old) [sv removeFromSuperview];

  // card_rect is in Chromium/CefRect coordinates: y grows down, y=0 at top
  // of the window content area. NSView default (non-flipped): y=0 at bottom.
  CGFloat rootH  = root.bounds.size.height;
  CGFloat cardX  = card_rect.x;
  CGFloat cardY  = card_rect.y;      // y from top
  CGFloat cardW  = card_rect.width;
  CGFloat cardH  = card_rect.height;
  CGFloat r      = (CGFloat)radius;

  // Build fill color.
  NSColor* fill = ColorFromArgb(bg_argb);

  // 4 corner positions in NSView (y=0 at bottom) coordinates:
  // Bottom-left  (NSView): (cardX, rootH - cardY - cardH)
  // Bottom-right (NSView): (cardX + cardW - r, rootH - cardY - cardH)
  // Top-right    (NSView): (cardX + cardW - r, rootH - cardY - r)
  // Top-left     (NSView): (cardX,             rootH - cardY - r)
  CGFloat nsCardBottom = rootH - cardY - cardH;  // y=0 at bottom
  CGFloat nsCardTop    = rootH - cardY;           // y=0 at bottom, top edge

  struct { CGFloat x, y; int corner; } patches[4] = {
    { cardX,              nsCardBottom,     0 },  // BL
    { cardX + cardW - r,  nsCardBottom,     1 },  // BR
    { cardX + cardW - r,  nsCardTop    - r, 2 },  // TR
    { cardX,              nsCardTop    - r, 3 },  // TL
  };

  for (int i = 0; i < 4; i++) {
    CronymaxCornerPunchView* v = [[CronymaxCornerPunchView alloc] init];
    v.punchColor  = fill;
    v.punchCorner = patches[i].corner;
    v.punchRadius = r;
    v.tag         = kCornerPunchTag;
    v.frame       = NSMakeRect(patches[i].x, patches[i].y, r, r);
    [root addSubview:v];
  }

  // Punch views are inserted via addSubview: which places them topmost.
  // If a scrim is already present (popover is open), re-raise it above the
  // newly added punch views so punch views stay below the scrim.
  // Both the scrim and punch views live in root (the main window contentView).
  CronymaxPopoverScrimView* existingScrim =
      objc_getAssociatedObject(root, &kPopoverScrimKey);
  if (existingScrim && existingScrim.superview == root) {
    [existingScrim removeFromSuperview];
    [root addSubview:existingScrim positioned:NSWindowAbove relativeTo:nil];
  }
}

void AddContentCardShadow(void* bv_nsview_ptr) {
  if (!bv_nsview_ptr) return;
  NSView* view = (__bridge NSView*)bv_nsview_ptr;
  // Shadow is placed on the BrowserView's host (superview) so it can
  // bleed outside the clipped layer area and appear around the card edge.
  NSView* host = view.superview;
  if (!host) return;
  host.wantsLayer = YES;
  if (CALayer* hl = host.layer) {
    hl.masksToBounds = NO;
    hl.shadowColor = [NSColor blackColor].CGColor;
    hl.shadowOpacity = 0.28f;
    hl.shadowRadius = 22.0f;
    hl.shadowOffset = CGSizeMake(0, -6);
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

// ---------------------------------------------------------------------------
// refine-ui-theme-layout: live theme application helpers
// ---------------------------------------------------------------------------

void SetMainWindowBackgroundColor(void* nswindow_ptr, cef_color_t argb) {
  if (!nswindow_ptr) return;
  NSView* content = (__bridge NSView*)nswindow_ptr;
  NSWindow* window = content.window;
  if (!window) return;
  NSColor* color = ColorFromArgb(argb);
  window.backgroundColor = color;
  if (CALayer* cl = content.layer) {
    cl.backgroundColor = color.CGColor;
  }
}

void SetAppAppearance(bool dark) {
  if (@available(macOS 10.14, *)) {
    NSAppearanceName name = dark ? NSAppearanceNameDarkAqua : NSAppearanceNameAqua;
    NSApp.appearance = [NSAppearance appearanceNamed:name];
  }
}

void InstallRoundedFrame(void* nsview_ptr,
                         double radius,
                         cef_color_t border_argb) {
  if (!nsview_ptr) return;
  NSView* view = (__bridge NSView*)nsview_ptr;
  view.wantsLayer = YES;
  if (CALayer* layer = view.layer) {
    layer.cornerRadius = radius;
    layer.maskedCorners =
        kCALayerMinXMinYCorner | kCALayerMaxXMinYCorner |
        kCALayerMinXMaxYCorner | kCALayerMaxXMaxYCorner;
    layer.masksToBounds = YES;
    layer.borderWidth = 1.0;
    layer.borderColor = ColorFromArgb(border_argb).CGColor;
  }
}

const char* CurrentSystemAppearance() {
  if (@available(macOS 10.14, *)) {
    NSAppearance* appearance = NSApp.effectiveAppearance;
    NSAppearanceName best = [appearance
        bestMatchFromAppearancesWithNames:@[ NSAppearanceNameAqua,
                                              NSAppearanceNameDarkAqua ]];
    if ([best isEqualToString:NSAppearanceNameDarkAqua]) return "dark";
  }
  return "light";
}

void* AddSystemAppearanceObserver(void (*on_changed)(void* user), void* user) {
  if (!on_changed) return nullptr;
  // AppleInterfaceThemeChangedNotification fires on the
  // NSDistributedNotificationCenter when System Settings toggles
  // Light/Dark. Run the callback on the main queue so MainWindow can
  // safely re-post onto TID_UI.
  id token = [[NSDistributedNotificationCenter defaultCenter]
      addObserverForName:@"AppleInterfaceThemeChangedNotification"
                  object:nil
                   queue:[NSOperationQueue mainQueue]
              usingBlock:^(NSNotification* /*note*/) {
                on_changed(user);
              }];
  // Retain the observer token across the bridge.
  return (__bridge_retained void*)token;
}

void RemoveSystemAppearanceObserver(void* token) {
  if (!token) return;
  id obs = (__bridge_transfer id)token;
  [[NSDistributedNotificationCenter defaultCenter] removeObserver:obs];
}

}  // namespace cronymax
