//! Platform-agnostic input events.

use glam::Vec2;

/// Modifier keys state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool, // Cmd on Mac, Win on Windows
}

impl Modifiers {
    /// Check if any modifier is pressed.
    pub fn any(&self) -> bool {
        self.shift || self.ctrl || self.alt || self.meta
    }

    /// Check if the primary modifier is pressed (Ctrl on Windows/Linux, Cmd on Mac).
    /// For cross-platform shortcuts like Ctrl+Z / Cmd+Z.
    pub fn primary(&self) -> bool {
        // In a real app, this would be platform-dependent
        // For now, treat both ctrl and meta as primary
        self.ctrl || self.meta
    }
}

/// Mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// Input event from the platform.
#[derive(Debug, Clone)]
pub enum InputEvent {
    /// Pointer/mouse button pressed
    PointerDown {
        position: Vec2,
        button: MouseButton,
        modifiers: Modifiers,
    },

    /// Pointer/mouse moved
    PointerMove {
        position: Vec2,
        modifiers: Modifiers,
    },

    /// Pointer/mouse button released
    PointerUp {
        position: Vec2,
        button: MouseButton,
        modifiers: Modifiers,
    },

    /// Scroll/wheel event
    Scroll {
        position: Vec2,
        delta: Vec2,
        modifiers: Modifiers,
    },

    /// Key pressed
    KeyDown { key: Key, modifiers: Modifiers },

    /// Key released
    KeyUp { key: Key, modifiers: Modifiers },

    /// Modifier-key state changed without pointer movement.
    ModifiersChanged { modifiers: Modifiers },
}

/// Keyboard key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    // Letters
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,

    // Numbers
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,

    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    // Navigation
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,

    // Editing
    Backspace,
    Delete,
    Enter,
    Tab,
    Space,
    Escape,

    // Punctuation
    BracketLeft,  // [
    BracketRight, // ]

    // Modifiers (usually handled via Modifiers struct, but useful for detection)
    Shift,
    Control,
    Alt,
    Meta,

    // Other
    Unknown,
}

impl Key {
    /// Parse a key from a JavaScript key string.
    pub fn from_js_key(key: &str) -> Self {
        match key {
            "a" | "A" => Key::A,
            "b" | "B" => Key::B,
            "c" | "C" => Key::C,
            "d" | "D" => Key::D,
            "e" | "E" => Key::E,
            "f" | "F" => Key::F,
            "g" | "G" => Key::G,
            "h" | "H" => Key::H,
            "i" | "I" => Key::I,
            "j" | "J" => Key::J,
            "k" | "K" => Key::K,
            "l" | "L" => Key::L,
            "m" | "M" => Key::M,
            "n" | "N" => Key::N,
            "o" | "O" => Key::O,
            "p" | "P" => Key::P,
            "q" | "Q" => Key::Q,
            "r" | "R" => Key::R,
            "s" | "S" => Key::S,
            "t" | "T" => Key::T,
            "u" | "U" => Key::U,
            "v" | "V" => Key::V,
            "w" | "W" => Key::W,
            "x" | "X" => Key::X,
            "y" | "Y" => Key::Y,
            "z" | "Z" => Key::Z,
            "0" => Key::Digit0,
            "1" => Key::Digit1,
            "2" => Key::Digit2,
            "3" => Key::Digit3,
            "4" => Key::Digit4,
            "5" => Key::Digit5,
            "6" => Key::Digit6,
            "7" => Key::Digit7,
            "8" => Key::Digit8,
            "9" => Key::Digit9,
            "F1" => Key::F1,
            "F2" => Key::F2,
            "F3" => Key::F3,
            "F4" => Key::F4,
            "F5" => Key::F5,
            "F6" => Key::F6,
            "F7" => Key::F7,
            "F8" => Key::F8,
            "F9" => Key::F9,
            "F10" => Key::F10,
            "F11" => Key::F11,
            "F12" => Key::F12,
            "ArrowUp" => Key::ArrowUp,
            "ArrowDown" => Key::ArrowDown,
            "ArrowLeft" => Key::ArrowLeft,
            "ArrowRight" => Key::ArrowRight,
            "Home" => Key::Home,
            "End" => Key::End,
            "PageUp" => Key::PageUp,
            "PageDown" => Key::PageDown,
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            "Enter" => Key::Enter,
            "Tab" => Key::Tab,
            " " => Key::Space,
            "Escape" => Key::Escape,
            "[" => Key::BracketLeft,
            "]" => Key::BracketRight,
            "Shift" => Key::Shift,
            "Control" => Key::Control,
            "Alt" => Key::Alt,
            "Meta" => Key::Meta,
            _ => Key::Unknown,
        }
    }
}

/// Output effects from event handling.
#[derive(Debug, Default)]
pub struct Effects {
    /// Request a redraw
    pub redraw: bool,

    /// Change cursor style
    pub cursor: Option<Cursor>,
}

impl Effects {
    /// Create effects requesting a redraw.
    pub fn redraw() -> Self {
        Self {
            redraw: true,
            cursor: None,
        }
    }

    /// Merge another effects into this one.
    pub fn merge(&mut self, other: Effects) {
        self.redraw = self.redraw || other.redraw;
        if other.cursor.is_some() {
            self.cursor = other.cursor;
        }
    }
}

/// Cursor style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Default,
    Pointer,
    Move,
    Crosshair,
    Text,
    Grab,
    Grabbing,
    ResizeNS,
    ResizeEW,
    ResizeNESW,
    ResizeNWSE,
}

impl Cursor {
    /// Convert to CSS cursor value.
    pub fn to_css(&self) -> &'static str {
        match self {
            Cursor::Default => "default",
            Cursor::Pointer => "pointer",
            Cursor::Move => "move",
            Cursor::Crosshair => "crosshair",
            Cursor::Text => "text",
            Cursor::Grab => "grab",
            Cursor::Grabbing => "grabbing",
            Cursor::ResizeNS => "ns-resize",
            Cursor::ResizeEW => "ew-resize",
            Cursor::ResizeNESW => "nesw-resize",
            Cursor::ResizeNWSE => "nwse-resize",
        }
    }
}
