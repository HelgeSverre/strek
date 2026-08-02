use glam::Vec2;
use serde::{Deserialize, Deserializer, Serialize};

/// A single path command in a vector path.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathCmd {
    /// Move to position (start new subpath)
    MoveTo(Vec2),

    /// Line to position
    LineTo(Vec2),

    /// Cubic bezier curve with two control points and endpoint
    CubicTo { c1: Vec2, c2: Vec2, p: Vec2 },

    /// Close current subpath
    Close,
}

/// Relationship between an anchor and its Bézier handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HandleMode {
    /// No curve handles.
    #[default]
    Corner,
    /// Handles stay collinear with equal lengths.
    Mirrored,
    /// Handles stay collinear while retaining independent lengths.
    Aligned,
    /// Handles move independently.
    Independent,
}

/// An editable path anchor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PathAnchor {
    /// Anchor position in node-local coordinates.
    pub position: Vec2,
    /// Incoming handle offset relative to `position`.
    pub in_handle: Option<Vec2>,
    /// Outgoing handle offset relative to `position`.
    pub out_handle: Option<Vec2>,
    /// Handle constraint used by direct editing.
    pub mode: HandleMode,
}

impl PathAnchor {
    /// Create a corner anchor.
    pub fn corner(position: Vec2) -> Self {
        Self {
            position,
            in_handle: None,
            out_handle: None,
            mode: HandleMode::Corner,
        }
    }

    /// Create a smooth anchor with mirrored handles.
    pub fn mirrored(position: Vec2, in_handle: Vec2, out_handle: Vec2) -> Self {
        Self {
            position,
            in_handle: Some(in_handle),
            out_handle: Some(out_handle),
            mode: HandleMode::Mirrored,
        }
    }
}

/// One connected contour within a compound path.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PathContour {
    pub anchors: Vec<PathAnchor>,
    pub closed: bool,
}

impl PathContour {
    /// Create an open contour.
    pub fn open(anchors: impl IntoIterator<Item = PathAnchor>) -> Self {
        Self {
            anchors: anchors.into_iter().collect(),
            closed: false,
        }
    }

    /// Create a closed contour.
    pub fn closed(anchors: impl IntoIterator<Item = PathAnchor>) -> Self {
        Self {
            anchors: anchors.into_iter().collect(),
            closed: true,
        }
    }
}

/// Editable vector path data containing one or more contours.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PathData {
    pub contours: Vec<PathContour>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SerializedPathData {
    Contours { contours: Vec<PathContour> },
    Commands { commands: Vec<PathCmd> },
}

impl<'de> Deserialize<'de> for PathData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match SerializedPathData::deserialize(deserializer)? {
            SerializedPathData::Contours { contours } => Ok(Self { contours }),
            SerializedPathData::Commands { commands } => Ok(Self::from_commands(&commands)),
        }
    }
}

impl PathData {
    /// Create a new empty path.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a rectangle path.
    pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            contours: vec![PathContour::closed([
                PathAnchor::corner(Vec2::new(x, y)),
                PathAnchor::corner(Vec2::new(x + width, y)),
                PathAnchor::corner(Vec2::new(x + width, y + height)),
                PathAnchor::corner(Vec2::new(x, y + height)),
            ])],
        }
    }

    /// Create an ellipse path contained within the given rectangle.
    pub fn ellipse(x: f32, y: f32, width: f32, height: f32) -> Self {
        let rx = width * 0.5;
        let ry = height * 0.5;
        let cx = x + rx;
        let cy = y + ry;
        let k = 0.552_284_8;

        Self {
            contours: vec![PathContour::closed([
                PathAnchor::mirrored(
                    Vec2::new(cx + rx, cy),
                    Vec2::new(0.0, -ry * k),
                    Vec2::new(0.0, ry * k),
                ),
                PathAnchor::mirrored(
                    Vec2::new(cx, cy + ry),
                    Vec2::new(rx * k, 0.0),
                    Vec2::new(-rx * k, 0.0),
                ),
                PathAnchor::mirrored(
                    Vec2::new(cx - rx, cy),
                    Vec2::new(0.0, ry * k),
                    Vec2::new(0.0, -ry * k),
                ),
                PathAnchor::mirrored(
                    Vec2::new(cx, cy - ry),
                    Vec2::new(-rx * k, 0.0),
                    Vec2::new(rx * k, 0.0),
                ),
            ])],
        }
    }

    /// Convert legacy drawing commands into editable contours.
    pub fn from_commands(commands: &[PathCmd]) -> Self {
        let mut contours = Vec::new();
        let mut current = PathContour::default();

        for command in commands {
            match *command {
                PathCmd::MoveTo(position) => {
                    if !current.anchors.is_empty() {
                        contours.push(current);
                        current = PathContour::default();
                    }
                    current.anchors.push(PathAnchor::corner(position));
                }
                PathCmd::LineTo(position) => {
                    current.anchors.push(PathAnchor::corner(position));
                }
                PathCmd::CubicTo { c1, c2, p } => {
                    if let Some(previous) = current.anchors.last_mut() {
                        previous.out_handle = Some(c1 - previous.position);
                        if previous.mode == HandleMode::Corner {
                            previous.mode = HandleMode::Independent;
                        }
                    }
                    current.anchors.push(PathAnchor {
                        position: p,
                        in_handle: Some(c2 - p),
                        out_handle: None,
                        mode: HandleMode::Independent,
                    });
                }
                PathCmd::Close => {
                    current.closed = true;
                    if !current.anchors.is_empty() {
                        contours.push(current);
                        current = PathContour::default();
                    }
                }
            }
        }

        if !current.anchors.is_empty() {
            contours.push(current);
        }

        Self { contours }
    }

    /// Convert editable contours into renderer commands.
    pub fn to_commands(&self) -> Vec<PathCmd> {
        let mut commands = Vec::new();

        for contour in &self.contours {
            let Some(first) = contour.anchors.first() else {
                continue;
            };
            commands.push(PathCmd::MoveTo(first.position));

            for pair in contour.anchors.windows(2) {
                push_segment_command(&mut commands, pair[0], pair[1]);
            }

            if contour.closed && contour.anchors.len() > 1 {
                let last = *contour
                    .anchors
                    .last()
                    .expect("a non-empty contour has a last anchor");
                push_segment_command(&mut commands, last, *first);
                commands.push(PathCmd::Close);
            }
        }

        commands
    }

    /// Compute the axis-aligned bounding box of this path in local coordinates.
    pub fn bounds(&self) -> Option<Rect> {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        let mut has_points = false;

        for contour in &self.contours {
            for anchor in &contour.anchors {
                min = min.min(anchor.position);
                max = max.max(anchor.position);
                has_points = true;
            }
            for (from, to) in contour_segments(contour) {
                if let Some((c1, c2)) = segment_controls(from, to) {
                    for axis in 0..2 {
                        for t in cubic_extrema(
                            component(from.position, axis),
                            component(c1, axis),
                            component(c2, axis),
                            component(to.position, axis),
                        ) {
                            let point = cubic_point(from.position, c1, c2, to.position, t);
                            min = min.min(point);
                            max = max.max(point);
                        }
                    }
                }
            }
        }

        if has_points {
            Some(Rect { min, max })
        } else {
            None
        }
    }

    /// Test whether a point lies inside the path using the even-odd fill rule.
    pub fn contains_point(&self, point: Vec2) -> bool {
        self.contains_point_with_rule(point, crate::FillRule::EvenOdd)
    }

    /// Test whether a point lies inside the path using an explicit fill rule.
    pub fn contains_point_with_rule(&self, point: Vec2, fill_rule: crate::FillRule) -> bool {
        let mut crossings = 0_i32;

        for contour in &self.contours {
            let mut points = flatten_contour(contour, 0.25);
            // SVG fill and clip geometry implicitly closes every open subpath.
            // Stroke distance continues to use the authored open contour.
            if points.len() >= 3 && points.first() != points.last() {
                points.push(points[0]);
            }
            for pair in points.windows(2) {
                let a = pair[0];
                let b = pair[1];
                match fill_rule {
                    crate::FillRule::EvenOdd => {
                        if (a.y > point.y) != (b.y > point.y) {
                            let intersection_x = (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x;
                            if point.x < intersection_x {
                                crossings ^= 1;
                            }
                        }
                    }
                    crate::FillRule::NonZero => {
                        let side = (b.x - a.x) * (point.y - a.y) - (point.x - a.x) * (b.y - a.y);
                        if a.y <= point.y && b.y > point.y && side > 0.0 {
                            crossings += 1;
                        } else if a.y > point.y && b.y <= point.y && side < 0.0 {
                            crossings -= 1;
                        }
                    }
                }
            }
        }

        crossings != 0
    }

    /// Distance from a point to the nearest flattened path segment.
    pub fn distance_to_point(&self, point: Vec2, tolerance: f32) -> f32 {
        self.contours
            .iter()
            .map(|contour| {
                flatten_contour(contour, tolerance)
                    .windows(2)
                    .map(|pair| point_segment_distance(point, pair[0], pair[1]))
                    .fold(f32::INFINITY, f32::min)
            })
            .fold(f32::INFINITY, f32::min)
    }
}

fn push_segment_command(commands: &mut Vec<PathCmd>, from: PathAnchor, to: PathAnchor) {
    if let Some((c1, c2)) = segment_controls(from, to) {
        commands.push(PathCmd::CubicTo {
            c1,
            c2,
            p: to.position,
        });
    } else {
        commands.push(PathCmd::LineTo(to.position));
    }
}

fn contour_segments(contour: &PathContour) -> Vec<(PathAnchor, PathAnchor)> {
    let mut segments = contour
        .anchors
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .collect::<Vec<_>>();

    if contour.closed && contour.anchors.len() > 1 {
        segments.push((
            *contour
                .anchors
                .last()
                .expect("a closed contour has a last anchor"),
            contour.anchors[0],
        ));
    }

    segments
}

fn segment_controls(from: PathAnchor, to: PathAnchor) -> Option<(Vec2, Vec2)> {
    if from.out_handle.is_none() && to.in_handle.is_none() {
        return None;
    }

    Some((
        from.position + from.out_handle.unwrap_or(Vec2::ZERO),
        to.position + to.in_handle.unwrap_or(Vec2::ZERO),
    ))
}

fn component(point: Vec2, axis: usize) -> f32 {
    if axis == 0 {
        point.x
    } else {
        point.y
    }
}

fn cubic_extrema(p0: f32, p1: f32, p2: f32, p3: f32) -> Vec<f32> {
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    const EPSILON: f32 = 1.0e-6;

    if a.abs() < EPSILON {
        if b.abs() < EPSILON {
            return Vec::new();
        }
        let t = -c / b;
        return (0.0..=1.0).contains(&t).then_some(t).into_iter().collect();
    }

    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return Vec::new();
    }

    let root = discriminant.sqrt();
    [(-b + root) / (2.0 * a), (-b - root) / (2.0 * a)]
        .into_iter()
        .filter(|t| (0.0..=1.0).contains(t))
        .collect()
}

fn cubic_point(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f32) -> Vec2 {
    let one_minus_t = 1.0 - t;
    one_minus_t.powi(3) * p0
        + 3.0 * one_minus_t.powi(2) * t * p1
        + 3.0 * one_minus_t * t.powi(2) * p2
        + t.powi(3) * p3
}

fn flatten_contour(contour: &PathContour, tolerance: f32) -> Vec<Vec2> {
    let Some(first) = contour.anchors.first() else {
        return Vec::new();
    };
    let mut points = vec![first.position];
    for (from, to) in contour_segments(contour) {
        if let Some((c1, c2)) = segment_controls(from, to) {
            flatten_cubic(
                from.position,
                c1,
                c2,
                to.position,
                tolerance.max(0.01),
                0,
                &mut points,
            );
        } else {
            points.push(to.position);
        }
    }
    points
}

fn flatten_cubic(
    p0: Vec2,
    p1: Vec2,
    p2: Vec2,
    p3: Vec2,
    tolerance: f32,
    depth: u8,
    points: &mut Vec<Vec2>,
) {
    let flatness = point_segment_distance(p1, p0, p3).max(point_segment_distance(p2, p0, p3));
    if flatness <= tolerance || depth >= 12 {
        points.push(p3);
        return;
    }

    let p01 = (p0 + p1) * 0.5;
    let p12 = (p1 + p2) * 0.5;
    let p23 = (p2 + p3) * 0.5;
    let p012 = (p01 + p12) * 0.5;
    let p123 = (p12 + p23) * 0.5;
    let midpoint = (p012 + p123) * 0.5;
    flatten_cubic(p0, p01, p012, midpoint, tolerance, depth + 1, points);
    flatten_cubic(midpoint, p123, p23, p3, tolerance, depth + 1, points);
}

fn point_segment_distance(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + segment * t)
}

/// Axis-aligned bounding rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    /// Create a new rect from min and max corners.
    pub fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    /// Create a rect from position and size.
    pub fn from_pos_size(pos: Vec2, size: Vec2) -> Self {
        Self {
            min: pos,
            max: pos + size,
        }
    }

    /// Width of the rectangle.
    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    /// Height of the rectangle.
    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    /// Size as a Vec2.
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }

    /// Center point.
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    /// Check if a point is inside the rectangle.
    pub fn contains(&self, point: Vec2) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    /// Union of two rectangles.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Create an empty rect (for accumulating unions).
    pub fn empty() -> Self {
        Self {
            min: Vec2::splat(f32::INFINITY),
            max: Vec2::splat(f32::NEG_INFINITY),
        }
    }

    /// Check if rect is empty/invalid.
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y
    }

    /// Check if this rect intersects with another rect.
    pub fn intersects(&self, other: &Rect) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }

    /// Check if this rect fully encloses another rect, including shared edges.
    pub fn contains_rect(&self, other: &Rect) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.min.x <= other.min.x
            && self.min.y <= other.min.y
            && self.max.x >= other.max.x
            && self.max.y >= other.max.y
    }

    /// Calculate the intersection of two rects, if any.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        if !self.intersects(other) {
            return None;
        }
        Some(Rect {
            min: Vec2::new(self.min.x.max(other.min.x), self.min.y.max(other.min.y)),
            max: Vec2::new(self.max.x.min(other.max.x), self.max.y.min(other.max.y)),
        })
    }
}

impl Default for Rect {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_path_bounds() {
        let path = PathData::rect(10.0, 20.0, 100.0, 50.0);
        let bounds = path.bounds().unwrap();
        assert_eq!(bounds.min, Vec2::new(10.0, 20.0));
        assert_eq!(bounds.max, Vec2::new(110.0, 70.0));
    }

    #[test]
    fn test_ellipse_path_bounds() {
        let path = PathData::ellipse(10.0, 20.0, 100.0, 50.0);
        let bounds = path.bounds().unwrap();
        assert_eq!(bounds.min, Vec2::new(10.0, 20.0));
        assert_eq!(bounds.max, Vec2::new(110.0, 70.0));
        assert_eq!(path.to_commands().len(), 6);
    }

    #[test]
    fn cubic_bounds_ignore_control_points_outside_curve_extrema() {
        let path = PathData::from_commands(&[
            PathCmd::MoveTo(Vec2::ZERO),
            PathCmd::CubicTo {
                c1: Vec2::new(0.0, 100.0),
                c2: Vec2::new(100.0, 100.0),
                p: Vec2::new(100.0, 0.0),
            },
        ]);

        let bounds = path.bounds().unwrap();
        assert!((bounds.max.y - 75.0).abs() < 0.001);
    }

    #[test]
    fn fill_hit_test_does_not_use_the_bounding_box() {
        let ellipse = PathData::ellipse(0.0, 0.0, 100.0, 100.0);
        assert!(ellipse.contains_point(Vec2::splat(50.0)));
        assert!(!ellipse.contains_point(Vec2::splat(2.0)));
    }

    #[test]
    fn compound_path_hit_testing_respects_fill_rule() {
        let path = PathData::from_commands(&[
            PathCmd::MoveTo(Vec2::new(0.0, 0.0)),
            PathCmd::LineTo(Vec2::new(10.0, 0.0)),
            PathCmd::LineTo(Vec2::new(10.0, 10.0)),
            PathCmd::LineTo(Vec2::new(0.0, 10.0)),
            PathCmd::Close,
            PathCmd::MoveTo(Vec2::new(2.0, 2.0)),
            PathCmd::LineTo(Vec2::new(8.0, 2.0)),
            PathCmd::LineTo(Vec2::new(8.0, 8.0)),
            PathCmd::LineTo(Vec2::new(2.0, 8.0)),
            PathCmd::Close,
        ]);

        assert!(path.contains_point_with_rule(Vec2::splat(5.0), crate::FillRule::NonZero));
        assert!(!path.contains_point_with_rule(Vec2::splat(5.0), crate::FillRule::EvenOdd));
    }

    #[test]
    fn fill_hit_testing_implicitly_closes_open_contours() {
        let triangle = PathData {
            contours: vec![PathContour::open([
                PathAnchor::corner(Vec2::new(0.0, 0.0)),
                PathAnchor::corner(Vec2::new(10.0, 0.0)),
                PathAnchor::corner(Vec2::new(10.0, 10.0)),
            ])],
        };

        assert!(triangle.contains_point_with_rule(Vec2::new(8.0, 2.0), crate::FillRule::NonZero));
        assert!(!triangle.contains_point_with_rule(Vec2::new(2.0, 8.0), crate::FillRule::NonZero));
    }

    #[test]
    fn legacy_commands_deserialize_into_contours() {
        let json = r#"{"commands":[{"MoveTo":[0.0,0.0]},{"LineTo":[10.0,0.0]},{"LineTo":[10.0,10.0]},"Close"]}"#;
        let path: PathData = serde_json::from_str(json).unwrap();

        assert_eq!(path.contours.len(), 1);
        assert!(path.contours[0].closed);
        assert_eq!(path.contours[0].anchors.len(), 3);
    }

    #[test]
    fn test_rect_contains() {
        let rect = Rect::new(Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0));
        assert!(rect.contains(Vec2::new(50.0, 50.0)));
        assert!(!rect.contains(Vec2::new(150.0, 50.0)));
        assert!(rect.contains_rect(&Rect::new(Vec2::ZERO, Vec2::splat(100.0))));
        assert!(rect.contains_rect(&Rect::new(Vec2::splat(25.0), Vec2::splat(75.0))));
        assert!(!rect.contains_rect(&Rect::new(Vec2::new(-1.0, 25.0), Vec2::splat(75.0),)));
    }
}
