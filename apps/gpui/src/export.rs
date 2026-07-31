//! SVG and PNG artwork export.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use editor_core::{ArtworkSnapshot, Rect};
use glam::Vec2;

use crate::{canvas, document_io};

const MAX_RASTER_DIMENSION: u32 = 16_384;
const MAX_RASTER_PIXELS: u64 = 64_000_000;

/// User-selectable artwork export format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Svg,
    Png,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
        }
    }
}

/// Failure while preparing or writing an artwork export.
#[derive(Debug)]
pub enum ExportError {
    InvalidBounds,
    RasterTooLarge { width: u32, height: u32 },
    Rasterize(String),
    Write { path: PathBuf, source: io::Error },
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBounds => write!(formatter, "artwork has invalid bounds"),
            Self::RasterTooLarge { width, height } => write!(
                formatter,
                "PNG dimensions {width} × {height} exceed the safe export limit"
            ),
            Self::Rasterize(detail) => write!(formatter, "could not rasterize artwork: {detail}"),
            Self::Write { path, source } => {
                write!(formatter, "could not write {}: {source}", path.display())
            }
        }
    }
}

impl Error for ExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Write { source, .. } => Some(source),
            Self::InvalidBounds | Self::RasterTooLarge { .. } | Self::Rasterize(_) => None,
        }
    }
}

/// Force the destination extension to match the chosen export format.
pub fn normalize_path(path: PathBuf, format: ExportFormat) -> PathBuf {
    let matches_format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(format.extension()));
    if matches_format {
        path
    } else {
        path.with_extension(format.extension())
    }
}

/// Encode and atomically write one immutable artwork snapshot.
pub fn write_export(
    path: &Path,
    format: ExportFormat,
    snapshot: &ArtworkSnapshot,
) -> Result<(), ExportError> {
    let bytes = encode(format, snapshot)?;
    document_io::write_atomic(path, &bytes).map_err(|source| ExportError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn encode(format: ExportFormat, snapshot: &ArtworkSnapshot) -> Result<Vec<u8>, ExportError> {
    let (view_min, size) = normalized_view_box(snapshot.bounds)?;
    let svg =
        render_svg::to_svg_string_export_with_view_box(&snapshot.display_list, view_min, size);

    match format {
        ExportFormat::Svg => Ok(svg.into_bytes()),
        ExportFormat::Png => {
            let width = raster_dimension(size.x)?;
            let height = raster_dimension(size.y)?;
            if width > MAX_RASTER_DIMENSION
                || height > MAX_RASTER_DIMENSION
                || u64::from(width) * u64::from(height) > MAX_RASTER_PIXELS
            {
                return Err(ExportError::RasterTooLarge { width, height });
            }
            canvas::render_svg_pixmap(svg.as_bytes(), width, height)
                .ok_or_else(|| ExportError::Rasterize("the SVG renderer rejected the data".into()))?
                .encode_png()
                .map_err(|error| ExportError::Rasterize(error.to_string()))
        }
    }
}

fn normalized_view_box(bounds: Rect) -> Result<(Vec2, Vec2), ExportError> {
    if !bounds.min.is_finite() || !bounds.max.is_finite() {
        return Err(ExportError::InvalidBounds);
    }
    let mut min = bounds.min;
    let mut size = bounds.size();
    for axis in 0..2 {
        if size[axis] < 0.0 {
            return Err(ExportError::InvalidBounds);
        }
        if size[axis] == 0.0 {
            min[axis] -= 0.5;
            size[axis] = 1.0;
        }
    }
    Ok((min, size))
}

fn raster_dimension(value: f32) -> Result<u32, ExportError> {
    let value = value.ceil();
    if !value.is_finite() || value <= 0.0 || value > u32::MAX as f32 {
        return Err(ExportError::InvalidBounds);
    }
    Ok(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor_render::{DisplayItem, DisplayList, Paint, PathData};
    use glam::Affine2;
    use image::GenericImageView;

    fn snapshot() -> ArtworkSnapshot {
        let mut display_list = DisplayList::new();
        display_list.push(DisplayItem::FillPath {
            path: PathData::rect(0.0, 0.0, 20.0, 10.0),
            paint: Paint::rgb(0.2, 0.4, 0.8),
            transform: Affine2::from_translation(Vec2::new(-25.0, 40.0)),
            opacity: 1.0,
        });
        ArtworkSnapshot {
            display_list,
            bounds: Rect::new(Vec2::new(-27.0, 38.0), Vec2::new(-3.0, 52.0)),
        }
    }

    #[test]
    fn normalizes_the_extension_to_the_selected_format() {
        assert_eq!(
            normalize_path(PathBuf::from("/tmp/drawing.final"), ExportFormat::Svg),
            PathBuf::from("/tmp/drawing.svg")
        );
        assert_eq!(
            normalize_path(PathBuf::from("/tmp/drawing.PNG"), ExportFormat::Png),
            PathBuf::from("/tmp/drawing.PNG")
        );
    }

    #[test]
    fn svg_encoding_uses_the_artwork_bounds() {
        let bytes = encode(ExportFormat::Svg, &snapshot()).unwrap();
        let svg = String::from_utf8(bytes).unwrap();

        assert!(svg.contains(r#"width="24" height="14""#));
        assert!(svg.contains(r#"viewBox="-27 38 24 14""#));
    }

    #[test]
    fn png_encoding_is_transparent_and_uses_artwork_dimensions() {
        let bytes = encode(ExportFormat::Png, &snapshot()).unwrap();
        let image = image::load_from_memory(&bytes).unwrap();

        assert_eq!(image.dimensions(), (24, 14));
        assert!(image.color().has_alpha());
    }

    #[test]
    fn rejects_oversized_raster_exports_before_allocating() {
        let mut snapshot = snapshot();
        snapshot.bounds.max = snapshot.bounds.min + Vec2::splat(20_000.0);

        assert!(matches!(
            encode(ExportFormat::Png, &snapshot),
            Err(ExportError::RasterTooLarge {
                width: 20_000,
                height: 20_000
            })
        ));
    }
}
