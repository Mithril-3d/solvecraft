//! Interactive sketch tools: click points on the sketch plane (or pick sketch entities) and the
//! tool runs the matching command. Tools are UI state only; the work is done by commands.

use egui::{Pos2, Stroke};
use serde_json::{Value, json};
use solvecraft_engine::geom::Vec2;

use crate::SolveApp;
use crate::viewport::{Hit, Proj, pick, sketch_point_at};

#[cfg(test)]
#[path = "sketch_display_preview_tests.rs"]
mod display_preview_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Click points; `n` points complete the shape (0 = chain until finished).
    Draw(usize),
    /// Pick sketch entities; `n` picks run the command.
    Pick(usize),
    /// Pick one straight line, or two distinct points to align.
    HorizontalVertical,
    Dimension,
    /// A tool from `sketch_tools` (it handles its own clicks).
    Ext,
}

#[derive(Clone, Debug)]
pub struct Tool {
    pub cmd: &'static str,
    pub kind: Kind,
    /// Clicked points (sketch coordinates) and the sketch point they snapped to.
    pub pts: Vec<(Vec2, Option<String>)>,
    pub hover: Option<(Vec2, Option<String>)>,
    /// Picked entity ids.
    pub picks: Vec<String>,
    /// Inline dimension boxes for the shape being drawn, the point count they were made for,
    /// and whether the first box should take the keyboard.
    pub dims: Vec<crate::sketch_dims::DimBox>,
    /// Locked inputs from earlier stages of this shape (arc radius, slot length).
    pub dim_history: Vec<crate::sketch_dims::DimBox>,
    pub dims_stage: usize,
    pub dims_focus: bool,
}

impl Tool {
    pub fn for_command(id: &str) -> Option<Tool> {
        let (cmd, kind): (&'static str, Kind) = match id {
            "sketch.line" => ("sketch.line", Kind::Draw(0)),
            "sketch.rectangle.two_point" => ("sketch.rectangle.two_point", Kind::Draw(2)),
            "sketch.rectangle.center" => ("sketch.rectangle.center", Kind::Draw(2)),
            "sketch.rectangle.three_point" => ("sketch.rectangle.three_point", Kind::Draw(3)),
            "sketch.circle.center" => ("sketch.circle.center", Kind::Draw(2)),
            "sketch.circle.two_point" => ("sketch.circle.two_point", Kind::Draw(2)),
            "sketch.circle.three_point" => ("sketch.circle.three_point", Kind::Draw(3)),
            "sketch.arc.three_point" => ("sketch.arc.three_point", Kind::Draw(3)),
            "sketch.arc.center_point" => ("sketch.arc.center_point", Kind::Draw(3)),
            "sketch.polygon.inscribed" => ("sketch.polygon.inscribed", Kind::Draw(2)),
            "sketch.polygon.circumscribed" => ("sketch.polygon.circumscribed", Kind::Draw(2)),
            "sketch.polygon.edge" => ("sketch.polygon.edge", Kind::Draw(2)),
            "sketch.slot.center_to_center" => ("sketch.slot.center_to_center", Kind::Draw(3)),
            "sketch.slot.overall" => ("sketch.slot.overall", Kind::Draw(3)),
            "sketch.point" => ("sketch.point", Kind::Draw(1)),
            "sketch.dimension" => ("sketch.dimension", Kind::Dimension),
            "sketch.constraint.horizontal_vertical" => ("sketch.constraint.horizontal_vertical", Kind::HorizontalVertical),
            "sketch.constraint.fix" => ("sketch.constraint.fix", Kind::Pick(1)),
            "sketch.constraint.coincident" => ("sketch.constraint.coincident", Kind::Pick(2)),
            "sketch.constraint.tangent" => ("sketch.constraint.tangent", Kind::Pick(2)),
            "sketch.constraint.equal" => ("sketch.constraint.equal", Kind::Pick(2)),
            "sketch.constraint.parallel" => ("sketch.constraint.parallel", Kind::Pick(2)),
            "sketch.constraint.perpendicular" => ("sketch.constraint.perpendicular", Kind::Pick(2)),
            "sketch.constraint.concentric" => ("sketch.constraint.concentric", Kind::Pick(2)),
            "sketch.constraint.collinear" => ("sketch.constraint.collinear", Kind::Pick(2)),
            "sketch.constraint.midpoint" => ("sketch.constraint.midpoint", Kind::Pick(2)),
            "sketch.constraint.symmetry" => ("sketch.constraint.symmetry", Kind::Pick(3)),
            _ => return crate::sketch_tools::tool_for(id),
        };
        Some(Tool {
            cmd,
            kind,
            pts: Vec::new(),
            hover: None,
            picks: Vec::new(),
            dims: Vec::new(),
            dim_history: Vec::new(),
            dims_stage: usize::MAX,
            dims_focus: false,
        })
    }
}

pub fn hint(id: &str) -> String {
    match id {
        "sketch.line" => "Line: click points; right-click or Esc to finish".into(),
        "sketch.rectangle.two_point" => "Rectangle: click two opposite corners".into(),
        "sketch.rectangle.center" => "Center rectangle: click the centre, then a corner".into(),
        "sketch.rectangle.three_point" => "3-point rectangle: click two points of one edge, then the width".into(),
        "sketch.circle.center" => "Circle: click the centre, then a point on the circle".into(),
        "sketch.arc.three_point" => "Arc: click start, end, then a point on the arc".into(),
        "sketch.arc.center_point" => "Arc: click centre and start, then click end or type a signed sweep".into(),
        "sketch.dimension" => "Dimension: pick one or two entities, then click to place the value".into(),
        "sketch.constraint.horizontal_vertical" => "Horizontal/Vertical: pick a line or two points to align".into(),
        "sketch.slot.center_to_center" | "sketch.slot.overall" => "Slot: click both ends, then the width".into(),
        _ if id.starts_with("sketch.constraint.") => "Constraint: pick the sketch entities".into(),
        _ => crate::sketch_tools::hint(id).unwrap_or_else(|| "Click in the sketch".into()),
    }
}

fn arg(p: &(Vec2, Option<String>)) -> Value {
    match &p.1 {
        Some(id) => json!(id),
        None => json!([p.0.x, p.0.y]),
    }
}

fn xy(p: &(Vec2, Option<String>)) -> Value {
    json!([p.0.x, p.0.y])
}

pub fn on_hover(app: &mut SolveApp, proj: &Proj, pos: Pos2) {
    crate::sketch3d::on_hover(app, proj, pos);
    let h = if let Some(point) = app.tool.as_ref().and_then(|tool| crate::sketch_tools::periodic_start_snap(app, tool, proj, pos)) {
        Some((point, Some("Close smooth loop".into())))
    } else if let Some(tool) = app.tool.as_ref().filter(|t| t.cmd == "sketch.fillet" || t.cmd.starts_with("sketch.chamfer.")) {
        crate::sketch_tools::corner_point(app, proj, pos, tool.cmd).map(|(at, id)| (at, Some(id)))
    } else {
        sketch_point_at(app, proj, pos)
    };
    if let Some(t) = app.tool.as_mut() {
        t.hover = h;
    }
}

pub fn on_click(app: &mut SolveApp, proj: &Proj, pos: Pos2) {
    // A click away dismisses the inline editor; it must not also pick the first entity of
    // the next dimension before that cancellation has been processed.
    if crate::dim_view::editing() {
        return;
    }
    let Some(mut tool) = app.tool.take() else { return };
    // In a 3D sketch the drawing tools take points off the plane.
    if crate::sketch3d::on_click(app, &mut tool, proj, pos) {
        app.tool = Some(tool);
        return;
    }
    match tool.kind {
        Kind::Draw(_) => {
            if let Some(mut p) = sketch_point_at(app, proj, pos) {
                // Typed values win over the cursor; lines snap to horizontal/vertical.
                if tool.dims.iter().any(|b| b.locked) || (tool.cmd == "sketch.line" && p.1.is_none()) {
                    p = (crate::sketch_dims::effective(app, &tool, p.0), None);
                }
                app.tool = Some(tool);
                place(app, p);
                return;
            }
        }
        Kind::HorizontalVertical => {
            let hits = pick(app, proj, pos);
            let hit = hits.iter().find(|hit| match hit {
                Hit::SketchPoint { sketch, .. } | Hit::SketchCurve { sketch, .. } => Some(*sketch) == app.session.active_sketch,
                _ => false,
            });
            match hit {
                Some(Hit::SketchPoint { id, .. }) => {
                    if let Some(first) = tool.picks.first() {
                        if first != id {
                            let _ = app.run(tool.cmd, json!({"points": [first, id]}));
                            tool.picks.clear();
                        }
                    } else {
                        tool.picks.push(id.clone());
                    }
                }
                Some(Hit::SketchCurve { id, straight: true, .. }) if tool.picks.is_empty() => {
                    let _ = app.run(tool.cmd, json!({"line": id}));
                }
                _ => {}
            }
        }
        Kind::Pick(n) => {
            let hits = pick(app, proj, pos);
            if let Some(id) = hits.iter().find_map(|h| match h {
                Hit::SketchPoint { id, .. } | Hit::SketchCurve { id, .. } => Some(id.clone()),
                _ => None,
            }) {
                tool.picks.push(id);
            }
            if tool.picks.len() >= n {
                run_pick(app, &tool);
                tool.picks.clear();
            }
        }
        Kind::Ext => crate::sketch_tools::on_click(app, &mut tool, proj, pos),
        Kind::Dimension => {
            let hits = pick(app, proj, pos);
            let hit = hits.iter().find_map(|h| match h {
                Hit::SketchPoint { id, .. } | Hit::SketchCurve { id, .. } => Some(id.clone()),
                _ => None,
            });
            if let Some(id) = hit {
                if !tool.picks.contains(&id) && tool.picks.len() < 2 {
                    tool.picks.push(id);
                }
            } else if !tool.picks.is_empty()
                && let Some((at, _)) = sketch_point_at(app, proj, pos)
            {
                match crate::dim_view::begin_dimension(app, tool.picks.clone(), at) {
                    Ok(()) => tool.picks.clear(),
                    Err(error) => app.set_status(error, true),
                }
            }
        }
    }
    app.tool = Some(tool);
}

/// Put down the next point of the shape being drawn; the last point runs the command (with
/// the typed values as dimension constraints).
pub fn place(app: &mut SolveApp, p: (Vec2, Option<String>)) {
    let Some(mut tool) = app.tool.take() else { return };
    let Kind::Draw(n) = tool.kind else {
        app.tool = Some(tool);
        return;
    };
    if let Err(error) = crate::sketch_dims::validate(app, &tool) {
        app.set_status(error, true);
        app.tool = Some(tool);
        return;
    }
    tool.pts.push(p);
    if tool.cmd == "sketch.line" {
        if tool.pts.len() >= 2 {
            let (a, b) = (tool.pts[tool.pts.len() - 2].clone(), tool.pts[tool.pts.len() - 1].clone());
            let angled = crate::sketch_dims::has_locked(&tool, "Angle");
            let dimensioned = tool.dims.iter().any(|dimension| dimension.locked);
            if let Ok(v) = draw(app, &tool, json!({"points": [arg(&a), arg(&b)], "infer": !angled})) {
                let curves: Vec<String> = v["curves"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_string)).collect();
                // Chain: the next segment starts at this one's end point.
                if let Some(id) = curves.first() {
                    if angled {
                        crate::inference::clear();
                    } else {
                        // Endpoint inference is part of this drawing. Preserve its original
                        // undo history even when the history is already at the engine's cap.
                        let history = dimensioned.then(|| std::mem::take(&mut app.session.undo));
                        crate::inference::commit(app, id, b.0);
                        if let Some(history) = history {
                            app.session.undo = history;
                        }
                    }
                    let last = tool.pts.len() - 1;
                    tool.pts[last].1 = Some(format!("{id}.end"));
                    // The chain needs only its current endpoint, even after many segments.
                    tool.pts = vec![tool.pts[last].clone()];
                }
                crate::sketch_dims::reset(&mut tool);
            } else {
                tool.pts.pop();
            }
        }
    } else if n > 0 && tool.pts.len() >= n {
        match run_shape(app, &tool) {
            Ok(_) => {
                tool.pts.clear();
                crate::sketch_dims::reset(&mut tool);
            }
            Err(error) => {
                app.set_status(error, true);
                tool.pts.pop();
            }
        }
    }
    app.tool = Some(tool);
}

/// Geometry and typed constraints are committed atomically, as one undo step.
fn draw(app: &mut SolveApp, t: &Tool, params: Value) -> Result<Value, String> {
    let dimensions = crate::sketch_dims::dimensions(t);
    if dimensions.as_object().is_none_or(|values| values.is_empty()) {
        app.run(t.cmd, params)
    } else {
        let result = app.run("sketch.draw.dimensioned", json!({"command": t.cmd, "params": params, "dimensions": dimensions}))?;
        // The internal wrapper has no toolbar category; apply the original creator's line
        // type while retaining the draw's undo history, including at the history limit.
        let history = std::mem::take(&mut app.session.undo);
        crate::sketch_palette::apply_mode(app, t.cmd, &result);
        app.session.undo = history;
        Ok(result)
    }
}

fn run_shape(app: &mut SolveApp, t: &Tool) -> Result<Vec<String>, String> {
    let p = &t.pts;
    let d = |a: usize, b: usize| p[a].0.dist(p[b].0);
    let sides = crate::sketch_dims::locked_value(app, t, "Sides").map_or(6, |v| v as usize);
    let params = match t.cmd {
        "sketch.rectangle.two_point" => json!({"p0": xy(&p[0]), "p1": xy(&p[1])}),
        "sketch.rectangle.center" => json!({"center": xy(&p[0]), "corner": xy(&p[1])}),
        "sketch.rectangle.three_point" => json!({"p0": xy(&p[0]), "p1": xy(&p[1]), "p2": xy(&p[2])}),
        "sketch.circle.center" => json!({"center": arg(&p[0]), "radius": d(0, 1)}),
        "sketch.circle.two_point" => json!({"p0": xy(&p[0]), "p1": xy(&p[1])}),
        "sketch.circle.three_point" => json!({"p0": xy(&p[0]), "p1": xy(&p[1]), "p2": xy(&p[2])}),
        "sketch.arc.three_point" => json!({"start": arg(&p[0]), "end": arg(&p[1]), "through": xy(&p[2])}),
        "sketch.arc.center_point" => match crate::sketch_dims::locked_value(app, t, "Sweep") {
            Some(sweep) => json!({"center": arg(&p[0]), "start": arg(&p[1]), "sweep": sweep.to_degrees()}),
            None => json!({"center": arg(&p[0]), "start": arg(&p[1]), "end": xy(&p[2])}),
        },
        "sketch.polygon.inscribed" | "sketch.polygon.circumscribed" => {
            let a = (p[1].0 - p[0].0).angle().to_degrees();
            json!({"center": xy(&p[0]), "radius": d(0, 1), "sides": sides, "angle": a})
        }
        "sketch.polygon.edge" => json!({"p0": xy(&p[0]), "p1": xy(&p[1]), "sides": sides}),
        "sketch.slot.center_to_center" | "sketch.slot.overall" => {
            let dir = (p[1].0 - p[0].0).normalized().unwrap_or(Vec2::X);
            let w = 2.0 * dir.cross(p[2].0 - p[0].0).abs();
            json!({"p0": xy(&p[0]), "p1": xy(&p[1]), "width": w.max(1e-3)})
        }
        "sketch.point" => json!({"point": xy(&p[0])}),
        _ => return Err(format!("drawing {} is not supported", t.cmd)),
    };
    let result = draw(app, t, params)?;
    Ok(result["curves"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_string)).collect())
}

/// Only consume a complete, unambiguous selection; command validation owns geometry.
pub(crate) fn constraint_preselection(command: &str, selection: &[solvecraft_engine::Sel]) -> Option<Value> {
    use solvecraft_engine::Sel::{SketchCurve, SketchPoint};
    match (command, selection) {
        ("sketch.constraint.horizontal_vertical", [SketchCurve { id }]) => Some(json!({"line":id})),
        ("sketch.constraint.horizontal_vertical", [SketchPoint { id: a }, SketchPoint { id: b }]) if a != b => Some(json!({"points":[a,b]})),
        ("sketch.constraint.coincident", [a, b]) => {
            let entity = |s: &solvecraft_engine::Sel| match s {
                SketchPoint { id } | SketchCurve { id } => Some(id.clone()),
                _ => None,
            };
            let (a, b) = (entity(a)?, entity(b)?);
            (a != b).then(|| json!({"a":a,"b":b}))
        }
        (
            "sketch.constraint.midpoint",
            [SketchPoint { id: point }, SketchCurve { id: line }] | [SketchCurve { id: line }, SketchPoint { id: point }],
        ) => Some(json!({"point":point,"line":line})),
        ("sketch.constraint.symmetry", [a, b, c]) => {
            let mut points = Vec::new();
            let mut lines = Vec::new();
            for entity in [a, b, c] {
                match entity {
                    SketchPoint { id } => points.push(id),
                    SketchCurve { id } => lines.push(id),
                    _ => return None,
                }
            }
            match (points.as_slice(), lines.as_slice()) {
                ([a, b], [line]) if a != b => Some(json!({"a":a,"b":b,"line":line})),
                _ => None,
            }
        }
        _ => None,
    }
}

fn run_pick(app: &mut SolveApp, t: &Tool) {
    let k = &t.picks;
    let params = match t.cmd {
        "sketch.constraint.fix" => json!({"entity": k[0]}),
        "sketch.constraint.coincident" => json!({"a": k[0], "b": k[1]}),
        "sketch.constraint.midpoint" => json!({"point": k[0], "line": k[1]}),
        "sketch.constraint.symmetry" => json!({"a": k[0], "b": k[1], "line": k[2]}),
        _ => json!({"a": k[0], "b": k[1]}),
    };
    let _ = app.run(t.cmd, params);
}

/// Right click / Esc: end the current chain (line) or cancel the tool.
pub fn finish(app: &mut SolveApp) {
    crate::dim_view::cancel();
    if crate::sketch3d::finish(app) {
        return;
    }
    if let Some(mut t) = app.tool.take_if(|t| t.kind == Kind::Ext) {
        if crate::sketch_tools::finish(app, &mut t) {
            app.tool = Some(t);
        }
        return;
    }
    if let Some(t) = app.tool.as_mut()
        && !t.pts.is_empty()
    {
        t.pts.clear();
        t.picks.clear();
        crate::sketch_dims::reset(t);
        return;
    }
    app.tool = None;
}

/// Rubber-band preview of the shape being drawn.
pub(crate) fn paint_preview_curve(
    sketch: &solvecraft_engine::sketch::Sketch,
    plane: &solvecraft_engine::geom::Plane,
    painter: &egui::Painter,
    proj: &Proj,
    stroke: Stroke,
) {
    paint_curve(sketch, 0, plane, painter, proj, stroke, &mut solvecraft_engine::render::SamplingBudget::new(4096, 65_536));
}

pub(crate) fn paint_curve(
    sketch: &solvecraft_engine::sketch::Sketch,
    index: usize,
    plane: &solvecraft_engine::geom::Plane,
    painter: &egui::Painter,
    proj: &Proj,
    stroke: Stroke,
    budget: &mut solvecraft_engine::render::SamplingBudget,
) {
    for part in crate::sketch_display::curve_with_budget(sketch, index, plane, proj, painter.ctx().pixels_per_point(), budget).parts {
        let screen: Vec<_> = part.into_iter().filter_map(|point| proj.to_screen(point)).collect();
        if screen.len() >= 2 {
            painter.add(egui::Shape::line(screen, stroke));
        }
    }
}

/// Temporary analytic geometry uses the same exact span extraction as committed curves.
/// It never solves constraints or modifies the document.
pub(crate) fn preview_curve(t: &Tool, hover: Vec2, sweep: Option<f64>) -> Option<solvecraft_engine::sketch::Sketch> {
    use solvecraft_engine::sketch::{CurveKind, Sketch};
    if !hover.is_finite() || t.pts.len() >= solvecraft_engine::sketch::MAX_SPLINE_POINTS || t.pts.iter().any(|p| !p.0.is_finite()) {
        return None;
    }
    let mut sk = Sketch::new();
    let pts: Vec<_> = t.pts.iter().map(|p| p.0).collect();
    match (t.cmd, pts.as_slice()) {
        ("sketch.circle.center" | "sketch.polygon.inscribed" | "sketch.polygon.circumscribed", [c]) => {
            sk.add_circle(*c, c.dist(hover), None, None).ok()?;
        }
        ("sketch.circle.two_point", [a]) => {
            sk.add_circle((*a + hover) * 0.5, a.dist(hover) * 0.5, None, None).ok()?;
        }
        ("sketch.circle.three_point" | "sketch.arc.three_point", [a, b]) => {
            let ab = *b - *a;
            let ah = hover - *a;
            let d = 2.0 * ab.cross(ah);
            if !d.is_finite() || d.abs() < 1e-12 {
                return None;
            }
            let centre = *a + Vec2::new(ah.y * ab.len2() - ab.y * ah.len2(), ab.x * ah.len2() - ah.x * ab.len2()) / d;
            if t.cmd == "sketch.circle.three_point" {
                sk.add_circle(centre, centre.dist(*a), None, None).ok()?;
            } else {
                let angle = |p: Vec2| (p - centre).angle();
                let span = |p: Vec2| (angle(p) - angle(*a)).rem_euclid(std::f64::consts::TAU);
                let ccw = span(hover) < span(*b);
                sk.add_arc(centre, if ccw { *a } else { *b }, if ccw { *b } else { *a }, [None; 3], None).ok()?;
                if let Some(curve) = sk.curves.first_mut() {
                    curve.reversed = !ccw;
                }
            }
        }
        ("sketch.arc.center_point", [c, a]) => {
            let start = (*a - *c).angle();
            let sweep = sweep.unwrap_or_else(|| ((hover - *c).angle() - start).rem_euclid(std::f64::consts::TAU));
            if !sweep.is_finite() || sweep.abs() <= 1e-9 || sweep.abs() >= std::f64::consts::TAU {
                return None;
            }
            let b = *c + Vec2::from_angle(start + sweep) * c.dist(*a);
            sk.add_arc(*c, if sweep > 0.0 { *a } else { b }, if sweep > 0.0 { b } else { *a }, [None; 3], None).ok()?;
            if let Some(curve) = sk.curves.first_mut() {
                curve.reversed = sweep < 0.0;
            }
        }
        ("sketch.ellipse", [c, m]) => {
            let u = (*m - *c).normalized()?;
            let d = hover - *c;
            let k = 1.0 - (d.dot(u) / c.dist(*m)).powi(2);
            if !k.is_finite() || k <= 1e-12 {
                return None;
            }
            let r = d.cross(u).abs() / k.sqrt();
            let c = sk.add_point(*c, None).ok()?;
            let m = sk.add_point(*m, None).ok()?;
            sk.add_curve(CurveKind::Ellipse { c, m, r }, None).ok()?;
        }
        ("sketch.spline.fit_point" | "sketch.spline.control_point" | "sketch.spline.control_point_5", points) if !points.is_empty() => {
            let periodic = t.cmd == "sketch.spline.fit_point" && points.len() >= 3 && points.first() == Some(&hover);
            let mut indices = points.iter().map(|p| sk.add_point(*p, None).ok()).collect::<Option<Vec<_>>>()?;
            if periodic {
                indices.push(*indices.first()?);
            } else if points.last() != Some(&hover) {
                indices.push(sk.add_point(hover, None).ok()?);
            }
            sk.add_curve(
                CurveKind::Spline {
                    pts: indices,
                    control: t.cmd != "sketch.spline.fit_point",
                    degree: if t.cmd == "sketch.spline.control_point_5" { 5 } else { 3 },
                    periodic,
                },
                None,
            )
            .ok()?;
        }
        _ => return None,
    }
    Some(sk)
}

pub fn preview(app: &SolveApp, t: &Tool, painter: &egui::Painter, proj: &Proj) {
    if crate::sketch3d::preview(app, t, painter, proj) {
        return;
    }
    if t.kind == Kind::Ext {
        return crate::sketch_tools::preview(app, t, painter, proj);
    }
    let Some(sid) = app.session.active_sketch else { return };
    let st = app.session.world_state();
    let Some(ss) = st.sketch(sid) else { return };
    let to = |p: Vec2| proj.to_screen(ss.plane.to_world(p));
    let tk = crate::theme::Tokens::get();
    let stroke = Stroke::new(1.5, tk.rubber_band);
    if let Some((hp, snap)) = &t.hover
        && let Some(sp) = to(*hp)
    {
        painter.circle_stroke(sp, if snap.is_some() { 6.0 } else { 3.0 }, stroke);
        painter.text(
            sp + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            format!("{:.2}, {:.2}", hp.x, hp.y),
            egui::FontId::proportional(11.0),
            tk.text_dim,
        );
    }
    let Some((h, _)) = t.hover.clone() else { return };
    let h = crate::sketch_dims::effective(app, t, h);
    if let Some(curve) = preview_curve(t, h, crate::sketch_dims::locked_value(app, t, "Sweep")) {
        paint_preview_curve(&curve, &ss.plane, painter, proj, stroke);
        return;
    }
    let pts: Vec<Vec2> = t.pts.iter().map(|p| p.0).collect();
    let line = |a: Vec2, b: Vec2| {
        if let (Some(x), Some(y)) = (to(a), to(b)) {
            painter.line_segment([x, y], stroke);
        }
    };
    match (t.cmd, pts.as_slice()) {
        ("sketch.line", [.., last]) => {
            line(*last, h);
            // Horizontal/vertical inference marker.
            if let Some(horizontal) = crate::sketch_dims::axis_aligned(*last, h)
                && let Some(m) = to((*last + h) * 0.5)
            {
                let g = if horizontal { egui::vec2(0.0, -12.0) } else { egui::vec2(12.0, 0.0) };
                painter.text(
                    m + g,
                    egui::Align2::CENTER_CENTER,
                    if horizontal { "H" } else { "V" },
                    egui::FontId::proportional(10.0),
                    tk.sketch_accent,
                );
            }
        }
        ("sketch.rectangle.two_point", [a]) => {
            let (b, c) = (Vec2::new(h.x, a.y), Vec2::new(a.x, h.y));
            line(*a, b);
            line(b, h);
            line(h, c);
            line(c, *a);
        }
        ("sketch.rectangle.center", [c]) => {
            let d = h - *c;
            let k =
                [Vec2::new(c.x - d.x, c.y - d.y), Vec2::new(c.x + d.x, c.y - d.y), Vec2::new(c.x + d.x, c.y + d.y), Vec2::new(c.x - d.x, c.y + d.y)];
            for i in 0..4 {
                line(k[i], k[(i + 1) % 4]);
            }
        }
        (_, [a]) => line(*a, h),
        (_, [a, b]) => {
            line(*a, *b);
            line(*b, h);
        }
        _ => {}
    }
}
