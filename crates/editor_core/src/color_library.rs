//! Document-local, copy-by-value saved colors.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

/// Maximum number of color groups accepted in a document.
pub const MAX_COLOR_GROUPS: usize = 256;
/// Maximum number of saved colors accepted in a document.
pub const MAX_SAVED_COLORS: usize = 10_000;
/// Maximum UTF-8 byte length of a group or color name.
pub const MAX_COLOR_NAME_BYTES: usize = 256;

macro_rules! stable_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            /// Return the persisted numeric representation.
            pub const fn get(self) -> u64 {
                self.0
            }

            /// Reconstruct a non-zero ID received from persistence or automation.
            pub const fn from_opaque(value: u64) -> Option<Self> {
                if value == 0 {
                    None
                } else {
                    Some(Self(value))
                }
            }

            pub(crate) const fn from_raw(raw: u64) -> Self {
                Self(raw)
            }
        }
    };
}

stable_id!(ColorGroupId);
stable_id!(SavedColorId);

/// A normalized sRGB color with alpha.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RgbaColor([f32; 4]);

impl RgbaColor {
    /// Construct a color whose components are all finite and in `0.0..=1.0`.
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Result<Self, ColorLibraryError> {
        Self::from_array([r, g, b, a])
    }

    /// Construct a color from normalized RGBA components.
    pub fn from_array(components: [f32; 4]) -> Result<Self, ColorLibraryError> {
        if components
            .iter()
            .all(|component| component.is_finite() && (0.0..=1.0).contains(component))
        {
            Ok(Self(components))
        } else {
            Err(ColorLibraryError::InvalidColor)
        }
    }

    /// Return normalized RGBA components.
    pub const fn components(self) -> [f32; 4] {
        self.0
    }

    /// Produce the uppercase display label used for unnamed colors.
    pub fn hex_label(self) -> String {
        let [r, g, b, a] = self.to_bytes();
        if a == u8::MAX {
            format!("#{r:02X}{g:02X}{b:02X}")
        } else {
            format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
        }
    }

    pub(crate) fn validate(self) -> Result<(), ColorLibraryError> {
        Self::from_array(self.0).map(|_| ())
    }

    fn to_bytes(self) -> [u8; 4] {
        self.0.map(|component| (component * 255.0).round() as u8)
    }
}

/// Ordering mode for colors inside a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSortMode {
    #[default]
    Manual,
    Name,
    HueAndShades,
    Lightness,
    Chroma,
    Brightness,
}

/// Direction applied to a group's ordering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

/// An ordered organizational section in the document color library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorGroup {
    pub id: ColorGroupId,
    pub name: String,
    pub manual_order: u32,
    pub sort_mode: ColorSortMode,
    pub sort_direction: SortDirection,
}

/// One reusable, unlinked solid color value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedColor {
    pub id: SavedColorId,
    pub group_id: Option<ColorGroupId>,
    pub name: Option<String>,
    pub rgba: RgbaColor,
    pub manual_order: u32,
}

/// Complete document-local collection of groups and saved colors.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ColorLibrary {
    pub groups: Vec<ColorGroup>,
    pub colors: Vec<SavedColor>,
}

impl ColorLibrary {
    /// Find a group by stable ID.
    pub fn group(&self, id: ColorGroupId) -> Option<&ColorGroup> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// Find a saved color by stable ID.
    pub fn color(&self, id: SavedColorId) -> Option<&SavedColor> {
        self.colors.iter().find(|color| color.id == id)
    }

    /// Return colors in the authored display order for a group or Ungrouped.
    pub fn colors_in_group(&self, group_id: Option<ColorGroupId>) -> Vec<&SavedColor> {
        let mut colors = self
            .colors
            .iter()
            .filter(|color| color.group_id == group_id)
            .collect::<Vec<_>>();
        let (mode, direction) = group_id
            .and_then(|id| self.group(id))
            .map_or((ColorSortMode::Manual, SortDirection::Ascending), |group| {
                (group.sort_mode, group.sort_direction)
            });
        colors.sort_by(|left, right| compare_colors(left, right, mode, direction));
        colors
    }

    /// Find an exact RGBA match, including alpha.
    pub fn find_exact(&self, rgba: RgbaColor) -> Option<&SavedColor> {
        self.colors.iter().find(|color| color.rgba == rgba)
    }

    pub(crate) fn validate(&self) -> Result<(), ColorLibraryError> {
        if self.groups.len() > MAX_COLOR_GROUPS {
            return Err(ColorLibraryError::TooManyGroups);
        }
        if self.colors.len() > MAX_SAVED_COLORS {
            return Err(ColorLibraryError::TooManyColors);
        }

        let mut group_ids = std::collections::HashSet::with_capacity(self.groups.len());
        for group in &self.groups {
            if group.id.get() == 0 {
                return Err(ColorLibraryError::InvalidGroupId);
            }
            if !group_ids.insert(group.id) {
                return Err(ColorLibraryError::DuplicateGroupId(group.id));
            }
            validate_required_name(&group.name)?;
        }
        validate_contiguous_orders(self.groups.iter().map(|group| group.manual_order))?;

        let mut color_ids = std::collections::HashSet::with_capacity(self.colors.len());
        for color in &self.colors {
            if color.id.get() == 0 {
                return Err(ColorLibraryError::InvalidColorId);
            }
            if !color_ids.insert(color.id) {
                return Err(ColorLibraryError::DuplicateColorId(color.id));
            }
            if let Some(group_id) = color.group_id {
                if !group_ids.contains(&group_id) {
                    return Err(ColorLibraryError::GroupNotFound(group_id));
                }
            }
            if let Some(name) = &color.name {
                validate_required_name(name)?;
            }
            color.rgba.validate()?;
        }

        for group_id in std::iter::once(None).chain(self.groups.iter().map(|group| Some(group.id)))
        {
            validate_contiguous_orders(
                self.colors
                    .iter()
                    .filter(|color| color.group_id == group_id)
                    .map(|color| color.manual_order),
            )?;
        }
        Ok(())
    }
}

/// Invalid saved-color data or operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorLibraryError {
    InvalidColor,
    InvalidName,
    NameTooLong,
    InvalidGroupId,
    InvalidColorId,
    DuplicateGroupId(ColorGroupId),
    DuplicateColorId(SavedColorId),
    GroupNotFound(ColorGroupId),
    ColorNotFound(SavedColorId),
    TooManyGroups,
    TooManyColors,
    InvalidManualOrder,
    IdSpaceExhausted,
}

impl fmt::Display for ColorLibraryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidColor => formatter
                .write_str("saved color components must be finite and between zero and one"),
            Self::InvalidName => formatter.write_str("color names must not be blank"),
            Self::NameTooLong => write!(
                formatter,
                "color names cannot exceed {MAX_COLOR_NAME_BYTES} UTF-8 bytes"
            ),
            Self::InvalidGroupId => formatter.write_str("color group ID must be non-zero"),
            Self::InvalidColorId => formatter.write_str("saved color ID must be non-zero"),
            Self::DuplicateGroupId(id) => {
                write!(formatter, "duplicate color group ID {}", id.get())
            }
            Self::DuplicateColorId(id) => {
                write!(formatter, "duplicate saved color ID {}", id.get())
            }
            Self::GroupNotFound(id) => write!(formatter, "color group {} does not exist", id.get()),
            Self::ColorNotFound(id) => write!(formatter, "saved color {} does not exist", id.get()),
            Self::TooManyGroups => {
                write!(
                    formatter,
                    "document cannot exceed {MAX_COLOR_GROUPS} color groups"
                )
            }
            Self::TooManyColors => {
                write!(
                    formatter,
                    "document cannot exceed {MAX_SAVED_COLORS} saved colors"
                )
            }
            Self::InvalidManualOrder => {
                formatter.write_str("manual orders must be unique and contiguous from zero")
            }
            Self::IdSpaceExhausted => formatter.write_str("color library ID space is exhausted"),
        }
    }
}

impl std::error::Error for ColorLibraryError {}

pub(crate) fn normalize_optional_name(
    name: Option<&str>,
) -> Result<Option<String>, ColorLibraryError> {
    let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
        return Ok(None);
    };
    validate_name_length(name)?;
    Ok(Some(name.to_owned()))
}

pub(crate) fn normalize_required_name(name: &str) -> Result<String, ColorLibraryError> {
    let name = name.trim();
    validate_required_name(name)?;
    Ok(name.to_owned())
}

fn validate_required_name(name: &str) -> Result<(), ColorLibraryError> {
    if name.trim().is_empty() {
        return Err(ColorLibraryError::InvalidName);
    }
    validate_name_length(name)
}

fn validate_name_length(name: &str) -> Result<(), ColorLibraryError> {
    if name.len() > MAX_COLOR_NAME_BYTES {
        Err(ColorLibraryError::NameTooLong)
    } else {
        Ok(())
    }
}

fn validate_contiguous_orders(orders: impl Iterator<Item = u32>) -> Result<(), ColorLibraryError> {
    let mut orders = orders.collect::<Vec<_>>();
    orders.sort_unstable();
    if orders
        .iter()
        .enumerate()
        .all(|(index, order)| usize::try_from(*order) == Ok(index))
    {
        Ok(())
    } else {
        Err(ColorLibraryError::InvalidManualOrder)
    }
}

fn compare_colors(
    left: &SavedColor,
    right: &SavedColor,
    mode: ColorSortMode,
    direction: SortDirection,
) -> Ordering {
    let authored = || {
        left.manual_order
            .cmp(&right.manual_order)
            .then_with(|| left.id.cmp(&right.id))
    };
    if mode == ColorSortMode::Manual {
        return authored();
    }

    let ordering = match mode {
        ColorSortMode::Manual => unreachable!("manual sorting returned above"),
        ColorSortMode::Name => compare_names(left, right),
        ColorSortMode::HueAndShades => compare_hue_and_shades(left.rgba, right.rgba),
        ColorSortMode::Lightness => color_metrics(left.rgba)
            .lightness
            .total_cmp(&color_metrics(right.rgba).lightness),
        ColorSortMode::Chroma => color_metrics(left.rgba)
            .chroma
            .total_cmp(&color_metrics(right.rgba).chroma),
        ColorSortMode::Brightness => {
            relative_luminance(left.rgba).total_cmp(&relative_luminance(right.rgba))
        }
    };
    let ordering = match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    };
    ordering
        .then_with(|| left.rgba.components()[3].total_cmp(&right.rgba.components()[3]))
        .then_with(authored)
}

fn compare_names(left: &SavedColor, right: &SavedColor) -> Ordering {
    match (&left.name, &right.name) {
        (Some(left), Some(right)) => left
            .to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.rgba.hex_label().cmp(&right.rgba.hex_label()),
    }
}

const NEUTRAL_CHROMA: f32 = 0.0001;

fn compare_hue_and_shades(left: RgbaColor, right: RgbaColor) -> Ordering {
    let left = color_metrics(left);
    let right = color_metrics(right);
    let left_neutral = left.chroma < NEUTRAL_CHROMA;
    let right_neutral = right.chroma < NEUTRAL_CHROMA;
    left_neutral
        .cmp(&right_neutral)
        .then_with(|| left.hue.total_cmp(&right.hue))
        .then_with(|| left.lightness.total_cmp(&right.lightness))
}

#[derive(Debug, Clone, Copy)]
struct ColorMetrics {
    lightness: f32,
    chroma: f32,
    hue: f32,
}

fn color_metrics(color: RgbaColor) -> ColorMetrics {
    let [r, g, b, _] = color.components().map(linear_srgb);
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let l = l.cbrt();
    let m = m.cbrt();
    let s = s.cbrt();
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    let chroma = a.hypot(b);
    let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
    ColorMetrics {
        lightness,
        chroma,
        hue,
    }
}

fn relative_luminance(color: RgbaColor) -> f32 {
    let [r, g, b, _] = color.components();
    0.2126 * linear_srgb(r) + 0.7152 * linear_srgb(g) + 0.0722 * linear_srgb(b)
}

fn linear_srgb(component: f32) -> f32 {
    if component <= 0.04045 {
        component / 12.92
    } else {
        ((component + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(id: u64, rgba: [f32; 4], order: u32) -> SavedColor {
        SavedColor {
            id: SavedColorId::from_raw(id),
            group_id: None,
            name: None,
            rgba: RgbaColor::from_array(rgba).unwrap(),
            manual_order: order,
        }
    }

    #[test]
    fn hex_labels_include_alpha_only_when_needed() {
        assert_eq!(
            RgbaColor::new(1.0, 0.0, 0.5, 1.0).unwrap().hex_label(),
            "#FF0080"
        );
        assert_eq!(
            RgbaColor::new(1.0, 0.0, 0.5, 0.5).unwrap().hex_label(),
            "#FF008080"
        );
    }

    #[test]
    fn computed_sort_keeps_manual_order_as_stable_tie_breaker() {
        let group_id = ColorGroupId::from_raw(1);
        let library = ColorLibrary {
            groups: vec![ColorGroup {
                id: group_id,
                name: "Brand".into(),
                manual_order: 0,
                sort_mode: ColorSortMode::Brightness,
                sort_direction: SortDirection::Ascending,
            }],
            colors: vec![
                SavedColor {
                    group_id: Some(group_id),
                    ..saved(1, [0.5, 0.5, 0.5, 1.0], 1)
                },
                SavedColor {
                    group_id: Some(group_id),
                    ..saved(2, [0.5, 0.5, 0.5, 1.0], 0)
                },
            ],
        };

        let ids = library
            .colors_in_group(Some(group_id))
            .into_iter()
            .map(|color| color.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![SavedColorId::from_raw(2), SavedColorId::from_raw(1)]
        );
    }

    #[test]
    fn validation_rejects_non_contiguous_orders() {
        let library = ColorLibrary {
            groups: Vec::new(),
            colors: vec![saved(1, [0.0, 0.0, 0.0, 1.0], 2)],
        };
        assert_eq!(
            library.validate(),
            Err(ColorLibraryError::InvalidManualOrder)
        );
    }
}
