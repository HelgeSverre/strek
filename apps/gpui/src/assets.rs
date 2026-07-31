//! Embedded assets used by the GPUI frontend.

use std::borrow::Cow;

use gpui::{svg, AssetSource, Hsla, SharedString, Styled, Svg};

const ICONS: &[(&str, &[u8])] = &[
    ("icons/menu.svg", include_bytes!("../assets/icons/menu.svg")),
    (
        "icons/pointer.svg",
        include_bytes!("../assets/icons/pointer.svg"),
    ),
    (
        "icons/frame.svg",
        include_bytes!("../assets/icons/frame.svg"),
    ),
    (
        "icons/rectangle.svg",
        include_bytes!("../assets/icons/rectangle.svg"),
    ),
    (
        "icons/ellipse.svg",
        include_bytes!("../assets/icons/ellipse.svg"),
    ),
    ("icons/pen.svg", include_bytes!("../assets/icons/pen.svg")),
    ("icons/text.svg", include_bytes!("../assets/icons/text.svg")),
    ("icons/undo.svg", include_bytes!("../assets/icons/undo.svg")),
    ("icons/redo.svg", include_bytes!("../assets/icons/redo.svg")),
    (
        "icons/panel-right.svg",
        include_bytes!("../assets/icons/panel-right.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
    ("icons/eye.svg", include_bytes!("../assets/icons/eye.svg")),
    (
        "icons/eye-off.svg",
        include_bytes!("../assets/icons/eye-off.svg"),
    ),
    ("icons/lock.svg", include_bytes!("../assets/icons/lock.svg")),
    (
        "icons/unlock.svg",
        include_bytes!("../assets/icons/unlock.svg"),
    ),
    (
        "icons/layers.svg",
        include_bytes!("../assets/icons/layers.svg"),
    ),
    (
        "icons/group.svg",
        include_bytes!("../assets/icons/group.svg"),
    ),
    (
        "icons/align-left.svg",
        include_bytes!("../assets/icons/align-left.svg"),
    ),
    (
        "icons/align-center.svg",
        include_bytes!("../assets/icons/align-center.svg"),
    ),
    (
        "icons/align-right.svg",
        include_bytes!("../assets/icons/align-right.svg"),
    ),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find_map(|(asset_path, bytes)| (*asset_path == path).then_some(Cow::Borrowed(*bytes))))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(asset_path, _)| asset_path.starts_with(path))
            .map(|(asset_path, _)| SharedString::from(*asset_path))
            .collect())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Menu,
    Pointer,
    Frame,
    Rectangle,
    Ellipse,
    Pen,
    Text,
    Undo,
    Redo,
    PanelRight,
    ChevronDown,
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Layers,
    Group,
    AlignLeft,
    AlignCenter,
    AlignRight,
}

impl Icon {
    fn path(self) -> &'static str {
        match self {
            Self::Menu => "icons/menu.svg",
            Self::Pointer => "icons/pointer.svg",
            Self::Frame => "icons/frame.svg",
            Self::Rectangle => "icons/rectangle.svg",
            Self::Ellipse => "icons/ellipse.svg",
            Self::Pen => "icons/pen.svg",
            Self::Text => "icons/text.svg",
            Self::Undo => "icons/undo.svg",
            Self::Redo => "icons/redo.svg",
            Self::PanelRight => "icons/panel-right.svg",
            Self::ChevronDown => "icons/chevron-down.svg",
            Self::Eye => "icons/eye.svg",
            Self::EyeOff => "icons/eye-off.svg",
            Self::Lock => "icons/lock.svg",
            Self::Unlock => "icons/unlock.svg",
            Self::Layers => "icons/layers.svg",
            Self::Group => "icons/group.svg",
            Self::AlignLeft => "icons/align-left.svg",
            Self::AlignCenter => "icons/align-center.svg",
            Self::AlignRight => "icons/align-right.svg",
        }
    }
}

pub fn icon(kind: Icon, size: f32, color: impl Into<Hsla>) -> Svg {
    svg()
        .path(kind.path())
        .w(gpui::px(size))
        .h(gpui::px(size))
        .text_color(color)
}
