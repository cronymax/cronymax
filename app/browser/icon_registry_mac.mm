// Copyright (c) 2026.
//
// macOS implementation of the IconRegistry platform bridge.
//
// RasterizeIconSvg  — loads SVG bytes via NSImage (requires macOS 13+),
//   renders into a CGBitmapContext at (logical_size × scale_factor) pixels,
//   tints the glyph with kCGBlendModeSourceIn, and wraps the result in a
//   CefImage via AddBitmap.
//
// GetPrimaryDisplayScale  — reads NSScreen.mainScreen.backingScaleFactor.

#include "browser/icon_registry.h"

#include <string_view>

#include "include/cef_image.h"
#include "include/wrapper/cef_helpers.h"

#import <AppKit/AppKit.h>
#import <CoreGraphics/CoreGraphics.h>

namespace cronymax {

CefRefPtr<CefImage> RasterizeIconSvg(std::string_view svg_data,
                                     int logical_size,
                                     float scale_factor,
                                     float r,
                                     float g,
                                     float b) {
  @autoreleasepool {
    NSData* data = [NSData dataWithBytes:svg_data.data()
                                  length:svg_data.size()];
    NSImage* img = [[NSImage alloc] initWithData:data];
    if (!img) {
      LOG(ERROR)
          << "IconRegistry: NSImage failed to load SVG from embedded data";
      return nullptr;
    }
    [img setSize:NSMakeSize(logical_size, logical_size)];

    const size_t pix_w = static_cast<size_t>(logical_size * scale_factor);
    const size_t pix_h = pix_w;
    const size_t row_bytes = pix_w * 4;
    const size_t data_size = row_bytes * pix_h;

    // Use native macOS format: ARGB with host byte order.
    // On little-endian (all modern Macs) this stores bytes as [B,G,R,A]
    // in memory, which matches CEF_COLOR_TYPE_BGRA_8888.
    // Using the native format ensures CoreGraphics renders the NSImage
    // correctly (non-native byte orders may silently fail to paint).
    CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
    CGContextRef ctx = CGBitmapContextCreate(
        nullptr, pix_w, pix_h, 8, row_bytes, cs,
        static_cast<uint32_t>(kCGImageAlphaPremultipliedFirst) |
            static_cast<uint32_t>(kCGBitmapByteOrder32Host));
    CGColorSpaceRelease(cs);
    if (!ctx) {
      LOG(ERROR) << "IconRegistry: failed to create CGBitmapContext";
      return nullptr;
    }

    NSGraphicsContext* gctx =
        [NSGraphicsContext graphicsContextWithCGContext:ctx flipped:NO];
    [NSGraphicsContext saveGraphicsState];
    [NSGraphicsContext setCurrentContext:gctx];

    // 1. Draw the SVG — Codicons render black by default.
    NSRect dst = NSMakeRect(0, 0, pix_w, pix_h);
    [img drawInRect:dst
           fromRect:NSZeroRect
          operation:NSCompositingOperationCopy
           fraction:1.0];

    // 2. Tint: replace the drawn RGB with the desired colour, preserving alpha.
    CGContextSetBlendMode(ctx, kCGBlendModeSourceIn);
    CGContextSetRGBFillColor(ctx, r, g, b, 1.0f);
    CGContextFillRect(ctx, CGRectMake(0, 0, pix_w, pix_h));

    [NSGraphicsContext restoreGraphicsState];

    const void* pixels = CGBitmapContextGetData(ctx);
    if (!pixels) {
      CGContextRelease(ctx);
      LOG(ERROR) << "IconRegistry: CGBitmapContextGetData returned null";
      return nullptr;
    }

    CefRefPtr<CefImage> cef_image = CefImage::CreateImage();
    bool ok =
        cef_image->AddBitmap(scale_factor, static_cast<int>(pix_w),
                             static_cast<int>(pix_h), CEF_COLOR_TYPE_BGRA_8888,
                             CEF_ALPHA_TYPE_PREMULTIPLIED, pixels, data_size);
    CGContextRelease(ctx);
    if (!ok) {
      LOG(ERROR) << "IconRegistry: CefImage::AddBitmap failed";
      return nullptr;
    }
    return cef_image;
  }
}

float GetPrimaryDisplayScale() {
  @autoreleasepool {
    NSScreen* main = [NSScreen mainScreen];
    return main ? static_cast<float>(main.backingScaleFactor) : 1.0f;
  }
}

}  // namespace cronymax
