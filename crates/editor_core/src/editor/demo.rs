//! Deterministic starter artwork used by the desktop application's welcome project.

use glam::{Affine2, Vec2};

use super::Editor;
use crate::{
    FontSpec, GuideAxis, Node, NodeId, NodeKind, Paint, PathCmd, PathData, RgbaColor, Stroke,
    Style, TextAlign,
};

const INK: [f32; 4] = [0.043, 0.051, 0.086, 1.0];
const SURFACE: [f32; 4] = [0.082, 0.098, 0.153, 1.0];
const SURFACE_RAISED: [f32; 4] = [0.118, 0.137, 0.204, 1.0];
const PAPER: [f32; 4] = [0.965, 0.973, 1.0, 1.0];
const MUTED: [f32; 4] = [0.576, 0.616, 0.737, 1.0];
const BLUE: [f32; 4] = [0.047, 0.549, 0.914, 1.0];
const VIOLET: [f32; 4] = [0.482, 0.341, 0.949, 1.0];
const MAGENTA: [f32; 4] = [1.0, 0.306, 0.804, 1.0];
const MINT: [f32; 4] = [0.184, 0.890, 0.655, 1.0];
const AMBER: [f32; 4] = [1.0, 0.690, 0.251, 1.0];

pub(super) fn populate(editor: &mut Editor) -> Option<()> {
    let root = editor.document.root;
    let board = add(
        editor,
        root,
        frame_node("Strek Showcase", 680.0, 560.0, INK).with_transform(translation(52.0, 48.0)),
    )?;

    let background = add(editor, board, Node::group("Backdrop"))?;
    add(
        editor,
        background,
        shape(
            "Blue Glow",
            PathData::ellipse(0.0, 0.0, 250.0, 250.0),
            translucent(BLUE, 0.16),
            390.0,
            18.0,
        ),
    )?;
    add(
        editor,
        background,
        shape(
            "Violet Glow",
            PathData::ellipse(0.0, 0.0, 190.0, 190.0),
            translucent(VIOLET, 0.14),
            465.0,
            330.0,
        ),
    )?;
    add(
        editor,
        background,
        shape(
            "Horizon",
            open_path(&[(0.0, 0.0), (568.0, 0.0)]),
            Style::stroke(Stroke::new(1.0, Paint::Solid([0.25, 0.29, 0.42, 0.5]))),
            56.0,
            196.0,
        ),
    )?;

    let hero = add(editor, board, Node::group("Brand Hero"))?;
    let mark = add(
        editor,
        hero,
        Node::group("Strek Mark").with_transform(translation(56.0, 58.0)),
    )?;
    add(
        editor,
        mark,
        shape(
            "Mark Base",
            PathData::ellipse(0.0, 0.0, 104.0, 104.0),
            Style::fill(paint(BLUE)),
            0.0,
            0.0,
        ),
    )?;
    add(
        editor,
        mark,
        shape(
            "Mark Orbit",
            PathData::ellipse(0.0, 0.0, 78.0, 78.0),
            Style::fill(paint(VIOLET)),
            26.0,
            26.0,
        ),
    )?;
    add(
        editor,
        mark,
        shape(
            "Mark Bolt",
            closed_path(&[
                (51.0, 17.0),
                (26.0, 55.0),
                (46.0, 55.0),
                (37.0, 87.0),
                (78.0, 43.0),
                (56.0, 43.0),
            ]),
            Style::fill(paint(PAPER)),
            0.0,
            0.0,
        ),
    )?;

    add(
        editor,
        hero,
        text(
            "Eyebrow",
            "NATIVE VECTOR EDITOR",
            13.0,
            700,
            BLUE,
            182.0,
            57.0,
        ),
    )?;
    add(
        editor,
        hero,
        text("Wordmark", "STREK", 58.0, 700, PAPER, 178.0, 76.0),
    )?;
    add(
        editor,
        hero,
        text(
            "Tagline",
            "Shape ideas. Ship pixels.",
            18.0,
            400,
            MUTED,
            182.0,
            148.0,
        ),
    )?;

    let features = add(editor, board, Node::group("Feature Cards"))?;
    add_precision_card(editor, features, 56.0)?;
    add_color_card(editor, features, 252.0)?;
    add_automation_card(editor, features, 448.0)?;

    add(
        editor,
        board,
        shape(
            "Signal Orb",
            PathData::ellipse(0.0, 0.0, 48.0, 48.0),
            Style::fill(paint(MAGENTA)),
            566.0,
            78.0,
        ),
    )?;

    let palette = add(editor, board, Node::group("Brand Palette"))?;
    add(
        editor,
        palette,
        text(
            "Palette Label",
            "CORE PALETTE",
            11.0,
            700,
            MUTED,
            56.0,
            462.0,
        ),
    )?;
    for (index, (name, color)) in [
        ("Ink", INK),
        ("Surface", SURFACE_RAISED),
        ("Electric Blue", BLUE),
        ("Violet", VIOLET),
        ("Magenta", MAGENTA),
        ("Mint", MINT),
        ("Amber", AMBER),
    ]
    .into_iter()
    .enumerate()
    {
        let style = if name == "Ink" {
            Style::fill_and_stroke(paint(color), Stroke::new(1.0, paint(SURFACE_RAISED)))
        } else {
            Style::fill(paint(color))
        };
        add(
            editor,
            palette,
            shape(
                name,
                PathData::rect(0.0, 0.0, 72.0, 28.0),
                style,
                56.0 + index as f32 * 82.0,
                488.0,
            ),
        )?;
    }

    let core = editor.add_color_group("Foundation").ok()?;
    let signals = editor.add_color_group("Signals").ok()?;
    for (group, name, color) in [
        (core, "Ink", INK),
        (core, "Surface", SURFACE),
        (core, "Paper", PAPER),
        (signals, "Electric Blue", BLUE),
        (signals, "Violet", VIOLET),
        (signals, "Magenta", MAGENTA),
        (signals, "Mint", MINT),
        (signals, "Amber", AMBER),
    ] {
        editor
            .add_saved_color(Some(group), Some(name), rgba(color)?)
            .ok()?;
    }

    editor
        .set_grid_settings(crate::GridSettings::new(8.0, 5).ok()?)
        .ok()?;
    for (axis, position) in [
        (GuideAxis::Vertical, 108.0),
        (GuideAxis::Vertical, 676.0),
        (GuideAxis::Horizontal, 104.0),
        (GuideAxis::Horizontal, 560.0),
    ] {
        editor.add_guide(axis, position).ok()?;
    }

    editor.layer_panel.collapse(background);
    editor.layer_panel.collapse(hero);
    editor.layer_panel.collapse(features);
    editor.layer_panel.collapse(palette);
    Some(())
}

fn add_precision_card(editor: &mut Editor, parent: NodeId, x: f32) -> Option<()> {
    let card = add(
        editor,
        parent,
        frame_node("Precision Card", 176.0, 190.0, SURFACE).with_transform(translation(x, 226.0)),
    )?;
    add(
        editor,
        card,
        shape(
            "Crosshair Ring",
            PathData::ellipse(0.0, 0.0, 58.0, 58.0),
            Style::fill_and_stroke(
                Paint::Solid([0.047, 0.549, 0.914, 0.13]),
                Stroke::new(2.0, paint(BLUE)),
            ),
            24.0,
            24.0,
        ),
    )?;
    add(
        editor,
        card,
        shape(
            "Crosshair",
            open_path(&[
                (29.0, 0.0),
                (29.0, 58.0),
                (29.0, 29.0),
                (0.0, 29.0),
                (58.0, 29.0),
            ]),
            Style::stroke(Stroke::new(1.5, paint(PAPER))),
            24.0,
            24.0,
        ),
    )?;
    add_card_copy(editor, card, "Precision", "Grid, guides & snapping")
}

fn add_color_card(editor: &mut Editor, parent: NodeId, x: f32) -> Option<()> {
    let card = add(
        editor,
        parent,
        frame_node("Color Card", 176.0, 190.0, SURFACE).with_transform(translation(x, 226.0)),
    )?;
    for (name, color, x, y) in [
        ("Blue Channel", BLUE, 24.0, 25.0),
        ("Violet Channel", VIOLET, 50.0, 25.0),
        ("Magenta Channel", MAGENTA, 37.0, 48.0),
    ] {
        add(
            editor,
            card,
            shape(
                name,
                PathData::ellipse(0.0, 0.0, 48.0, 48.0),
                translucent(color, 0.82),
                x,
                y,
            ),
        )?;
    }
    add_card_copy(editor, card, "Color systems", "HSV, HSL, RGB & OKLCH")
}

fn add_automation_card(editor: &mut Editor, parent: NodeId, x: f32) -> Option<()> {
    let card = add(
        editor,
        parent,
        frame_node("Automation Card", 176.0, 190.0, SURFACE).with_transform(translation(x, 226.0)),
    )?;
    add(
        editor,
        card,
        shape(
            "Automation Path",
            open_path(&[(20.0, 55.0), (48.0, 27.0), (78.0, 55.0), (108.0, 25.0)]),
            Style::stroke(Stroke::new(2.5, paint(MINT))),
            20.0,
            20.0,
        ),
    )?;
    for (index, (x, y)) in [(20.0, 55.0), (48.0, 27.0), (78.0, 55.0), (108.0, 25.0)]
        .into_iter()
        .enumerate()
    {
        add(
            editor,
            card,
            shape(
                format!("Automation Node {}", index + 1),
                PathData::ellipse(0.0, 0.0, 10.0, 10.0),
                Style::fill(paint(PAPER)),
                35.0 + x,
                15.0 + y,
            ),
        )?;
    }
    add_card_copy(editor, card, "Automation", "CLI, AppleScript & MCP")
}

fn add_card_copy(editor: &mut Editor, card: NodeId, title: &str, detail: &str) -> Option<()> {
    add(
        editor,
        card,
        text("Card Title", title, 18.0, 700, PAPER, 24.0, 112.0),
    )?;
    add(
        editor,
        card,
        text("Card Detail", detail, 11.0, 400, MUTED, 24.0, 144.0),
    )?;
    Some(())
}

fn add(editor: &mut Editor, parent: NodeId, node: Node) -> Option<NodeId> {
    editor.document.add_child(parent, node)
}

fn frame_node(name: &str, width: f32, height: f32, background: [f32; 4]) -> Node {
    let mut node = Node::frame(name, width, height);
    if let Some(frame) = node.frame_data_mut() {
        frame.background = Some(paint(background));
    }
    node
}

fn shape(name: impl Into<String>, path: PathData, style: Style, x: f32, y: f32) -> Node {
    Node::shape(name, path)
        .with_style(style)
        .with_transform(translation(x, y))
}

fn text(
    name: &str,
    content: &str,
    size: f32,
    weight: u16,
    color: [f32; 4],
    x: f32,
    y: f32,
) -> Node {
    let mut node = Node::text(name, content)
        .with_style(Style::fill(paint(color)))
        .with_transform(translation(x, y));
    if let NodeKind::Text(data) = &mut node.kind {
        data.font = FontSpec::new("sans-serif").with_weight(weight);
        data.font_size = size;
        data.line_height = 1.1;
        data.align = TextAlign::Left;
    }
    node
}

fn translation(x: f32, y: f32) -> Affine2 {
    Affine2::from_translation(Vec2::new(x, y))
}

fn paint(color: [f32; 4]) -> Paint {
    Paint::Solid(color)
}

fn rgba(color: [f32; 4]) -> Option<RgbaColor> {
    RgbaColor::from_array(color).ok()
}

fn translucent(color: [f32; 4], opacity: f32) -> Style {
    let mut style = Style::fill(paint(color));
    style.opacity = opacity;
    style
}

fn closed_path(points: &[(f32, f32)]) -> PathData {
    let Some((&first, rest)) = points.split_first() else {
        return PathData::new();
    };
    let commands = std::iter::once(PathCmd::MoveTo(point(first)))
        .chain(rest.iter().map(|&value| PathCmd::LineTo(point(value))))
        .chain(std::iter::once(PathCmd::Close))
        .collect::<Vec<_>>();
    PathData::from_commands(&commands)
}

fn open_path(points: &[(f32, f32)]) -> PathData {
    let Some((&first, rest)) = points.split_first() else {
        return PathData::new();
    };
    let commands = std::iter::once(PathCmd::MoveTo(point(first)))
        .chain(rest.iter().map(|&value| PathCmd::LineTo(point(value))))
        .collect::<Vec<_>>();
    PathData::from_commands(&commands)
}

fn point((x, y): (f32, f32)) -> Vec2 {
    Vec2::new(x, y)
}
