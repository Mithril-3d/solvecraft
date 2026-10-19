//! Text styles: alignment, letter and line spacing, a made-up italic, a bold the font lacks, and
//! text that follows a curve of its sketch.

use solidcraft_geom::Vec2;

use super::*;
use crate::{CurveKind, Sketch};

fn v(x: f64, y: f64) -> Vec2 {
    Vec2::new(x, y)
}

/// Every end and control point of the outlines.
fn points(g: &[LinkGeom]) -> Vec<Vec2> {
    g.iter()
        .flat_map(|x| match x {
            LinkGeom::Line(a, b) => vec![*a, *b],
            LinkGeom::Conic { a, apex, b, .. } => vec![*a, *apex, *b],
            LinkGeom::Spline { pts, .. } => pts.clone(),
            _ => vec![],
        })
        .collect()
}

/// The box around the outlines' end points: (min, max).
fn extent(g: &[LinkGeom]) -> (Vec2, Vec2) {
    let pts: Vec<Vec2> = g
        .iter()
        .flat_map(|x| match x {
            LinkGeom::Line(a, b) | LinkGeom::Conic { a, b, .. } => vec![*a, *b],
            _ => vec![],
        })
        .collect();
    let lo = pts.iter().fold(v(f64::MAX, f64::MAX), |m, p| v(m.x.min(p.x), m.y.min(p.y)));
    let hi = pts.iter().fold(v(f64::MIN, f64::MIN), |m, p| v(m.x.max(p.x), m.y.max(p.y)));
    (lo, hi)
}

fn styled(text: &str, style: &TextStyle, path: Option<&[Vec2]>) -> Vec<LinkGeom> {
    text_geometry_styled(None, text, Vec2::ZERO, 10.0, 0.0, style, path).unwrap()
}

#[test]
fn the_plain_style_is_the_text_as_before() {
    assert!(TextStyle::default().is_plain());
    assert_eq!(styled("Ab\nC", &TextStyle::default(), None), text_geometry("Ab\nC", Vec2::ZERO, 10.0, 0.0).unwrap());
}

#[test]
fn lines_align_left_center_or_right_on_the_anchor() {
    let left = extent(&styled("HH", &TextStyle::default(), None));
    let width = left.1.x - left.0.x;
    let center = extent(&styled("HH", &TextStyle { align: TextAlign::Center, ..TextStyle::default() }, None));
    let right = extent(&styled("HH", &TextStyle { align: TextAlign::Right, ..TextStyle::default() }, None));
    // H has the same side bearing both sides: centred, the ink is about the anchor; right, it ends left of it.
    assert!((center.0.x + center.1.x).abs() < 0.05, "{center:?}");
    assert!((right.1.x - right.0.x - width).abs() < 1e-9 && right.1.x < 0.0 && right.1.x > -2.0, "{right:?}");
    assert!((left.0.y - center.0.y).abs() < 1e-12, "alignment moves lines along, not up");
}

#[test]
fn letter_spacing_adds_between_letters_and_line_spacing_scales_the_lines() {
    let plain = extent(&styled("HHH", &TextStyle::default(), None));
    let spaced = extent(&styled("HHH", &TextStyle { letter_spacing: 2.5, ..TextStyle::default() }, None));
    assert!(((spaced.1.x - spaced.0.x) - (plain.1.x - plain.0.x) - 5.0).abs() < 1e-9, "two gaps of 2.5 mm");
    let one = extent(&styled("H\nH", &TextStyle::default(), None));
    let double = extent(&styled("H\nH", &TextStyle { line_spacing: 2.0, ..TextStyle::default() }, None));
    // The second line's baseline: one line distance below the first, then two.
    let gap = -one.0.y;
    assert!((-double.0.y - 2.0 * gap).abs() < 1e-9, "{gap} {double:?}");
    for bad in [TextStyle { line_spacing: 0.0, ..TextStyle::default() }, TextStyle { letter_spacing: f64::NAN, ..TextStyle::default() }] {
        assert!(text_geometry_styled(None, "H", Vec2::ZERO, 10.0, 0.0, &bad, None).is_err());
    }
}

#[test]
fn italic_without_an_italic_face_slants_and_bold_without_a_bold_face_is_an_error() {
    // The built-in font has neither: its I leans 12 degrees, its top moved right by the height times tan 12°.
    let upright = extent(&styled("I", &TextStyle::default(), None));
    let slanted = extent(&styled("I", &TextStyle { italic: true, ..TextStyle::default() }, None));
    assert!((slanted.1.x - upright.1.x - 10.0 * 12_f64.to_radians().tan()).abs() < 1e-6, "{upright:?} {slanted:?}");
    assert!((slanted.0.x - upright.0.x).abs() < 1e-9, "the baseline stays");
    let e = text_geometry_styled(None, "I", Vec2::ZERO, 10.0, 0.0, &TextStyle { bold: true, ..TextStyle::default() }, None).unwrap_err();
    assert!(e.to_string().contains("no bold face"), "{e}");
}

#[test]
fn text_on_a_straight_path_stands_on_it_reading_left_to_right_whichever_way_it_was_drawn() {
    let plain = styled("Hi", &TextStyle::default(), None);
    let on = |path: &[Vec2], reverse: bool| styled("Hi", &TextStyle { path: Some("l".into()), reverse, ..TextStyle::default() }, Some(path));
    let same = |a: &[LinkGeom], b: &[LinkGeom]| points(a).iter().zip(points(b)).all(|(p, q)| p.dist(q) < 1e-9);
    // Along the x axis from the origin: the text as it is at the origin.
    assert!(same(&on(&[v(0.0, 0.0), v(100.0, 0.0)], false), &plain));
    // The same line drawn right to left still reads left to right, from its left end.
    assert!(same(&on(&[v(100.0, 0.0), v(0.0, 0.0)], false), &plain));
    // Reversed: right to left from the right end, upside down below the line.
    let (lo, hi) = extent(&on(&[v(0.0, 0.0), v(100.0, 0.0)], true));
    assert!(hi.y <= 1e-9 && lo.y < -5.0 && hi.x <= 100.0 + 1e-9 && lo.x > 80.0, "{lo:?} {hi:?}");
    // Centred on the path's middle, ended at its end.
    let c = extent(&styled(
        "HH",
        &TextStyle { path: Some("l".into()), align: TextAlign::Center, ..TextStyle::default() },
        Some(&[v(0.0, 0.0), v(100.0, 0.0)]),
    ));
    assert!(((c.0.x + c.1.x) / 2.0 - 50.0).abs() < 0.05, "{c:?}");
}

#[test]
fn text_on_a_circle_stands_outside_it_centred_on_top_and_inside_reversed() {
    let r = 30.0;
    let circle: Vec<Vec2> = (0..=360).map(|i| v(r * (i as f64).to_radians().cos(), r * (i as f64).to_radians().sin())).collect();
    let style = TextStyle { path: Some("c".into()), align: TextAlign::Center, ..TextStyle::default() };
    let outside = styled("TOP", &style, Some(&circle));
    // Every outline point is outside the circle (but for the O dipping below its baseline), and the text is about the top.
    assert!(points(&outside).iter().all(|p| p.len() >= r - 0.3), "outside");
    let (lo, hi) = extent(&outside);
    // (The ink is not quite symmetric: the P has a wide right side bearing.)
    assert!((lo.x + hi.x).abs() < 2.0 && lo.y > 0.0, "{lo:?} {hi:?}");
    let inside = styled("TOP", &TextStyle { reverse: true, ..style }, Some(&circle));
    assert!(points(&inside).iter().all(|p| p.len() <= r + 0.3), "inside");
}

#[test]
fn a_sketch_sets_its_text_on_one_of_its_curves() {
    let mut sk = Sketch::new();
    let a = sk.add_point(v(0.0, 0.0), None).unwrap();
    let b = sk.add_point(v(80.0, 0.0), None).unwrap();
    let line = sk.add_curve(CurveKind::Line { a, b }, None).unwrap();
    let id = sk.curves[line].id.clone();
    let source = |path: &str| LinkSource::Text {
        text: "Hi".into(),
        at: v(5.0, 5.0),
        height: 10.0,
        angle: 0.7,
        font: None,
        style: TextStyle { path: Some(path.into()), ..TextStyle::default() },
        exprs: Default::default(),
    };
    // On the line: `at` and the angle do not place it.
    let on = sk.text_source_geometry(&source(&id)).unwrap();
    assert_eq!(on, styled("Hi", &TextStyle { path: Some("x".into()), ..TextStyle::default() }, Some(&[v(0.0, 0.0), v(80.0, 0.0)])));
    let e = sk.text_source_geometry(&source("nope")).unwrap_err().to_string();
    assert!(e.contains("not a curve of this sketch"), "{e}");
    // A text cannot follow its own (or another text's) outline.
    let text = sk.add_link(LinkKind::Text, source(&id), &on).unwrap();
    let outline = sk.curves.iter().find(|c| c.link.as_deref() == Some(text.as_str())).unwrap().id.clone();
    let e = sk.text_source_geometry(&source(&outline)).unwrap_err().to_string();
    assert!(e.contains("outline of a text"), "{e}");
}

#[test]
fn a_style_is_saved_only_when_it_is_not_plain_and_reads_back() {
    let plain = LinkSource::Text {
        text: "A".into(),
        at: Vec2::ZERO,
        height: 5.0,
        angle: 0.0,
        font: None,
        style: TextStyle::default(),
        exprs: Default::default(),
    };
    let json = serde_json::to_value(&plain).unwrap();
    assert!(json.get("style").is_none(), "{json}");
    let styled = LinkSource::Text {
        text: "A".into(),
        at: Vec2::ZERO,
        height: 5.0,
        angle: 0.0,
        font: None,
        style: TextStyle {
            bold: true,
            italic: true,
            align: TextAlign::Right,
            letter_spacing: -0.5,
            line_spacing: 1.5,
            path: Some("l3".into()),
            reverse: true,
        },
        exprs: Default::default(),
    };
    let json = serde_json::to_value(&styled).unwrap();
    assert_eq!(json["style"]["align"], "right");
    assert_eq!(serde_json::from_value::<LinkSource>(json).unwrap(), styled);
    // Saved without a style (before styles): plain.
    let mut old = serde_json::to_value(&plain).unwrap();
    old.as_object_mut().unwrap().remove("style");
    assert_eq!(serde_json::from_value::<LinkSource>(old).unwrap(), plain);
}

#[test]
fn on_a_closed_curve_left_alignment_starts_at_the_top() {
    let r = 30.0;
    let circle: Vec<Vec2> = (0..=360).map(|i| v(r * (i as f64).to_radians().cos(), r * (i as f64).to_radians().sin())).collect();
    let (lo, hi) = extent(&styled("START", &TextStyle { path: Some("c".into()), ..TextStyle::default() }, Some(&circle)));
    // It runs clockwise from the top: to the right of it, coming down.
    assert!(lo.x > -1.0 && hi.x > 10.0 && hi.y > r, "{lo:?} {hi:?}");
}
