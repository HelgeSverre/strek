//! Display list generation from the document.

use editor_render::{
    ClipNode, ClipPath, ClipShape, DisplayItem, DisplayList, FillRule, GradientStop, LineCap,
    LineJoin, LinearGradient, Paint, PathCmd, PathData, RadialGradient, ResolvedTextLayout,
    ResolvedTextLine, SpreadMethod, Stroke, TextAlignment, TextItem,
};

use crate::node::NodeKind;
use crate::transform::View;
use crate::Document;

impl Document {
    /// Collect render text descriptors for platform shaping.
    pub fn text_items_for_layout(&self) -> Vec<(crate::NodeId, TextItem)> {
        self.nodes
            .iter()
            .filter_map(|(id, node)| {
                if node.deleted {
                    return None;
                }
                let NodeKind::Text(text) = &node.kind else {
                    return None;
                };
                let fill = node.style.fill.clone().unwrap_or_else(crate::Paint::black);
                Some((id, convert_text_item(text, &fill, None)))
            })
            .collect()
    }

    /// Build a display list for rendering.
    ///
    /// The display list contains all visible items in paint order,
    /// with transforms already composed to screen space.
    pub fn build_display_list(&mut self, view: &View) -> DisplayList {
        let mut items = Vec::new();
        let screen_from_world = view.screen_from_world();

        enum Traversal {
            Enter(crate::NodeId),
            EndGroup,
        }
        let mut stack = vec![Traversal::Enter(self.root)];
        while let Some(step) = stack.pop() {
            let Traversal::Enter(id) = step else {
                items.push(DisplayItem::EndGroup);
                continue;
            };
            let Some(node) = self.nodes.get(id).cloned() else {
                continue;
            };
            if !node.visible || node.deleted {
                continue;
            }

            let screen_transform = screen_from_world * self.world_transform(id);
            let node_opacity = node.style.opacity.clamp(0.0, 1.0);
            let inline_opacity = node.clip_path.is_none()
                && node.children.is_empty()
                && painted_item_count(&node) == 1;
            let opens_group =
                node.clip_path.is_some() || (node.style.opacity != 1.0 && !inline_opacity);
            let item_opacity = if opens_group { 1.0 } else { node_opacity };
            if opens_group {
                items.push(DisplayItem::BeginGroup {
                    opacity: node_opacity,
                    clip_path: node
                        .clip_path
                        .as_ref()
                        .map(|clip| convert_clip_path(clip, screen_transform)),
                });
                stack.push(Traversal::EndGroup);
            }

            match node.kind {
                NodeKind::Group => {}
                NodeKind::Shape(path) => {
                    append_shape_items(
                        &mut items,
                        convert_path(&path),
                        &node.style,
                        screen_transform,
                        item_opacity,
                    );
                }
                NodeKind::Text(text) => {
                    if let Some(fill) = &node.style.fill {
                        let layout = self.text_layout(id).map(convert_text_layout);
                        items.push(DisplayItem::Text {
                            text: convert_text_item(&text, fill, layout),
                            transform: screen_transform,
                            opacity: item_opacity,
                        });
                    }
                }
                NodeKind::Frame(frame) => {
                    if let Some(bg) = &frame.background {
                        items.push(DisplayItem::FillPath {
                            path: PathData::rect(0.0, 0.0, frame.width, frame.height),
                            paint: convert_paint(bg),
                            fill_rule: FillRule::NonZero,
                            transform: screen_transform,
                            opacity: item_opacity,
                        });
                    }
                }
            }

            stack.extend(node.children.into_iter().rev().map(Traversal::Enter));
        }

        DisplayList { items }
    }
}

fn append_shape_items(
    items: &mut Vec<DisplayItem>,
    path: PathData,
    style: &crate::Style,
    transform: glam::Affine2,
    opacity: f32,
) {
    let fill = style.fill.as_ref().map(|paint| DisplayItem::FillPath {
        path: path.clone(),
        paint: convert_paint(paint),
        fill_rule: convert_fill_rule(style.fill_rule),
        transform,
        opacity,
    });
    let stroke = style.stroke.as_ref().map(|stroke| DisplayItem::StrokePath {
        path,
        stroke: convert_stroke(stroke),
        transform,
        opacity,
    });
    match style.paint_order {
        crate::PaintOrder::FillAndStroke => items.extend(fill.into_iter().chain(stroke)),
        crate::PaintOrder::StrokeAndFill => items.extend(stroke.into_iter().chain(fill)),
    }
}

fn painted_item_count(node: &crate::Node) -> usize {
    match &node.kind {
        NodeKind::Group => 0,
        NodeKind::Shape(_) => {
            usize::from(node.style.fill.is_some()) + usize::from(node.style.stroke.is_some())
        }
        NodeKind::Text(_) => usize::from(node.style.fill.is_some()),
        NodeKind::Frame(frame) => usize::from(frame.background.is_some()),
    }
}

fn convert_text_item(
    text: &crate::TextData,
    fill: &crate::Paint,
    layout: Option<ResolvedTextLayout>,
) -> TextItem {
    TextItem {
        content: text.content.clone(),
        font_family: text.font_family().to_string(),
        font_size: text.font_size,
        font_weight: text.weight(),
        font_italic: text.italic(),
        fill: convert_paint(fill),
        line_height: text.line_height,
        alignment: match text.align {
            crate::TextAlign::Left => TextAlignment::Left,
            crate::TextAlign::Center => TextAlignment::Center,
            crate::TextAlign::Right => TextAlignment::Right,
        },
        wrap_width: text.fixed_width(),
        layout,
    }
}

fn convert_text_layout(layout: crate::TextLayout) -> ResolvedTextLayout {
    ResolvedTextLayout {
        lines: layout
            .lines
            .into_iter()
            .map(|line| ResolvedTextLine {
                range: line.range,
                x: line.x,
            })
            .collect(),
        line_height: layout.line_height,
        width: layout.width,
    }
}

/// Convert core path data to render path data.
pub(crate) fn convert_path(path: &crate::path::PathData) -> PathData {
    PathData {
        commands: path
            .to_commands()
            .into_iter()
            .map(|cmd| match cmd {
                crate::path::PathCmd::MoveTo(p) => PathCmd::MoveTo(p),
                crate::path::PathCmd::LineTo(p) => PathCmd::LineTo(p),
                crate::path::PathCmd::CubicTo { c1, c2, p } => PathCmd::CubicTo { c1, c2, p },
                crate::path::PathCmd::Close => PathCmd::Close,
            })
            .collect(),
    }
}

/// Convert core paint to render paint.
fn convert_paint(paint: &crate::style::Paint) -> Paint {
    match paint {
        crate::style::Paint::Solid(color) => Paint::Solid(*color),
        crate::style::Paint::LinearGradient(gradient) => Paint::LinearGradient(LinearGradient {
            start: gradient.start,
            end: gradient.end,
            transform: gradient.transform,
            spread: convert_spread_method(gradient.spread),
            stops: convert_gradient_stops(&gradient.stops),
        }),
        crate::style::Paint::RadialGradient(gradient) => Paint::RadialGradient(RadialGradient {
            center: gradient.center,
            focal: gradient.focal,
            radius: gradient.radius,
            transform: gradient.transform,
            spread: convert_spread_method(gradient.spread),
            stops: convert_gradient_stops(&gradient.stops),
        }),
    }
}

/// Convert core stroke to render stroke.
fn convert_stroke(stroke: &crate::style::Stroke) -> Stroke {
    Stroke {
        width: stroke.width,
        paint: convert_paint(&stroke.paint),
        line_cap: match stroke.line_cap {
            crate::LineCap::Butt => LineCap::Butt,
            crate::LineCap::Round => LineCap::Round,
            crate::LineCap::Square => LineCap::Square,
        },
        line_join: match stroke.line_join {
            crate::LineJoin::Miter => LineJoin::Miter,
            crate::LineJoin::MiterClip => LineJoin::MiterClip,
            crate::LineJoin::Round => LineJoin::Round,
            crate::LineJoin::Bevel => LineJoin::Bevel,
        },
        miter_limit: stroke.miter_limit,
        dash_array: stroke.dash_array.clone(),
        dash_offset: stroke.dash_offset,
    }
}

fn convert_fill_rule(fill_rule: crate::FillRule) -> FillRule {
    match fill_rule {
        crate::FillRule::NonZero => FillRule::NonZero,
        crate::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

fn convert_spread_method(spread: crate::SpreadMethod) -> SpreadMethod {
    match spread {
        crate::SpreadMethod::Pad => SpreadMethod::Pad,
        crate::SpreadMethod::Reflect => SpreadMethod::Reflect,
        crate::SpreadMethod::Repeat => SpreadMethod::Repeat,
    }
}

fn convert_gradient_stops(stops: &[crate::GradientStop]) -> Vec<GradientStop> {
    stops
        .iter()
        .map(|stop| GradientStop {
            offset: stop.offset,
            color: stop.color,
        })
        .collect()
}

fn convert_clip_path(clip: &crate::ClipPath, transform: glam::Affine2) -> ClipPath {
    let content_transform = transform * clip.transform;
    ClipPath {
        children: clip
            .children
            .iter()
            .map(|child| match child {
                crate::ClipNode::Group(group) => {
                    ClipNode::Group(convert_clip_path(group, content_transform))
                }
                crate::ClipNode::Shape(shape) => ClipNode::Shape(ClipShape {
                    path: convert_path(&shape.path),
                    transform: content_transform * shape.transform,
                    fill_rule: convert_fill_rule(shape.fill_rule),
                }),
            })
            .collect(),
        clip_path: clip
            .clip_path
            .as_ref()
            .map(|nested| Box::new(convert_clip_path(nested, transform))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;
    use crate::path::PathData as CorePathData;
    use crate::style::{Paint as CorePaint, Style};
    use glam::Vec2;

    #[test]
    fn test_build_display_list_empty() {
        let mut doc = Document::new();
        let view = View::default();

        let list = doc.build_display_list(&view);
        assert!(list.is_empty());
    }

    #[test]
    fn test_build_display_list_with_shape() {
        let mut doc = Document::new();
        let view = View::default();

        let shape = Node::shape("Rect", CorePathData::rect(0.0, 0.0, 100.0, 100.0))
            .with_style(Style::fill(CorePaint::rgb(1.0, 0.0, 0.0)));

        doc.add_child(doc.root, shape);

        let list = doc.build_display_list(&view);
        assert_eq!(list.len(), 1);

        match &list.items[0] {
            DisplayItem::FillPath { paint, opacity, .. } => {
                assert_eq!(*paint, Paint::Solid([1.0, 0.0, 0.0, 1.0]));
                assert_eq!(*opacity, 1.0);
            }
            _ => panic!("Expected FillPath"),
        }
    }

    #[test]
    fn test_build_display_list_with_stroke() {
        let mut doc = Document::new();
        let view = View::default();

        let shape = Node::shape("Rect", CorePathData::rect(0.0, 0.0, 100.0, 100.0)).with_style(
            Style::fill_and_stroke(
                CorePaint::rgb(1.0, 0.0, 0.0),
                crate::style::Stroke::black(2.0),
            ),
        );

        doc.add_child(doc.root, shape);

        let list = doc.build_display_list(&view);
        // Should have both fill and stroke
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn shape_style_preserves_isolated_opacity_order_fill_rule_and_stroke_details() {
        let mut doc = Document::new();
        let mut stroke = crate::Stroke::black(3.0);
        stroke.line_cap = crate::LineCap::Round;
        stroke.line_join = crate::LineJoin::Bevel;
        stroke.miter_limit = 6.0;
        stroke.dash_array = vec![5.0, 2.0];
        stroke.dash_offset = 1.0;
        let mut style = Style::fill_and_stroke(CorePaint::white(), stroke);
        style.opacity = 0.5;
        style.fill_rule = crate::FillRule::EvenOdd;
        style.paint_order = crate::PaintOrder::StrokeAndFill;
        doc.add_child(
            doc.root,
            Node::shape("Styled", CorePathData::rect(0.0, 0.0, 10.0, 10.0)).with_style(style),
        );

        let list = doc.build_display_list(&View::default());

        assert_eq!(list.len(), 4);
        assert!(matches!(
            list.items[0],
            DisplayItem::BeginGroup { opacity, .. } if opacity == 0.5
        ));
        let DisplayItem::StrokePath {
            stroke, opacity, ..
        } = &list.items[1]
        else {
            panic!("expected stroke before fill");
        };
        assert_eq!(stroke.line_cap, LineCap::Round);
        assert_eq!(stroke.line_join, LineJoin::Bevel);
        assert_eq!(stroke.miter_limit, 6.0);
        assert_eq!(stroke.dash_array, [5.0, 2.0]);
        assert_eq!(stroke.dash_offset, 1.0);
        assert_eq!(*opacity, 1.0);
        assert!(matches!(
            list.items[2],
            DisplayItem::FillPath {
                fill_rule: FillRule::EvenOdd,
                opacity: 1.0,
                ..
            }
        ));
        assert!(matches!(list.items[3], DisplayItem::EndGroup));
    }

    #[test]
    fn display_list_preserves_resolved_text_layout() {
        let mut doc = Document::new();
        let view = View::default();
        let text = doc
            .add_child(doc.root, Node::text("Label", "iW"))
            .expect("text node should be added");
        doc.set_text_layouts(std::collections::HashMap::from([(
            text,
            crate::TextLayout {
                lines: vec![crate::TextLayoutLine {
                    range: 0..2,
                    x: 4.0,
                    character_count: 2,
                    hard_break: false,
                    positions: vec![(0, 0.0), (1, 3.0), (2, 15.0)],
                }],
                character_width: 7.5,
                line_height: 12.0,
                width: 23.0,
            },
        )]));

        let list = doc.build_display_list(&view);
        let DisplayItem::Text { text, .. } = &list.items[0] else {
            panic!("expected text display item");
        };
        let layout = text.layout.as_ref().expect("resolved layout should be set");

        assert_eq!(
            layout.lines,
            [ResolvedTextLine {
                range: 0..2,
                x: 4.0,
            }]
        );
        assert_eq!(layout.line_height, 12.0);
        assert_eq!(layout.width, 23.0);
    }

    #[test]
    fn test_build_display_list_invisible() {
        let mut doc = Document::new();
        let view = View::default();

        let shape =
            Node::shape("Rect", CorePathData::rect(0.0, 0.0, 100.0, 100.0)).with_visible(false);

        doc.add_child(doc.root, shape);

        let list = doc.build_display_list(&view);
        assert!(list.is_empty());
    }

    #[test]
    fn hidden_container_hides_descendants() {
        let mut doc = Document::new();
        let view = View::default();
        let group = doc
            .add_child(doc.root, Node::group("Hidden").with_visible(false))
            .unwrap();
        doc.add_child(
            group,
            Node::shape("Child", CorePathData::rect(0.0, 0.0, 10.0, 10.0)),
        );

        assert!(doc.build_display_list(&view).is_empty());
    }

    #[test]
    fn container_opacity_is_isolated_while_single_paint_leaf_opacity_is_inline() {
        let mut doc = Document::new();
        let view = View::default();

        // Create a group with 50% opacity
        let mut group = Node::group("Group");
        group.style.opacity = 0.5;
        let group_id = doc.add_child(doc.root, group).unwrap();

        // Create a shape with 50% opacity inside the group
        let mut shape = Node::shape("Rect", CorePathData::rect(0.0, 0.0, 100.0, 100.0));
        shape.style.opacity = 0.5;
        doc.add_child(group_id, shape);

        let list = doc.build_display_list(&view);
        assert_eq!(list.len(), 3);
        assert!(matches!(
            list.items[0],
            DisplayItem::BeginGroup { opacity, .. } if opacity == 0.5
        ));
        assert!(matches!(
            list.items[1],
            DisplayItem::FillPath { opacity, .. } if opacity == 0.5
        ));
        assert!(matches!(list.items[2], DisplayItem::EndGroup));
    }

    #[test]
    fn test_view_transform_applied() {
        let mut doc = Document::new();
        let view = View {
            pan: Vec2::new(100.0, 50.0),
            zoom: 2.0,
        };

        let shape = Node::shape("Rect", CorePathData::rect(0.0, 0.0, 10.0, 10.0));
        doc.add_child(doc.root, shape);

        let list = doc.build_display_list(&view);
        assert_eq!(list.len(), 1);

        match &list.items[0] {
            DisplayItem::FillPath { transform, .. } => {
                // The transform should include the view transform
                let origin = transform.transform_point2(Vec2::ZERO);
                // (0,0) in world -> (0,0) * 2 + (100, 50) = (100, 50) in screen
                assert!((origin.x - 100.0).abs() < 0.001);
                assert!((origin.y - 50.0).abs() < 0.001);
            }
            _ => panic!("Expected FillPath"),
        }
    }
}
