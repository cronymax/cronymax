/// A rectangular region in the window (in physical pixels).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    /// Create panel bounds from physical pixel values.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Create a viewport from window dimensions, applying default 4px padding.
    pub fn from_window_size(width: u32, height: u32) -> Self {
        const PADDING: f32 = 4.0;
        Self {
            x: PADDING,
            y: PADDING,
            width: (width as f32 - 2.0 * PADDING).max(0.0),
            height: (height as f32 - 2.0 * PADDING).max(0.0),
        }
    }

    /// Convert to a wry Rect for set_bounds().
    pub fn to_wry_rect(self) -> wry::Rect {
        wry::Rect {
            position: wry::dpi::PhysicalPosition::new(self.x, self.y).into(),
            size: wry::dpi::PhysicalSize::new(self.width, self.height).into(),
        }
    }

    pub fn shrink(self, padding: f32) -> Self {
        Self {
            x: self.x + padding,
            y: self.y + padding,
            width: (self.width - 2.0 * padding).max(0.0),
            height: (self.height - 2.0 * padding).max(0.0),
        }
    }

    pub fn expand(self, padding: f32) -> Self {
        Self {
            x: self.x - padding,
            y: self.y - padding,
            width: (self.width + 2.0 * padding).max(0.0),
            height: (self.height + 2.0 * padding).max(0.0),
        }
    }
}
