//! Display list generation from the document.

use editor_render::{
    DisplayItem, DisplayList, Paint, PathCmd, PathData, ResolvedTextLayout, ResolvedTextLine,
    Stroke, TextAlignment, TextItem,
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

        for id in self.paint_order().collect::<Vec<_>>() {
            // Extract node data first to avoid borrow conflicts
            let (visible, kind, style) = match self.nodes.get(id) {
                Some(n) => (n.visible && !n.deleted, n.kind.clone(), n.style.clone()),
                None => continue,
            };

            // Visibility is inherited. A hidden frame or group must not leak
            // visible descendants into the display list.
            if !visible
                || self.ancestors(id).any(|ancestor| {
                    self.nodes
                        .get(ancestor)
                        .is_some_and(|node| !node.visible || node.deleted)
                })
            {
                continue;
            }

            let world_transform = self.world_transform(id);
            let screen_transform = screen_from_world * world_transform;
            let opacity = self.compute_opacity_chain(id);

            match kind {
                NodeKind::Group => {
                    // Groups don't render directly
                }
                NodeKind::Shape(path) => {
                    let render_path = convert_path(&path);

                    // Fill
                    if let Some(fill) = &style.fill {
                        items.push(DisplayItem::FillPath {
                            path: render_path.clone(),
                            paint: convert_paint(fill),
                            transform: screen_transform,
                            opacity,
                        });
                    }

                    // Stroke
                    if let Some(stroke) = &style.stroke {
                        items.push(DisplayItem::StrokePath {
                            path: render_path,
                            stroke: convert_stroke(stroke),
                            transform: screen_transform,
                            opacity,
                        });
                    }
                }
                NodeKind::Text(text) => {
                    if let Some(fill) = &style.fill {
                        let layout = self.text_layout(id).map(convert_text_layout);
                        items.push(DisplayItem::Text {
                            text: convert_text_item(&text, fill, layout),
                            transform: screen_transform,
                            opacity,
                        });
                    }
                }
                NodeKind::Frame(frame) => {
                    // Render background if present
                    if let Some(bg) = &frame.background {
                        let rect_path = PathData::rect(0.0, 0.0, frame.width, frame.height);
                        items.push(DisplayItem::FillPath {
                            path: rect_path,
                            paint: convert_paint(bg),
                            transform: screen_transform,
                            opacity,
                        });
                    }
                    // Frames are artboards, not implicit clipping masks.
                    // Children render through the normal paint-order traversal.
                }
            }
        }

        DisplayList { items }
    }

    /// Compute the effective opacity for a node by multiplying through the ancestor chain.
    fn compute_opacity_chain(&self, id: crate::NodeId) -> f32 {
        let mut opacity = self.nodes.get(id).map(|n| n.style.opacity).unwrap_or(1.0);

        for ancestor_id in self.ancestors(id) {
            if let Some(ancestor) = self.nodes.get(ancestor_id) {
                opacity *= ancestor.style.opacity;
            }
        }

        opacity
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
    }
}

/// Convert core stroke to render stroke.
fn convert_stroke(stroke: &crate::style::Stroke) -> Stroke {
    Stroke::new(stroke.width, convert_paint(&stroke.paint))
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
    fn test_opacity_chain() {
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
        assert_eq!(list.len(), 1);

        match &list.items[0] {
            DisplayItem::FillPath { opacity, .. } => {
                // 0.5 * 0.5 = 0.25
                assert!((opacity - 0.25).abs() < 0.001);
            }
            _ => panic!("Expected FillPath"),
        }
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
