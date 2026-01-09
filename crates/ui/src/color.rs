//! Color type for UI rendering.

/// RGBA color with components in 0.0-1.0 range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// Create a new color from RGBA components (0.0-1.0).
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Create a new opaque color from RGB components (0.0-1.0).
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Create a color from 8-bit RGBA components (0-255).
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// Create a color from a hex value (0xRRGGBB or 0xRRGGBBAA).
    pub fn from_hex(hex: u32) -> Self {
        if hex > 0xFFFFFF {
            // Has alpha
            Self::from_rgba8(
                ((hex >> 24) & 0xFF) as u8,
                ((hex >> 16) & 0xFF) as u8,
                ((hex >> 8) & 0xFF) as u8,
                (hex & 0xFF) as u8,
            )
        } else {
            // No alpha
            Self::from_rgba8(
                ((hex >> 16) & 0xFF) as u8,
                ((hex >> 8) & 0xFF) as u8,
                (hex & 0xFF) as u8,
                255,
            )
        }
    }

    /// Transparent (fully invisible).
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    /// Black.
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);

    /// White.
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);

    /// Create a grayscale color.
    pub const fn gray(value: f32) -> Self {
        Self::rgb(value, value, value)
    }

    /// Return this color with a different alpha.
    pub const fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Convert to an array [r, g, b, a].
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Linearly interpolate between two colors.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

impl From<[f32; 4]> for Color {
    fn from(arr: [f32; 4]) -> Self {
        Self::new(arr[0], arr[1], arr[2], arr[3])
    }
}

impl From<[f32; 3]> for Color {
    fn from(arr: [f32; 3]) -> Self {
        Self::rgb(arr[0], arr[1], arr[2])
    }
}
