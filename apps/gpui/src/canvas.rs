//! Canvas rendering for the GPUI frontend.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fmt::Write;
use std::rc::Rc;
use std::sync::Arc;

use editor_core::{ArtworkSnapshot, NodeId, Rect, TextLayout, TextLayoutLine, View};
use editor_render::{
    DisplayItem, DisplayList, FillRule, LineCap, LineJoin, Paint, PathCmd, PathData, Stroke,
    TextAlignment, TextItem,
};
use glam::{Affine2, Vec2};
use gpui::{
    canvas, point, px, App, Bounds, Corners, DispatchPhase, FillOptions, FillRule as GpuiFillRule,
    IntoElement, MouseMoveEvent, Path, PathBuilder, PathStyle, Pixels, Point, RenderImage, Styled,
    TextAlign as GpuiTextAlign, TextRun, Window,
};
use image::{Frame, RgbaImage};
use lyon::math::point as lyon_point;
use lyon::tessellation::{
    BuffersBuilder, LineCap as LyonLineCap, LineJoin as LyonLineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use smallvec::smallvec;
use unicode_segmentation::UnicodeSegmentation;

const TRANSFORMED_TEXT_CACHE_LIMIT: usize = 256;
const MAX_AFFINE_TEXT_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AFFINE_TEXT_RASTER_DIMENSION: u32 = 16_384;
const MAX_AFFINE_TEXT_RASTER_PIXELS: u64 = 16_000_000;
const MAX_AFFINE_TEXT_FRAME_PIXELS: u64 = MAX_AFFINE_TEXT_CACHE_BYTES / 4;
const MAX_ARTWORK_RASTER_DIMENSION: u32 = 16_384;
const MAX_ARTWORK_RASTER_PIXELS: u64 = 16_000_000;
const MAX_INTERACTIVE_ARTWORK_RASTER_DIMENSION: u32 = 4_096;
const MAX_INTERACTIVE_ARTWORK_RASTER_PIXELS: u64 = 1_000_000;
const ARTWORK_RASTER_OVERSAMPLE: f32 = 1.5;
const ARTWORK_RASTER_PADDING: f32 = 1.0;

struct CachedTextImage {
    key: String,
    image: Arc<RenderImage>,
    byte_len: u64,
    last_used: u64,
}

#[derive(Default)]
struct TextImageCacheState {
    entries: VecDeque<CachedTextImage>,
    pending_drops: Vec<Arc<RenderImage>>,
    generation: u64,
    total_bytes: u64,
}

/// Bounded cache for text that must be rasterized with a full affine transform.
#[derive(Clone, Default)]
pub struct TextImageCache(Rc<RefCell<TextImageCacheState>>);

struct CachedArtworkImage {
    key: ArtworkCacheKey,
    bounds: Rect,
    raster_width: u32,
    raster_height: u32,
    image: Arc<RenderImage>,
}

/// Identity of one immutable world-space artwork snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtworkCacheKey {
    pub document_epoch: u64,
    pub history_revision: u64,
    pub transient_revision: u64,
}

/// CPU-only request that can be prepared away from GPUI's paint thread.
#[derive(Debug, Clone)]
pub struct ArtworkRasterRequest {
    pub key: ArtworkCacheKey,
    snapshot: Arc<ArtworkSnapshot>,
    bounds: Rect,
    raster_width: u32,
    raster_height: u32,
}

pub type ArtworkRasterIdentity = (ArtworkCacheKey, Rect, u32, u32);

impl ArtworkRasterRequest {
    pub fn identity(&self) -> ArtworkRasterIdentity {
        (self.key, self.bounds, self.raster_width, self.raster_height)
    }
}

/// Prepared pixels returned by the background raster worker.
pub struct PreparedArtworkRaster {
    key: ArtworkCacheKey,
    bounds: Rect,
    raster_width: u32,
    raster_height: u32,
    buffer: RgbaImage,
}

#[derive(Default)]
struct ArtworkImageCacheState {
    entry: Option<CachedArtworkImage>,
    previous: Option<CachedArtworkImage>,
    paint_failure: Option<ArtworkRasterIdentity>,
    pending_drops: Vec<Arc<RenderImage>>,
}

/// Dedicated world-space cache for artwork that GPUI cannot paint natively.
#[derive(Clone, Default)]
pub struct ArtworkImageCache(Rc<RefCell<ArtworkImageCacheState>>);

struct CanvasPaintCaches<'a> {
    text: &'a TextImageCache,
    artwork: &'a ArtworkImageCache,
}

pub type WindowMouseMoveHandler = Rc<dyn Fn(&MouseMoveEvent, DispatchPhase, &mut Window, &mut App)>;

struct AffineTextFrameBudget {
    remaining_items: usize,
    remaining_pixels: u64,
}

impl AffineTextFrameBudget {
    fn new(display_list: &DisplayList) -> Self {
        let remaining_items = display_list
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    DisplayItem::Text { transform, .. }
                        if !supports_native_text_transform(transform)
                )
            })
            .count();
        Self {
            remaining_items,
            remaining_pixels: MAX_AFFINE_TEXT_FRAME_PIXELS,
        }
    }

    fn take_raster_limit(&mut self) -> u64 {
        if self.remaining_items == 0 {
            return 1;
        }
        let limit = (self.remaining_pixels / self.remaining_items as u64)
            .clamp(1, MAX_AFFINE_TEXT_RASTER_PIXELS);
        self.remaining_items -= 1;
        self.remaining_pixels = self.remaining_pixels.saturating_sub(limit);
        limit
    }

    fn skip_item(&mut self) {
        self.remaining_items = self.remaining_items.saturating_sub(1);
    }
}

impl TextImageCache {
    pub fn clear(&self) {
        let mut state = self.0.borrow_mut();
        let images = state
            .entries
            .drain(..)
            .map(|entry| entry.image)
            .collect::<Vec<_>>();
        state.pending_drops.extend(images);
        state.total_bytes = 0;
    }

    fn begin_frame(&self, window: &mut Window) {
        let mut state = self.0.borrow_mut();
        let pending_drops = std::mem::take(&mut state.pending_drops);
        for image in pending_drops {
            let _ = window.drop_image(image);
        }
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        while state.entries.len() > TRANSFORMED_TEXT_CACHE_LIMIT
            || state.total_bytes > MAX_AFFINE_TEXT_CACHE_BYTES
        {
            let Some(index) = state
                .entries
                .iter()
                .position(|entry| entry.last_used != generation)
            else {
                break;
            };
            if let Some(entry) = state.entries.remove(index) {
                state.total_bytes = state.total_bytes.saturating_sub(entry.byte_len);
                let _ = window.drop_image(entry.image);
            }
        }
    }

    fn get(&self, key: &str) -> Option<Arc<RenderImage>> {
        let mut state = self.0.borrow_mut();
        let generation = state.generation;
        let entry = state.entries.iter_mut().find(|entry| entry.key == key)?;
        entry.last_used = generation;
        Some(entry.image.clone())
    }

    fn prepare_insert(&self, byte_len: u64, window: &mut Window) -> bool {
        if byte_len > MAX_AFFINE_TEXT_CACHE_BYTES {
            return false;
        }

        let mut state = self.0.borrow_mut();
        let generation = state.generation;
        while state.entries.len() >= TRANSFORMED_TEXT_CACHE_LIMIT
            || state.total_bytes.saturating_add(byte_len) > MAX_AFFINE_TEXT_CACHE_BYTES
        {
            let Some(index) = state
                .entries
                .iter()
                .position(|entry| entry.last_used != generation)
            else {
                return false;
            };
            if let Some(entry) = state.entries.remove(index) {
                state.total_bytes = state.total_bytes.saturating_sub(entry.byte_len);
                let _ = window.drop_image(entry.image);
            }
        }
        true
    }

    fn insert(&self, key: String, image: Arc<RenderImage>, byte_len: u64) {
        let mut state = self.0.borrow_mut();
        let generation = state.generation;
        state.entries.push_back(CachedTextImage {
            key,
            image,
            byte_len,
            last_used: generation,
        });
        state.total_bytes += byte_len;
    }
}

impl ArtworkImageCache {
    pub fn clear(&self) {
        let mut state = self.0.borrow_mut();
        let images = [state.entry.take(), state.previous.take()]
            .into_iter()
            .flatten()
            .map(|entry| entry.image)
            .collect::<Vec<_>>();
        state.pending_drops.extend(images);
        state.paint_failure = None;
    }

    fn get(
        &self,
        key: ArtworkCacheKey,
        bounds: Rect,
        raster_width: u32,
        raster_height: u32,
    ) -> Option<(Arc<RenderImage>, Rect)> {
        let state = self.0.borrow();
        let entry = state.entry.as_ref()?;
        (entry.bounds == bounds
            && entry.key == key
            && entry.raster_width >= raster_width
            && entry.raster_height >= raster_height)
            .then(|| (Arc::clone(&entry.image), entry.bounds))
    }

    fn current_for_document(&self, document_epoch: u64) -> Option<(Arc<RenderImage>, Rect)> {
        let state = self.0.borrow();
        let entry = state.entry.as_ref()?;
        (entry.key.document_epoch == document_epoch)
            .then(|| (Arc::clone(&entry.image), entry.bounds))
    }

    pub fn contains(&self, request: &ArtworkRasterRequest) -> bool {
        self.get(
            request.key,
            request.bounds,
            request.raster_width,
            request.raster_height,
        )
        .is_some()
    }

    pub fn has_artwork_for_key(&self, key: ArtworkCacheKey) -> bool {
        self.0
            .borrow()
            .entry
            .as_ref()
            .is_some_and(|entry| entry.key == key)
    }

    pub fn install(&self, prepared: PreparedArtworkRaster) {
        let image = Arc::new(RenderImage::new(smallvec![Frame::new(prepared.buffer)]));
        let entry = CachedArtworkImage {
            key: prepared.key,
            bounds: prepared.bounds,
            raster_width: prepared.raster_width,
            raster_height: prepared.raster_height,
            image,
        };
        let mut state = self.0.borrow_mut();
        let stale_previous = state.previous.take();
        state.previous = state.entry.replace(entry);
        if let Some(stale_previous) = stale_previous {
            state.pending_drops.push(stale_previous.image);
        }
    }

    fn flush_pending_drops(&self, window: &mut Window) {
        let pending_drops = std::mem::take(&mut self.0.borrow_mut().pending_drops);
        for image in pending_drops {
            let _ = window.drop_image(image);
        }
    }

    fn confirm_painted(&self, image: &Arc<RenderImage>, window: &mut Window) {
        let mut state = self.0.borrow_mut();
        let is_current = state
            .entry
            .as_ref()
            .is_some_and(|entry| Arc::ptr_eq(&entry.image, image));
        let previous = if is_current {
            state.previous.take()
        } else {
            None
        };
        drop(state);
        if let Some(previous) = previous {
            let _ = window.drop_image(previous.image);
        }
    }

    fn restore_previous_after_paint_failure(
        &self,
        image: &Arc<RenderImage>,
        window: &mut Window,
    ) -> bool {
        let mut state = self.0.borrow_mut();
        let is_current = state
            .entry
            .as_ref()
            .is_some_and(|entry| Arc::ptr_eq(&entry.image, image));
        if !is_current {
            return false;
        }
        let failed = state.entry.take();
        let previous = state.previous.take();
        let restored = previous.is_some();
        state.entry = previous;
        drop(state);
        if let Some(failed) = failed {
            let _ = window.drop_image(failed.image);
        }
        restored
    }

    fn record_paint_failure(&self, image: &Arc<RenderImage>) {
        let mut state = self.0.borrow_mut();
        let Some(entry) = state
            .entry
            .as_ref()
            .filter(|entry| Arc::ptr_eq(&entry.image, image))
        else {
            return;
        };
        state.paint_failure = Some((
            entry.key,
            entry.bounds,
            entry.raster_width,
            entry.raster_height,
        ));
    }

    pub fn take_paint_failure(&self) -> Option<ArtworkRasterIdentity> {
        self.0.borrow_mut().paint_failure.take()
    }
}

/// Render a display list into a canvas that fills its parent.
pub fn render_canvas(
    display_list: DisplayList,
    artwork: Option<ArtworkCacheKey>,
    view: View,
    text_cache: TextImageCache,
    artwork_cache: ArtworkImageCache,
    window_mouse_move: WindowMouseMoveHandler,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, cx| {
            let window_mouse_move = Rc::clone(&window_mouse_move);
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                window_mouse_move(event, phase, window, cx);
            });
            let caches = CanvasPaintCaches {
                text: &text_cache,
                artwork: &artwork_cache,
            };
            paint_display_list(&display_list, artwork, view, bounds, caches, window, cx)
        },
    )
    .size_full()
}

pub(crate) fn requires_rasterized_artwork(display_list: &DisplayList) -> bool {
    display_list.items.iter().any(|item| match item {
        DisplayItem::BeginGroup { opacity, clip_path } => *opacity != 1.0 || clip_path.is_some(),
        DisplayItem::FillPath { paint, .. } => !matches!(paint, Paint::Solid(_)),
        DisplayItem::StrokePath { stroke, .. } => {
            !matches!(stroke.paint, Paint::Solid(_)) || !stroke.dash_array.is_empty()
        }
        DisplayItem::Text { text, .. } => !matches!(text.fill, Paint::Solid(_)),
        DisplayItem::EndGroup
        | DisplayItem::ToolPreview { .. }
        | DisplayItem::SnapGuide { .. }
        | DisplayItem::SelectionRect { .. }
        | DisplayItem::SelectionQuad { .. }
        | DisplayItem::MarqueeRect { .. }
        | DisplayItem::VectorAnchor { .. }
        | DisplayItem::VectorHandle { .. }
        | DisplayItem::TextCaret { .. }
        | DisplayItem::TextSelectionRect { .. }
        | DisplayItem::TransformHandle { .. } => false,
    })
}

pub fn artwork_raster_request(
    key: ArtworkCacheKey,
    snapshot: Arc<ArtworkSnapshot>,
    view: View,
    device_scale: f32,
    interactive: bool,
) -> Option<ArtworkRasterRequest> {
    let (artwork_bounds, raster_width, raster_height) =
        artwork_raster_geometry(&snapshot, view, device_scale, interactive)?;
    Some(ArtworkRasterRequest {
        key,
        snapshot,
        bounds: artwork_bounds,
        raster_width,
        raster_height,
    })
}

fn artwork_raster_geometry(
    snapshot: &ArtworkSnapshot,
    view: View,
    device_scale: f32,
    interactive: bool,
) -> Option<(Rect, u32, u32)> {
    let artwork_bounds = padded_artwork_bounds(snapshot)?;
    let logical_size = artwork_bounds.size();
    // Raster density follows zoom in power-of-two tiers. Camera motion never
    // invalidates the content key, and the UI keeps showing the previous tier
    // while a sharper image is prepared in the background.
    let desired_scale =
        raster_scale_bucket(device_scale * ARTWORK_RASTER_OVERSAMPLE * view.zoom.abs().max(1.0));
    let (raster_width, raster_height) = affine_text_raster_dimensions(
        logical_size,
        desired_scale,
        if interactive {
            MAX_INTERACTIVE_ARTWORK_RASTER_DIMENSION
        } else {
            MAX_ARTWORK_RASTER_DIMENSION
        },
        if interactive {
            MAX_INTERACTIVE_ARTWORK_RASTER_PIXELS
        } else {
            MAX_ARTWORK_RASTER_PIXELS
        },
    )?;
    Some((artwork_bounds, raster_width, raster_height))
}

pub fn prepare_artwork_raster(request: ArtworkRasterRequest) -> Option<PreparedArtworkRaster> {
    let logical_size = request.bounds.size();
    let svg = render_svg::to_svg_string_export_with_view_box(
        &request.snapshot.display_list,
        request.bounds.min,
        logical_size,
    );
    let needs_system_fonts = request
        .snapshot
        .display_list
        .items
        .iter()
        .any(|item| matches!(item, DisplayItem::Text { .. }));
    let pixmap = render_svg_pixmap_with_fonts(
        svg.as_bytes(),
        request.raster_width,
        request.raster_height,
        needs_system_fonts,
    )?;
    let pixmap_width = pixmap.width();
    let pixmap_height = pixmap.height();
    let mut pixels = pixmap.take();
    premultiplied_rgba_to_unpremultiplied_bgra(&mut pixels);
    let buffer = RgbaImage::from_raw(pixmap_width, pixmap_height, pixels)?;
    Some(PreparedArtworkRaster {
        key: request.key,
        bounds: request.bounds,
        raster_width: request.raster_width,
        raster_height: request.raster_height,
        buffer,
    })
}

fn paint_rasterized_artwork(
    key: ArtworkCacheKey,
    view: View,
    canvas_bounds: Bounds<Pixels>,
    image_cache: &ArtworkImageCache,
    window: &mut Window,
) -> bool {
    for attempt in 0..2 {
        let Some((image, image_artwork_bounds)) =
            image_cache.current_for_document(key.document_epoch)
        else {
            return false;
        };

        let screen_min = view.to_screen(image_artwork_bounds.min);
        let screen_size = image_artwork_bounds.size() * view.zoom.abs();
        let image_bounds = Bounds {
            origin: point(
                canvas_bounds.origin.x + px(screen_min.x),
                canvas_bounds.origin.y + px(screen_min.y),
            ),
            size: gpui::size(px(screen_size.x), px(screen_size.y)),
        };
        if window
            .paint_image(
                image_bounds,
                Corners::default(),
                Arc::clone(&image),
                0,
                false,
            )
            .is_ok()
        {
            image_cache.confirm_painted(&image, window);
            return true;
        }
        if attempt == 0 {
            image_cache.record_paint_failure(&image);
            window.refresh();
            if image_cache.restore_previous_after_paint_failure(&image, window) {
                continue;
            }
        }
        return false;
    }
    false
}

fn padded_artwork_bounds(snapshot: &ArtworkSnapshot) -> Option<Rect> {
    let bounds = snapshot.bounds;
    if !bounds.min.is_finite() || !bounds.max.is_finite() {
        return None;
    }
    let mut min = bounds.min;
    let mut max = bounds.max;
    for axis in 0..2 {
        if max[axis] < min[axis] {
            return None;
        }
        if max[axis] == min[axis] {
            min[axis] -= 0.5;
            max[axis] += 0.5;
        }
    }
    let text_overhang = snapshot
        .display_list
        .items
        .iter()
        .filter_map(|item| {
            let DisplayItem::Text {
                text, transform, ..
            } = item
            else {
                return None;
            };
            let scale = transform
                .matrix2
                .x_axis
                .length()
                .max(transform.matrix2.y_axis.length());
            let padding = text.font_size.max(1.0) * scale * 0.5;
            padding.is_finite().then_some(padding)
        })
        .fold(ARTWORK_RASTER_PADDING, f32::max);
    Some(Rect::new(
        min - Vec2::splat(text_overhang),
        max + Vec2::splat(text_overhang),
    ))
}

fn raster_scale_bucket(scale: f32) -> f32 {
    if !scale.is_finite() || scale <= 1.0 {
        1.0
    } else {
        2.0_f32.powf(scale.log2().ceil()).min(256.0)
    }
}

fn paint_display_list(
    display_list: &DisplayList,
    artwork: Option<ArtworkCacheKey>,
    view: View,
    bounds: Bounds<Pixels>,
    caches: CanvasPaintCaches<'_>,
    window: &mut Window,
    cx: &mut App,
) {
    caches.artwork.flush_pending_drops(window);
    caches.text.begin_frame(window);
    let mut affine_text_budget = AffineTextFrameBudget::new(display_list);
    let rasterized_artwork = artwork
        .is_some_and(|key| paint_rasterized_artwork(key, view, bounds, caches.artwork, window));
    for item in &display_list.items {
        match item {
            DisplayItem::FillPath {
                path,
                paint,
                fill_rule,
                transform,
                opacity,
            } if !rasterized_artwork => {
                paint_fill_path(window, bounds, path, paint, *fill_rule, transform, *opacity)
            }
            DisplayItem::StrokePath {
                path,
                stroke,
                transform,
                opacity,
            } if !rasterized_artwork => {
                paint_stroke(window, bounds, path, stroke, transform, *opacity);
            }
            DisplayItem::Text {
                text,
                transform,
                opacity,
            } if !rasterized_artwork => {
                if supports_native_text_transform(transform) {
                    paint_native_text(window, cx, bounds, text, transform, *opacity);
                } else {
                    paint_affine_text(
                        window,
                        bounds,
                        caches.text,
                        &mut affine_text_budget,
                        text,
                        transform,
                        *opacity,
                    );
                }
            }
            DisplayItem::ToolPreview {
                path,
                fill,
                stroke,
                transform,
            } => {
                paint_fill_path(
                    window,
                    bounds,
                    path,
                    fill,
                    FillRule::NonZero,
                    transform,
                    1.0,
                );
                paint_stroke(window, bounds, path, stroke, transform, 1.0);
            }
            DisplayItem::SelectionRect { min, max } => {
                paint_selection_rect(window, bounds, *min, *max)
            }
            DisplayItem::SelectionQuad { corners } => {
                paint_selection_quad(window, bounds, *corners)
            }
            DisplayItem::MarqueeRect { min, max } => paint_marquee_rect(window, bounds, *min, *max),
            DisplayItem::SnapGuide { start, end, .. } => {
                paint_snap_guide(window, bounds, *start, *end)
            }
            DisplayItem::VectorAnchor { position, selected } => {
                paint_vector_anchor(window, bounds, *position, *selected)
            }
            DisplayItem::VectorHandle { anchor, handle } => {
                paint_vector_handle(window, bounds, *anchor, *handle)
            }
            DisplayItem::TextCaret { start, end } => paint_text_caret(window, bounds, *start, *end),
            DisplayItem::TextSelectionRect { corners, marked } => {
                paint_text_selection_rect(window, bounds, *corners, *marked)
            }
            DisplayItem::TransformHandle { position, rotation } => {
                paint_transform_handle(window, bounds, *position, *rotation)
            }
            DisplayItem::BeginGroup { .. }
            | DisplayItem::EndGroup
            | DisplayItem::FillPath { .. }
            | DisplayItem::StrokePath { .. }
            | DisplayItem::Text { .. } => {}
        }
    }
}

fn paint_fill_path(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    path: &PathData,
    paint: &Paint,
    fill_rule: FillRule,
    transform: &Affine2,
    opacity: f32,
) {
    let fill_rule = match fill_rule {
        FillRule::NonZero => GpuiFillRule::NonZero,
        FillRule::EvenOdd => GpuiFillRule::EvenOdd,
    };
    let builder = PathBuilder::fill().with_style(PathStyle::Fill(
        FillOptions::default().with_fill_rule(fill_rule),
    ));
    if let Some(path) = build_path(builder, bounds, path, transform) {
        window.paint_path(path, paint_color(paint, opacity));
    }
}

fn paint_stroke(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    path: &PathData,
    stroke: &Stroke,
    transform: &Affine2,
    opacity: f32,
) {
    let Some(mesh) = stroke_mesh(path, stroke, transform) else {
        return;
    };
    let mut builder = PathBuilder::fill();
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let points = [
            mesh.vertices[triangle[0] as usize],
            mesh.vertices[triangle[1] as usize],
            mesh.vertices[triangle[2] as usize],
        ];
        builder.move_to(canvas_point(bounds, &Affine2::IDENTITY, points[0]));
        builder.line_to(canvas_point(bounds, &Affine2::IDENTITY, points[1]));
        builder.line_to(canvas_point(bounds, &Affine2::IDENTITY, points[2]));
        builder.close();
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, paint_color(&stroke.paint, opacity));
    }
}

fn stroke_mesh(
    path: &PathData,
    stroke: &Stroke,
    transform: &Affine2,
) -> Option<VertexBuffers<Vec2, u32>> {
    let mut path_builder = lyon::path::Path::builder();
    let mut subpath_open = false;
    for command in &path.commands {
        match command {
            PathCmd::MoveTo(position) => {
                if subpath_open {
                    path_builder.end(false);
                }
                path_builder.begin(lyon_point(position.x, position.y));
                subpath_open = true;
            }
            PathCmd::LineTo(position) => {
                path_builder.line_to(lyon_point(position.x, position.y));
            }
            PathCmd::CubicTo { c1, c2, p } => {
                path_builder.cubic_bezier_to(
                    lyon_point(c1.x, c1.y),
                    lyon_point(c2.x, c2.y),
                    lyon_point(p.x, p.y),
                );
            }
            PathCmd::Close => {
                path_builder.close();
                subpath_open = false;
            }
        }
    }
    if subpath_open {
        path_builder.end(false);
    }
    let lyon_path = path_builder.build();
    let line_cap = match stroke.line_cap {
        LineCap::Butt => LyonLineCap::Butt,
        LineCap::Round => LyonLineCap::Round,
        LineCap::Square => LyonLineCap::Square,
    };
    let line_join = match stroke.line_join {
        LineJoin::Miter => LyonLineJoin::Miter,
        LineJoin::MiterClip => LyonLineJoin::MiterClip,
        LineJoin::Round => LyonLineJoin::Round,
        LineJoin::Bevel => LyonLineJoin::Bevel,
    };
    let options = StrokeOptions::default()
        .with_line_width(stroke.width)
        .with_start_cap(line_cap)
        .with_end_cap(line_cap)
        .with_line_join(line_join)
        .with_miter_limit(stroke.miter_limit.max(f32::EPSILON));
    let mut geometry = VertexBuffers::new();
    StrokeTessellator::new()
        .tessellate_path(
            &lyon_path,
            &options,
            &mut BuffersBuilder::new(&mut geometry, |vertex: StrokeVertex| {
                transform.transform_point2(Vec2::from_array(vertex.position().to_array()))
            }),
        )
        .ok()?;
    Some(geometry)
}

fn build_path(
    mut builder: PathBuilder,
    bounds: Bounds<Pixels>,
    path: &PathData,
    transform: &Affine2,
) -> Option<Path<Pixels>> {
    for command in &path.commands {
        match command {
            PathCmd::MoveTo(position) => {
                builder.move_to(canvas_point(bounds, transform, *position));
            }
            PathCmd::LineTo(position) => {
                builder.line_to(canvas_point(bounds, transform, *position));
            }
            PathCmd::CubicTo { c1, c2, p } => {
                builder.cubic_bezier_to(
                    canvas_point(bounds, transform, *p),
                    canvas_point(bounds, transform, *c1),
                    canvas_point(bounds, transform, *c2),
                );
            }
            PathCmd::Close => builder.close(),
        }
    }

    builder.build().ok()
}

fn canvas_point(bounds: Bounds<Pixels>, transform: &Affine2, position: Vec2) -> Point<Pixels> {
    let position = transform.transform_point2(position);
    point(
        bounds.origin.x + px(position.x),
        bounds.origin.y + px(position.y),
    )
}

fn transform_scale(transform: &Affine2) -> f32 {
    let scale = transform.matrix2.determinant().abs().sqrt();
    if scale.is_finite() {
        scale.max(f32::EPSILON)
    } else {
        1.0
    }
}

fn paint_color(paint: &Paint, opacity: f32) -> gpui::Rgba {
    match paint {
        Paint::Solid(color) => gpui::Rgba {
            r: color[0],
            g: color[1],
            b: color[2],
            a: color[3] * opacity,
        },
        Paint::LinearGradient(_) | Paint::RadialGradient(_) => gpui::Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        },
    }
}

fn paint_native_text(
    window: &mut Window,
    cx: &mut App,
    bounds: Bounds<Pixels>,
    text: &TextItem,
    transform: &Affine2,
    opacity: f32,
) {
    let scale = transform_scale(transform);
    let font_size = px(text.font_size * scale);
    let line_height = px(text.font_size * text.line_height.max(0.1) * scale);
    let font = text_font(text);
    let color: gpui::Hsla = paint_color(&text.fill, opacity).into();
    let run = text_run(text.content.len(), font, color);
    let Ok(shaped_lines) = window.text_system().shape_text(
        text.content.clone().into(),
        font_size,
        &[run],
        text.wrap_width.map(|width| px(width * scale)),
        None,
    ) else {
        return;
    };
    let container_width = text.wrap_width.map_or_else(
        || {
            shaped_lines
                .iter()
                .map(|line| line.width())
                .max()
                .unwrap_or(px(1.0))
        },
        |width| px(width * scale),
    );
    let alignment = match text.alignment {
        TextAlignment::Left => GpuiTextAlign::Left,
        TextAlignment::Center => GpuiTextAlign::Center,
        TextAlignment::Right => GpuiTextAlign::Right,
    };
    let base =
        bounds.origin + gpui::Point::new(px(transform.translation.x), px(transform.translation.y));
    let mut y = px(0.0);
    for shaped in shaped_lines {
        let height = shaped.size(line_height).height;
        let origin = base + gpui::Point::new(px(0.0), y);
        let paint_bounds = Bounds::new(origin, gpui::size(container_width, height));
        let _ = shaped.paint(
            origin,
            line_height,
            alignment,
            Some(paint_bounds),
            window,
            cx,
        );
        y += height;
    }
}

fn text_font(text: &TextItem) -> gpui::Font {
    let mut font = gpui::font(resolve_gpui_font_family(&text.font_family));
    font.weight = gpui::FontWeight(text.font_weight as f32);
    font.style = if text.font_italic {
        gpui::FontStyle::Italic
    } else {
        gpui::FontStyle::Normal
    };
    font
}

fn resolve_gpui_font_family(family: &str) -> String {
    let trimmed = family.trim();
    let resolved = if trimmed.eq_ignore_ascii_case("sans-serif")
        || trimmed.eq_ignore_ascii_case("system-ui")
    {
        ".SystemUIFont"
    } else if trimmed.eq_ignore_ascii_case("serif") {
        platform_serif_family()
    } else if trimmed.eq_ignore_ascii_case("monospace") {
        crate::typography::MONOSPACE_FONT_FAMILY
    } else {
        trimmed
    };
    resolved.to_owned()
}

fn platform_serif_family() -> &'static str {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        "Times New Roman"
    } else {
        "DejaVu Serif"
    }
}

fn text_run(len: usize, font: gpui::Font, color: gpui::Hsla) -> TextRun {
    TextRun {
        len,
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// Shape all document text through GPUI so core interaction geometry and painting agree.
pub fn shape_text_layouts(
    items: Vec<(NodeId, TextItem)>,
    window: &mut Window,
) -> HashMap<NodeId, TextLayout> {
    items
        .into_iter()
        .filter_map(|(id, text)| shape_text_layout(&text, window).map(|layout| (id, layout)))
        .collect()
}

fn shape_text_layout(text: &TextItem, window: &mut Window) -> Option<TextLayout> {
    let run = text_run(
        text.content.len(),
        text_font(text),
        gpui::transparent_black(),
    );
    let shaped_lines = window
        .text_system()
        .shape_text(
            text.content.clone().into(),
            px(text.font_size),
            &[run],
            text.wrap_width.map(px),
            None,
        )
        .ok()?;
    let line_height = text.font_size * text.line_height.max(0.1);
    let mut lines = Vec::new();
    let mut global_start = 0usize;
    for shaped in shaped_lines {
        let explicit_len = shaped.text.len();
        let mut starts = Vec::with_capacity(shaped.wrap_boundaries().len() + 1);
        starts.push(0usize);
        starts.extend(shaped.wrap_boundaries().iter().map(|boundary| {
            shaped.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index
        }));
        let ends = starts
            .iter()
            .copied()
            .skip(1)
            .chain(std::iter::once(explicit_len))
            .collect::<Vec<_>>();
        for (segment_index, (start, end)) in starts.into_iter().zip(ends).enumerate() {
            let start_x = shaped.unwrapped_layout.x_for_index(start).0;
            let positions =
                std::iter::once((global_start + start, 0.0))
                    .chain(shaped.text[start..end].grapheme_indices(true).map(
                        |(byte, grapheme)| {
                            let boundary = start + byte + grapheme.len();
                            (
                                global_start + boundary,
                                shaped.unwrapped_layout.x_for_index(boundary).0 - start_x,
                            )
                        },
                    ))
                    .collect::<Vec<_>>();
            let character_count = positions.len().saturating_sub(1);
            let hard_break = segment_index == shaped.wrap_boundaries().len()
                && global_start + explicit_len < text.content.len();
            lines.push(TextLayoutLine {
                range: global_start + start..global_start + end,
                x: 0.0,
                character_count,
                hard_break,
                positions,
            });
        }
        global_start = (global_start + explicit_len + 1).min(text.content.len());
    }
    if lines.is_empty() {
        lines.push(TextLayoutLine {
            range: 0..0,
            x: 0.0,
            character_count: 0,
            hard_break: false,
            positions: vec![(0, 0.0)],
        });
    }
    let content_width = lines
        .iter()
        .filter_map(|line| line.positions.last().map(|(_, x)| *x))
        .fold(1.0, f32::max);
    let width = text.wrap_width.unwrap_or(content_width).max(1.0);
    for line in &mut lines {
        let line_width = line.positions.last().map_or(0.0, |(_, x)| *x);
        line.x = match text.alignment {
            TextAlignment::Left => 0.0,
            TextAlignment::Center => (width - line_width) * 0.5,
            TextAlignment::Right => width - line_width,
        };
    }
    let grapheme_count = text.content.graphemes(true).count().max(1) as f32;
    Some(TextLayout {
        lines,
        character_width: (content_width / grapheme_count).max(1.0),
        line_height,
        width,
    })
}

fn supports_native_text_transform(transform: &Affine2) -> bool {
    let x = transform.matrix2.x_axis;
    let y = transform.matrix2.y_axis;
    let tolerance = x.length().max(y.length()).max(1.0) * 1e-4;
    x.y.abs() <= tolerance
        && y.x.abs() <= tolerance
        && x.x > 0.0
        && y.y > 0.0
        && (x.x - y.y).abs() <= tolerance
}

struct AffineTextImage {
    svg: String,
    min: Vec2,
    size: Vec2,
}

fn paint_affine_text(
    window: &mut Window,
    canvas_bounds: Bounds<Pixels>,
    text_cache: &TextImageCache,
    frame_budget: &mut AffineTextFrameBudget,
    text: &TextItem,
    transform: &Affine2,
    opacity: f32,
) {
    let Some(layout) = shape_text_layout(text, window) else {
        frame_budget.skip_item();
        return;
    };
    let Some(rendered) = affine_text_svg(text, &layout, transform, opacity) else {
        frame_budget.skip_item();
        return;
    };
    let canvas_size = Vec2::new(canvas_bounds.size.width.0, canvas_bounds.size.height.0);
    if !affine_text_intersects_canvas(rendered.min, rendered.size, canvas_size) {
        frame_budget.skip_item();
        return;
    }
    let raster_pixel_limit = frame_budget.take_raster_limit();
    let raster_scale = window.scale_factor() * 2.0;
    let Some((raster_width, raster_height)) = affine_text_raster_dimensions(
        rendered.size,
        raster_scale,
        MAX_AFFINE_TEXT_RASTER_DIMENSION,
        raster_pixel_limit,
    ) else {
        return;
    };
    let key = format!("{}x{}:{}", raster_width, raster_height, rendered.svg);

    let image = text_cache.get(&key).or_else(|| {
        let image_byte_len = u64::from(raster_width) * u64::from(raster_height) * 4;
        let cache_byte_len = image_byte_len.saturating_add(key.len() as u64);
        let should_cache = text_cache.prepare_insert(cache_byte_len, window);
        let pixmap = render_svg_pixmap(rendered.svg.as_bytes(), raster_width, raster_height)?;
        let mut pixels = pixmap.data().to_vec();
        premultiplied_rgba_to_unpremultiplied_bgra(&mut pixels);
        let buffer = RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixels)?;
        let image = Arc::new(RenderImage::new(smallvec![Frame::new(buffer)]));
        if should_cache {
            text_cache.insert(key, image.clone(), cache_byte_len);
        }
        Some(image)
    });
    let Some(image) = image else {
        return;
    };
    let image_bounds = Bounds {
        origin: canvas_bounds.origin + point(px(rendered.min.x), px(rendered.min.y)),
        size: gpui::size(px(rendered.size.x), px(rendered.size.y)),
    };
    let _ = window.paint_image(image_bounds, Corners::default(), image, 0, false);
}

fn affine_text_intersects_canvas(min: Vec2, size: Vec2, canvas_size: Vec2) -> bool {
    min.is_finite()
        && size.is_finite()
        && canvas_size.is_finite()
        && size.cmpgt(Vec2::ZERO).all()
        && canvas_size.cmpgt(Vec2::ZERO).all()
        && min.cmplt(canvas_size).all()
        && (min + size).cmpgt(Vec2::ZERO).all()
}

fn affine_text_raster_dimensions(
    size: Vec2,
    scale: f32,
    max_dimension: u32,
    max_pixels: u64,
) -> Option<(u32, u32)> {
    if !size.is_finite()
        || !scale.is_finite()
        || size.cmple(Vec2::ZERO).any()
        || scale <= 0.0
        || max_dimension == 0
        || max_pixels == 0
    {
        return None;
    }

    let desired_width = (f64::from(size.x) * f64::from(scale)).ceil().max(1.0);
    let desired_height = (f64::from(size.y) * f64::from(scale)).ceil().max(1.0);
    let max_dimension = f64::from(max_dimension);
    let dimension_scale = (max_dimension / desired_width)
        .min(max_dimension / desired_height)
        .min(1.0);
    let pixel_scale = ((max_pixels as f64) / (desired_width * desired_height))
        .sqrt()
        .min(1.0);
    let bounded_scale = dimension_scale.min(pixel_scale);
    if !bounded_scale.is_finite() || bounded_scale <= 0.0 {
        return None;
    }

    let mut width = (desired_width * bounded_scale)
        .floor()
        .clamp(1.0, max_dimension) as u32;
    let mut height = (desired_height * bounded_scale)
        .floor()
        .clamp(1.0, max_dimension) as u32;
    while u64::from(width) * u64::from(height) > max_pixels {
        if width >= height && width > 1 {
            width -= 1;
        } else if height > 1 {
            height -= 1;
        } else {
            return None;
        }
    }
    Some((width, height))
}

fn premultiplied_rgba_to_unpremultiplied_bgra(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = pixel[3] as u32;
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
        pixel.swap(0, 2);
    }
}

pub(crate) fn render_svg_pixmap(
    bytes: &[u8],
    width: u32,
    height: u32,
) -> Option<resvg::tiny_skia::Pixmap> {
    render_svg_pixmap_with_fonts(bytes, width, height, true)
}

fn render_svg_pixmap_with_fonts(
    bytes: &[u8],
    width: u32,
    height: u32,
    use_system_fonts: bool,
) -> Option<resvg::tiny_skia::Pixmap> {
    let tree = parse_svg_tree(bytes, use_system_fonts)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let scale = resvg::tiny_skia::Transform::from_scale(
        width as f32 / tree.size().width(),
        height as f32 / tree.size().height(),
    );
    resvg::render(&tree, scale, &mut pixmap.as_mut());
    Some(pixmap)
}

fn parse_svg_tree(bytes: &[u8], use_system_fonts: bool) -> Option<resvg::usvg::Tree> {
    let options = resvg::usvg::Options {
        fontdb: if use_system_fonts {
            crate::typography::system_font_database()
        } else {
            crate::typography::empty_font_database()
        },
        ..Default::default()
    };
    resvg::usvg::Tree::from_data(bytes, &options).ok()
}

fn svg_document(body: &str, view_min: Vec2, size: Vec2) -> String {
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" "#,
            r#"viewBox="{} {} {} {}">{}</svg>"#
        ),
        size.x, size.y, view_min.x, view_min.y, size.x, size.y, body,
    )
}

fn svg_line_markup(text: &str, line: &TextLayoutLine, baseline: f32) -> Option<String> {
    let has_independent_ascii_positions = text.is_ascii()
        && line.positions.len() == text.len() + 1
        && line
            .positions
            .windows(2)
            .all(|positions| positions[1].1 > positions[0].1);
    if has_independent_ascii_positions {
        let mut x_values = String::new();
        for (byte, _) in text.char_indices() {
            let index = line.range.start + byte;
            let position = line
                .positions
                .iter()
                .find_map(|(boundary, x)| (*boundary == index).then_some(*x))?;
            if !x_values.is_empty() {
                x_values.push(' ');
            }
            write!(x_values, "{}", line.x + position).ok()?;
        }
        if x_values.is_empty() {
            write!(x_values, "{}", line.x).ok()?;
        }
        return Some(format!(
            r#"<tspan x="{x_values}" y="{baseline}">{}</tspan>"#,
            xml_escape(text)
        ));
    }

    // Absolute positions for every Unicode scalar split combining marks and
    // ZWJ sequences into separate SVG text chunks. Keep complex Unicode as
    // one shaped chunk and constrain only its total advance to GPUI's result.
    let width = line.positions.last().map_or(0.0, |(_, x)| *x).max(0.0);
    let length =
        (width > 0.0).then(|| format!(r#" textLength="{width}" lengthAdjust="spacingAndGlyphs""#));
    Some(format!(
        r#"<tspan x="{}" y="{baseline}"{}>{}</tspan>"#,
        line.x,
        length.as_deref().unwrap_or_default(),
        xml_escape(text),
    ))
}

fn affine_text_svg(
    text: &TextItem,
    layout: &TextLayout,
    transform: &Affine2,
    opacity: f32,
) -> Option<AffineTextImage> {
    if !transform.matrix2.is_finite() || !transform.translation.is_finite() {
        return None;
    }
    let padding = text.font_size.max(1.0);
    let visual_min_x = layout.lines.iter().map(|line| line.x).fold(0.0, f32::min);
    let visual_max_x = layout.lines.iter().fold(layout.width, |right, line| {
        right.max(line.x + line.positions.last().map_or(0.0, |(_, x)| *x))
    });
    let local_min = Vec2::new(visual_min_x - padding, -padding);
    let local_max = Vec2::new(visual_max_x + padding, layout.height() + padding);
    let corners = [
        transform.transform_point2(local_min),
        transform.transform_point2(Vec2::new(local_max.x, local_min.y)),
        transform.transform_point2(local_max),
        transform.transform_point2(Vec2::new(local_min.x, local_max.y)),
    ];
    let min = corners
        .iter()
        .copied()
        .fold(Vec2::splat(f32::INFINITY), Vec2::min);
    let max = corners
        .iter()
        .copied()
        .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
    let size = (max - min).max(Vec2::ONE);
    if !min.is_finite() || !size.is_finite() {
        return None;
    }

    let matrix = transform.matrix2;
    let relative_translation = transform.translation - min;
    let color = text.fill.solid_color()?;
    let red = (color[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let green = (color[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let blue = (color[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    let fill_opacity = (color[3] * opacity).clamp(0.0, 1.0);
    let font_style = if text.font_italic { "italic" } else { "normal" };
    let mut body = format!(
        concat!(
            r#"<text xml:space="preserve" font-family="{}" font-size="{}" font-weight="{}" "#,
            r#"font-style="{}" text-anchor="start" fill="rgb({} {} {})" fill-opacity="{}" "#,
            r#"transform="matrix({} {} {} {} {} {})">"#
        ),
        xml_escape(&text.font_family),
        text.font_size,
        text.font_weight,
        font_style,
        red,
        green,
        blue,
        fill_opacity,
        matrix.x_axis.x,
        matrix.x_axis.y,
        matrix.y_axis.x,
        matrix.y_axis.y,
        relative_translation.x,
        relative_translation.y,
    );
    for (line_index, line) in layout.lines.iter().enumerate() {
        let content = &text.content[line.range.clone()];
        let baseline = text.font_size + line_index as f32 * layout.line_height;
        body.push_str(&svg_line_markup(content, line, baseline)?);
    }
    body.push_str("</text>");

    // Simple character origins and complex-line total advances above come
    // from GPUI shaping. The two engines can still resolve a fallback glyph
    // differently; measure the exact resvg geometry and use that as the
    // viewport so overhangs cannot clip.
    let provisional_svg = svg_document(&body, Vec2::ZERO, size);
    let tree = parse_svg_tree(provisional_svg.as_bytes(), true)?;
    let bounds = tree.root().abs_bounding_box();
    let raster_padding = 1.0;
    let view_min = Vec2::new(bounds.x(), bounds.y()) - Vec2::splat(raster_padding);
    let size = Vec2::new(bounds.width(), bounds.height()) + Vec2::splat(raster_padding * 2.0);
    if !view_min.is_finite() || !size.is_finite() || !size.cmpgt(Vec2::ZERO).all() {
        return None;
    }
    let svg = svg_document(&body, view_min, size);
    Some(AffineTextImage {
        svg,
        min: min + view_min,
        size,
    })
}

fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn paint_selection_rect(window: &mut Window, bounds: Bounds<Pixels>, min: Vec2, max: Vec2) {
    window.paint_quad(gpui::PaintQuad {
        bounds: rect_bounds(bounds, min, max),
        corner_radii: gpui::Corners::all(px(0.0)),
        background: gpui::Background::from(gpui::transparent_black()),
        border_widths: gpui::Edges::all(px(1.0)),
        border_color: gpui::Rgba {
            r: 0.0,
            g: 0.47,
            b: 0.84,
            a: 1.0,
        }
        .into(),
    });
}

fn paint_selection_quad(window: &mut Window, bounds: Bounds<Pixels>, corners: [Vec2; 4]) {
    let mut outline = PathBuilder::stroke(px(1.0));
    outline.move_to(canvas_point(bounds, &Affine2::IDENTITY, corners[0]));
    for corner in &corners[1..] {
        outline.line_to(canvas_point(bounds, &Affine2::IDENTITY, *corner));
    }
    outline.close();
    if let Ok(path) = outline.build() {
        window.paint_path(path, gpui::rgba(0x0078d7ff));
    }
}

fn paint_marquee_rect(window: &mut Window, bounds: Bounds<Pixels>, min: Vec2, max: Vec2) {
    window.paint_quad(gpui::PaintQuad {
        bounds: rect_bounds(bounds, min, max),
        corner_radii: gpui::Corners::all(px(0.0)),
        background: gpui::Background::from(gpui::Rgba {
            r: 0.0,
            g: 0.47,
            b: 0.84,
            a: 0.1,
        }),
        border_widths: gpui::Edges::all(px(1.0)),
        border_color: gpui::Rgba {
            r: 0.0,
            g: 0.47,
            b: 0.84,
            a: 0.5,
        }
        .into(),
    });
}

fn rect_bounds(bounds: Bounds<Pixels>, min: Vec2, max: Vec2) -> Bounds<Pixels> {
    Bounds {
        origin: point(bounds.origin.x + px(min.x), bounds.origin.y + px(min.y)),
        size: gpui::size(px(max.x - min.x), px(max.y - min.y)),
    }
}

fn paint_snap_guide(window: &mut Window, bounds: Bounds<Pixels>, start: Vec2, end: Vec2) {
    let min = start.min(end);
    let max = start.max(end);
    let max = Vec2::new(max.x.max(min.x + 1.0), max.y.max(min.y + 1.0));

    window.paint_quad(gpui::PaintQuad {
        bounds: rect_bounds(bounds, min, max),
        corner_radii: gpui::Corners::all(px(0.0)),
        background: gpui::Background::from(gpui::Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.5,
            a: 1.0,
        }),
        border_widths: gpui::Edges::all(px(0.0)),
        border_color: gpui::transparent_black(),
    });
}

fn paint_vector_anchor(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    position: Vec2,
    selected: bool,
) {
    let radius = 4.0;
    window.paint_quad(gpui::PaintQuad {
        bounds: rect_bounds(
            bounds,
            position - Vec2::splat(radius),
            position + Vec2::splat(radius),
        ),
        corner_radii: gpui::Corners::all(px(1.0)),
        background: gpui::Background::from(if selected {
            gpui::Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            }
        } else {
            gpui::Rgba {
                r: 0.047,
                g: 0.55,
                b: 0.91,
                a: 1.0,
            }
        }),
        border_widths: gpui::Edges::all(px(1.0)),
        border_color: gpui::Rgba {
            r: 0.047,
            g: 0.55,
            b: 0.91,
            a: 1.0,
        }
        .into(),
    });
}

fn paint_vector_handle(window: &mut Window, bounds: Bounds<Pixels>, anchor: Vec2, handle: Vec2) {
    let mut builder = PathBuilder::stroke(px(1.0));
    builder.move_to(point(
        bounds.origin.x + px(anchor.x),
        bounds.origin.y + px(anchor.y),
    ));
    builder.line_to(point(
        bounds.origin.x + px(handle.x),
        bounds.origin.y + px(handle.y),
    ));
    if let Ok(path) = builder.build() {
        window.paint_path(
            path,
            gpui::Rgba {
                r: 0.047,
                g: 0.55,
                b: 0.91,
                a: 0.8,
            },
        );
    }
    paint_vector_anchor(window, bounds, handle, false);
}

fn paint_text_caret(window: &mut Window, bounds: Bounds<Pixels>, start: Vec2, end: Vec2) {
    let mut builder = PathBuilder::stroke(px(1.0));
    builder.move_to(canvas_point(bounds, &Affine2::IDENTITY, start));
    builder.line_to(canvas_point(bounds, &Affine2::IDENTITY, end));
    if let Ok(path) = builder.build() {
        window.paint_path(
            path,
            gpui::Rgba {
                r: 0.047,
                g: 0.55,
                b: 0.91,
                a: 1.0,
            },
        );
    }
}

fn paint_text_selection_rect(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    corners: [Vec2; 4],
    marked: bool,
) {
    let mut fill = PathBuilder::fill();
    fill.move_to(canvas_point(bounds, &Affine2::IDENTITY, corners[0]));
    for corner in &corners[1..] {
        fill.line_to(canvas_point(bounds, &Affine2::IDENTITY, *corner));
    }
    fill.close();
    if let Ok(path) = fill.build() {
        window.paint_path(
            path,
            if marked {
                gpui::rgba(0x0c8ce936)
            } else {
                gpui::rgba(0x0c8ce958)
            },
        );
    }
    if marked {
        let mut underline = PathBuilder::stroke(px(1.0));
        underline.move_to(canvas_point(bounds, &Affine2::IDENTITY, corners[3]));
        underline.line_to(canvas_point(bounds, &Affine2::IDENTITY, corners[2]));
        if let Ok(path) = underline.build() {
            window.paint_path(path, gpui::rgba(0x5ebcffdd));
        }
    }
}

fn paint_transform_handle(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    position: Vec2,
    rotation: bool,
) {
    let radius = if rotation { 5.0 } else { 4.0 };
    window.paint_quad(gpui::PaintQuad {
        bounds: rect_bounds(
            bounds,
            position - Vec2::splat(radius),
            position + Vec2::splat(radius),
        ),
        corner_radii: gpui::Corners::all(px(if rotation { radius } else { 1.0 })),
        background: gpui::Background::from(gpui::Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }),
        border_widths: gpui::Edges::all(px(1.0)),
        border_color: gpui::Rgba {
            r: 0.047,
            g: 0.55,
            b: 0.91,
            a: 1.0,
        }
        .into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_point_applies_transform_and_canvas_origin() {
        let bounds = Bounds {
            origin: point(px(10.0), px(20.0)),
            size: gpui::size(px(100.0), px(100.0)),
        };
        let transform = Affine2::from_translation(Vec2::new(5.0, 7.0));

        let actual = canvas_point(bounds, &transform, Vec2::new(1.0, 2.0));

        assert_eq!(actual, point(px(16.0), px(29.0)));
    }

    #[test]
    fn transform_scale_tracks_uniform_zoom() {
        let transform = Affine2::from_scale(Vec2::splat(2.5));

        assert_eq!(transform_scale(&transform), 2.5);
    }

    #[test]
    fn advanced_artwork_selects_the_rasterized_canvas_path() {
        let simple = DisplayList {
            items: vec![DisplayItem::FillPath {
                path: PathData::rect(0.0, 0.0, 10.0, 10.0),
                paint: Paint::black(),
                fill_rule: FillRule::NonZero,
                transform: Affine2::IDENTITY,
                opacity: 1.0,
            }],
        };
        assert!(!requires_rasterized_artwork(&simple));

        let translucent_leaf = DisplayList {
            items: vec![DisplayItem::FillPath {
                path: PathData::rect(0.0, 0.0, 10.0, 10.0),
                paint: Paint::black(),
                fill_rule: FillRule::NonZero,
                transform: Affine2::IDENTITY,
                opacity: 0.5,
            }],
        };
        assert!(!requires_rasterized_artwork(&translucent_leaf));

        let mut dashed = Stroke::black(1.0);
        dashed.dash_array = vec![2.0, 1.0];
        let advanced = DisplayList {
            items: vec![DisplayItem::StrokePath {
                path: PathData::rect(0.0, 0.0, 10.0, 10.0),
                stroke: dashed,
                transform: Affine2::IDENTITY,
                opacity: 1.0,
            }],
        };
        assert!(requires_rasterized_artwork(&advanced));

        let isolated = DisplayList {
            items: vec![
                DisplayItem::BeginGroup {
                    opacity: 0.5,
                    clip_path: None,
                },
                DisplayItem::EndGroup,
            ],
        };
        assert!(requires_rasterized_artwork(&isolated));
    }

    #[test]
    fn world_space_artwork_cache_hits_without_a_camera_key() {
        let display_list = DisplayList {
            items: vec![DisplayItem::FillPath {
                path: PathData::rect(0.0, 0.0, 10.0, 10.0),
                paint: Paint::LinearGradient(editor_render::LinearGradient {
                    start: Vec2::ZERO,
                    end: Vec2::splat(10.0),
                    transform: Affine2::IDENTITY,
                    spread: editor_render::SpreadMethod::Pad,
                    stops: vec![
                        editor_render::GradientStop {
                            offset: 0.0,
                            color: [0.0, 0.0, 0.0, 1.0],
                        },
                        editor_render::GradientStop {
                            offset: 1.0,
                            color: [1.0, 1.0, 1.0, 1.0],
                        },
                    ],
                }),
                fill_rule: FillRule::NonZero,
                transform: Affine2::IDENTITY,
                opacity: 1.0,
            }],
        };
        let bounds = Rect::new(Vec2::splat(-1.0), Vec2::splat(11.0));
        let snapshot = ArtworkSnapshot {
            display_list,
            bounds,
        };
        let key = ArtworkCacheKey {
            document_epoch: 4,
            history_revision: 9,
            transient_revision: 2,
        };
        let image = Arc::new(RenderImage::new(smallvec![Frame::new(RgbaImage::new(
            24, 24,
        ))]));
        let cache = ArtworkImageCache::default();
        cache.0.borrow_mut().entry = Some(CachedArtworkImage {
            key,
            bounds,
            raster_width: 24,
            raster_height: 24,
            image: Arc::clone(&image),
        });

        let (hit, hit_bounds) = cache.get(key, bounds, 12, 12).unwrap();
        assert!(Arc::ptr_eq(&hit, &image));
        assert_eq!(hit_bounds, bounds);
        assert!(cache.has_artwork_for_key(key));
        assert!(!cache.has_artwork_for_key(ArtworkCacheKey {
            transient_revision: 3,
            ..key
        }));
        assert!(cache.get(key, bounds, 25, 24).is_none());
        assert!(cache
            .get(
                ArtworkCacheKey {
                    transient_revision: 3,
                    ..key
                },
                bounds,
                12,
                12,
            )
            .is_none());

        let low_zoom = artwork_raster_request(
            key,
            Arc::new(snapshot.clone()),
            View {
                zoom: 1.0,
                ..View::default()
            },
            2.0,
            false,
        )
        .unwrap();
        let high_zoom = artwork_raster_request(
            key,
            Arc::new(snapshot),
            View {
                zoom: 4.0,
                ..View::default()
            },
            2.0,
            false,
        )
        .unwrap();
        assert!(high_zoom.raster_width > low_zoom.raster_width);
        assert!(high_zoom.raster_height > low_zoom.raster_height);
    }

    #[test]
    fn artwork_raster_density_is_tiered_and_bounded() {
        assert_eq!(raster_scale_bucket(0.5), 1.0);
        assert_eq!(raster_scale_bucket(3.0), 4.0);
        assert_eq!(raster_scale_bucket(300.0), 256.0);
        assert_eq!(raster_scale_bucket(f32::NAN), 1.0);

        let snapshot = ArtworkSnapshot {
            display_list: DisplayList::default(),
            bounds: Rect::new(Vec2::ZERO, Vec2::splat(10.0)),
        };
        let padded = padded_artwork_bounds(&snapshot).unwrap();
        assert_eq!(padded.min, Vec2::splat(-ARTWORK_RASTER_PADDING));
        assert_eq!(padded.max, Vec2::splat(10.0 + ARTWORK_RASTER_PADDING));
    }

    #[test]
    fn css_generic_families_resolve_to_gpui_platform_fonts() {
        assert_eq!(resolve_gpui_font_family("sans-serif"), ".SystemUIFont");
        assert_eq!(resolve_gpui_font_family("system-ui"), ".SystemUIFont");
        assert_eq!(resolve_gpui_font_family("serif"), platform_serif_family());
        assert_eq!(
            resolve_gpui_font_family("monospace"),
            crate::typography::MONOSPACE_FONT_FAMILY
        );
        assert_eq!(resolve_gpui_font_family("Custom Family"), "Custom Family");
    }

    #[test]
    fn affine_text_fallback_preserves_the_full_matrix() {
        let transform = Affine2::from_translation(Vec2::new(80.0, 40.0))
            * Affine2::from_angle(0.4)
            * Affine2::from_scale(Vec2::new(1.5, 0.75));
        let text = TextItem::new("Rotated & scaled", 16.0);
        let positions = (0..=text.content.len())
            .map(|index| (index, index as f32 / text.content.len() as f32 * 140.0))
            .collect();
        let layout = TextLayout {
            lines: vec![TextLayoutLine {
                range: 0..text.content.len(),
                x: 0.0,
                character_count: text.content.graphemes(true).count(),
                hard_break: false,
                positions,
            }],
            character_width: 8.0,
            line_height: 19.2,
            width: 140.0,
        };

        assert!(!supports_native_text_transform(&transform));
        let rendered = affine_text_svg(&text, &layout, &transform, 0.8).unwrap();
        let matrix = transform.matrix2;
        let expected_matrix_prefix = format!(
            "matrix({} {} {} {} ",
            matrix.x_axis.x, matrix.x_axis.y, matrix.y_axis.x, matrix.y_axis.y
        );

        assert!(rendered.svg.contains(&expected_matrix_prefix));
        assert!(rendered.svg.contains("Rotated &amp; scaled"));
        assert!(rendered.size.cmpgt(Vec2::ZERO).all());
        let pixmap = render_svg_pixmap(rendered.svg.as_bytes(), 256, 128).unwrap();
        assert!(pixmap
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn affine_text_viewport_uses_resvg_glyph_bounds() {
        let transform = Affine2::from_translation(Vec2::new(30.0, 20.0)) * Affine2::from_angle(0.2);
        let text = TextItem::new("WWWWWW", 32.0);
        let positions = (0..=text.content.len())
            .map(|index| (index, index as f32 / text.content.len() as f32))
            .collect();
        let layout = TextLayout {
            lines: vec![TextLayoutLine {
                range: 0..text.content.len(),
                x: 0.0,
                character_count: text.content.graphemes(true).count(),
                hard_break: false,
                positions,
            }],
            character_width: 1.0,
            line_height: 38.4,
            width: 1.0,
        };

        let rendered = affine_text_svg(&text, &layout, &transform, 1.0).unwrap();

        assert!(
            rendered.size.x > 20.0,
            "measured width was {}",
            rendered.size.x
        );
        assert!(render_svg_pixmap(rendered.svg.as_bytes(), 512, 256).is_some());
    }

    #[test]
    fn affine_text_uses_gpui_caret_positions_for_simple_svg_text() {
        let content = "abc";
        let line = TextLayoutLine {
            range: 0..content.len(),
            x: 2.0,
            character_count: 3,
            hard_break: false,
            positions: vec![(0, 0.0), (1, 5.0), (2, 17.0), (3, 24.0)],
        };

        assert!(svg_line_markup(content, &line, 16.0)
            .unwrap()
            .contains(r#"x="2 7 19""#));
    }

    #[test]
    fn affine_text_keeps_multiscalar_graphemes_in_one_svg_chunk() {
        let content = "a\u{301}b";
        let line = TextLayoutLine {
            range: 0..content.len(),
            x: 2.0,
            character_count: 2,
            hard_break: false,
            positions: vec![(0, 0.0), (3, 10.0), (4, 24.0)],
        };
        let markup = svg_line_markup(content, &line, 16.0).unwrap();

        assert_eq!(markup.matches("<tspan").count(), 1);
        assert!(markup.contains(content));
        assert!(!markup.contains(r#"x="2 2"#));
        assert!(markup.contains(r#"textLength="24""#));
    }

    #[test]
    fn affine_text_keeps_ascii_ligatures_in_one_svg_chunk() {
        let content = "fi";
        let line = TextLayoutLine {
            range: 0..content.len(),
            x: 2.0,
            character_count: 2,
            hard_break: false,
            positions: vec![(0, 0.0), (1, 0.0), (2, 12.0)],
        };
        let markup = svg_line_markup(content, &line, 16.0).unwrap();

        assert_eq!(markup.matches("<tspan").count(), 1);
        assert!(markup.contains(r#"x="2""#));
        assert!(!markup.contains(r#"x="2 2""#));
        assert!(markup.contains(r#"textLength="12""#));
    }

    #[test]
    fn stroke_outline_receives_the_full_non_uniform_transform() {
        let path = PathData {
            commands: vec![
                PathCmd::MoveTo(Vec2::ZERO),
                PathCmd::LineTo(Vec2::new(10.0, 0.0)),
            ],
        };
        let stroke = Stroke::black(2.0);
        let transform = Affine2::from_scale(Vec2::new(2.0, 3.0));
        let mesh = stroke_mesh(&path, &stroke, &transform).unwrap();
        let min = mesh
            .vertices
            .iter()
            .copied()
            .fold(Vec2::splat(f32::INFINITY), Vec2::min);
        let max = mesh
            .vertices
            .iter()
            .copied()
            .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);

        assert!((min.x - 0.0).abs() < 0.001);
        assert!((max.x - 20.0).abs() < 0.001);
        assert!((min.y + 3.0).abs() < 0.001);
        assert!((max.y - 3.0).abs() < 0.001);
    }

    #[test]
    fn affine_text_pixels_are_unpremultiplied_and_swizzled_for_gpui() {
        let mut pixel = [64, 32, 16, 128];

        premultiplied_rgba_to_unpremultiplied_bgra(&mut pixel);

        assert_eq!(pixel, [32, 64, 128, 128]);
    }

    #[test]
    fn affine_text_raster_dimensions_downsample_to_dimension_and_pixel_limits() {
        assert_eq!(
            affine_text_raster_dimensions(Vec2::new(100.2, 50.1), 2.0, 1_000, 50_000),
            Some((201, 101))
        );
        for dimensions in [
            affine_text_raster_dimensions(Vec2::new(600.0, 10.0), 2.0, 1_000, 50_000),
            affine_text_raster_dimensions(Vec2::new(300.0, 300.0), 2.0, 1_000, 50_000),
        ] {
            let (width, height) = dimensions.expect("valid text should be downsampled");
            assert!(width <= 1_000);
            assert!(height <= 1_000);
            assert!(u64::from(width) * u64::from(height) <= 50_000);
        }
        assert_eq!(
            affine_text_raster_dimensions(Vec2::new(f32::INFINITY, 10.0), 1.0, 1_000, 50_000),
            None
        );
    }

    #[test]
    fn affine_text_rasters_only_when_they_intersect_the_canvas() {
        let canvas = Vec2::new(800.0, 600.0);

        assert!(affine_text_intersects_canvas(
            Vec2::new(-10.0, 20.0),
            Vec2::new(20.0, 20.0),
            canvas
        ));
        assert!(!affine_text_intersects_canvas(
            Vec2::new(800.0, 20.0),
            Vec2::new(20.0, 20.0),
            canvas
        ));
        assert!(!affine_text_intersects_canvas(
            Vec2::new(20.0, -20.0),
            Vec2::new(20.0, 20.0),
            canvas
        ));
    }

    #[test]
    fn affine_text_frame_budget_is_shared_without_starving_later_items() {
        let mut budget = AffineTextFrameBudget {
            remaining_items: 3,
            remaining_pixels: 10,
        };

        assert_eq!(budget.take_raster_limit(), 3);
        assert_eq!(budget.take_raster_limit(), 3);
        assert_eq!(budget.take_raster_limit(), 4);
        assert_eq!(budget.remaining_pixels, 0);
        assert_eq!(budget.take_raster_limit(), 1);
    }

    #[test]
    fn affine_text_frame_budget_skips_items_without_consuming_pixels() {
        let mut budget = AffineTextFrameBudget {
            remaining_items: 3,
            remaining_pixels: 10,
        };

        budget.skip_item();

        assert_eq!(budget.take_raster_limit(), 5);
        assert_eq!(budget.remaining_pixels, 5);
    }
}
