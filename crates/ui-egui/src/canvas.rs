//! On-canvas command input: while a feature dialog is open, its main value (extrude distance,
//! fillet radius…) gets a value box in the viewport next to the geometry, and an arrow
//! (extrude, fillet, shell, move) that can be dragged: values snap to round steps (hold Alt or
//! Ctrl for a free drag) and the dialog follows. The box takes the keyboard as soon as the
//! command has its geometry, so typing a number sets the value at once; Enter applies, Tab
//! moves on, Esc cancels.

use egui::{Color32, Pos2, Stroke, vec2};
use solvecraft_engine::Sel;
use solvecraft_engine::doc::expr::Kind as ValueKind;
use solvecraft_engine::geom::{Mesh, Vec3};

use crate::SolveApp;
use crate::dialogs::{Dialog, Kind};
use crate::theme::Tokens;
use crate::viewport::{Proj, projected_polygon};

/// Where the dialog's geometry is: a base point and, for values that can be dragged, the
/// direction a growing value points.
pub fn anchor(app: &SolveApp, d: &Dialog) -> Option<(Vec3, Option<Vec3>)> {
    let st = app.session.world_state();
    let first = d.inputs.first()?.items.first()?;
    let (at, normal) = match first {
        Sel::Profile { sketch, index } => {
            let ss = st.sketch(*sketch)?;
            let plane = profile_plane(app, *sketch)?;
            let p = ss.profiles.get(*index)?;
            let centre = if matches!(d.kind, Kind::Extrude { .. }) {
                let regions: Vec<_> = d
                    .inputs
                    .first()?
                    .items
                    .iter()
                    .filter_map(|item| {
                        if let Sel::Profile { sketch: id, index } = item { (*id == *sketch).then(|| ss.profiles.get(*index)).flatten() } else { None }
                    })
                    .map(|p| p.region.clone())
                    .collect();
                let regions = solvecraft_engine::sketch::merge_regions(&regions);
                let points: Vec<_> = regions.iter().flat_map(|r| r.outer.polyline(1e-2)).collect();
                points.iter().fold(solvecraft_engine::geom::Vec2::default(), |sum, p| sum + *p) * (1.0 / points.len().max(1) as f64)
            } else {
                p.region.interior_point()
            };
            (plane.to_world(centre), Some(plane.normal()))
        }
        Sel::Face { body, index, point } => (*point, crate::dialogs::planar_face(&app.session, body, *index).map(|(_, n)| n)),
        Sel::Edge { body, index, point } => (*point, st.body(body).and_then(|b| edge_bisector(&b.mesh(), *index, *point))),
        Sel::Vertex { point, .. } => (*point, None),
        Sel::SketchCurve { id } => {
            let ss = st.sketches.iter().rev().find(|ss| ss.sketch.curve_index(id).is_some())?;
            let pts = ss.sketch.polyline(ss.sketch.curve_index(id)?);
            (ss.plane.to_world(*pts.get(pts.len() / 2)?), None)
        }
        Sel::Body { name } => (st.body(name)?.mesh().bounds().center(), None),
        Sel::Plane { name } => {
            let pl = solvecraft_engine::geom::Plane::named(name).or_else(|| {
                solvecraft_engine::view::construction_planes(&app.session).into_iter().find(|(_, n, _)| n == name).map(|(_, _, pl)| pl)
            })?;
            (pl.origin, Some(pl.normal()))
        }
        _ => return None,
    };
    Some(match &d.kind {
        Kind::Extrude { start, .. } => {
            let n = normal?;
            let plane = solvecraft_engine::geom::Plane::from_normal(at, n)?;
            let from = reference_height(app, d, "From Object", &plane, at).unwrap_or(0.0);
            let offset = if start.trim().is_empty() { 0.0 } else { app.session.doc.eval(start, ValueKind::Length).ok()? };
            (at + n * (from + offset), Some(n))
        }
        Kind::Fillet { .. } | Kind::Section { .. } | Kind::Hole { .. } | Kind::OffsetPlane { .. } | Kind::OffsetFaces { .. } => (at, normal),
        // Shell thickness grows into the body.
        Kind::Shell { .. } => (at, normal.map(|n| -n)),
        Kind::Sheet(_) => (at, crate::dialogs_sheet::arrow(app, d).or(normal)),
        Kind::Plastic(_) => (at, normal),
        Kind::PatternRect { .. } => (at, d.inputs.get(1).and_then(|i| i.items.first()).and_then(|x| crate::dialogs::axis_of(app, x)).map(|a| a.1)),
        _ => (at, None),
    })
}

fn profile_plane(app: &SolveApp, sketch: u64) -> Option<solvecraft_engine::geom::Plane> {
    let component = app.session.doc.feature(sketch)?.component;
    let state = app.session.model.state();
    let plane = state.sketch(sketch)?.plane;
    let frame = crate::dialogs::feature_frame(&app.session, component);
    solvecraft_engine::geom::Plane::new(
        solvecraft_engine::doc::apply_point(&frame, plane.origin),
        solvecraft_engine::doc::apply_vector(&frame, plane.x),
        solvecraft_engine::doc::apply_vector(&frame, plane.y),
    )
}

fn reference_height(app: &SolveApp, d: &Dialog, label: &str, plane: &solvecraft_engine::geom::Plane, centre: Vec3) -> Option<f64> {
    let point = crate::dialogs::object_point(&d.inputs, label)?;
    let p = point.as_array()?;
    let p = Vec3::new(p.first()?.as_f64()?, p.get(1)?.as_f64()?, p.get(2)?.as_f64()?);
    Some(solvecraft_engine::doc::object_height(&app.session.world_state(), plane, centre, p))
}

/// The outward direction between the two faces at an edge (where a fillet's radius arrow
/// points).
fn edge_bisector(m: &Mesh, index: usize, p: Vec3) -> Option<Vec3> {
    let faces = m.edge_faces.get(index)?;
    let mut sum = Vec3::ZERO;
    for f in faces {
        let n = m
            .triangles
            .iter()
            .zip(&m.tri_face)
            .filter(|(_, tf)| *tf == f)
            .filter_map(|(t, _)| m.tri(t))
            .min_by(|a, b| ((a[0] + a[1] + a[2]) / 3.0).dist(p).total_cmp(&((b[0] + b[1] + b[2]) / 3.0).dist(p)))
            .and_then(|[a, b, c]| (b - a).cross(c - a).normalized())?;
        sum += n;
    }
    sum.normalized()
}

/// Draw the manipulator and the value box (called by the viewport after the model is drawn).
pub fn show(app: &mut SolveApp, ui: &mut egui::Ui, painter: &egui::Painter, proj: &Proj) {
    let Some(mut d) = app.dialog.take() else { return };
    if matches!(d.kind, Kind::Assembly(_)) {
        crate::dialogs_assembly::overlay(app, painter, proj, &mut d);
        app.dialog = Some(d);
        return;
    }
    if crate::dialogs_sheet::overlay(painter, proj.rect, &d) {
        app.dialog = Some(d);
        return;
    }
    if let Kind::Measure { result: Some(r), .. } = &d.kind {
        measure_line(painter, proj, r);
        app.dialog = Some(d);
        return;
    }
    // Move: the triad at the pivot; dragging it sets the distances and angles.
    if matches!(d.kind, Kind::Move { .. }) {
        if let Some(t) = crate::dialogs_move::triad(app, &d)
            && let Some(g) = crate::gizmo::show(ui, painter, proj, egui::Id::new("sc_move_triad"), &t, snap_step(app.cam.half_height()))
        {
            crate::dialogs_move::drag(app, &mut d, g);
        }
        app.dialog = Some(d);
        return;
    }
    if let Some((base, dir)) = anchor(app, &d)
        && let Some(bs) = proj.to_screen(base)
    {
        draw(app, ui, painter, proj, &mut d, base, bs, dir);
    }
    app.dialog = Some(d);
}

/// A round step for dragged values at this zoom (1, 2 or 5 times a power of ten).
pub fn snap_step(half_height: f64) -> f64 {
    let raw = (half_height / 40.0).max(1e-4);
    let p = 10f64.powf(raw.log10().floor());
    if raw / p < 2.0 {
        p
    } else if raw / p < 5.0 {
        2.0 * p
    } else {
        5.0 * p
    }
}

/// Parameter along the line `base + dir * t` closest to the view ray through `pos`.
fn along(proj: &Proj, pos: Pos2, base: Vec3, dir: Vec3) -> Option<f64> {
    let (o, r) = proj.ray(pos);
    let w0 = base - o;
    let (b, d, e) = (dir.dot(r), dir.dot(w0), r.dot(w0));
    let den = 1.0 - b * b;
    // Looking straight along the line: nothing to drag.
    if den.abs() < 1e-6 {
        return None;
    }
    Some((b * e - d) / den)
}

fn format_len(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{} mm", if s == "-0" { "0" } else { s })
}

/// The offset plane preview: a square of half side `h` around `c`, square to `n`.
fn offset_plane_quad(proj: &Proj, c: Vec3, n: Vec3, h: f64) -> Option<Vec<Pos2>> {
    let u = n.cross(if n.z.abs() < 0.9 { Vec3::Z } else { Vec3::X }).normalized()?;
    let v = n.cross(u);
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().map(|(a, b)| proj.to_screen(c + u * (a * h) + v * (b * h))).collect()
}

#[allow(clippy::too_many_arguments)]
fn draw(app: &SolveApp, ui: &mut egui::Ui, painter: &egui::Painter, proj: &Proj, d: &mut Dialog, base: Vec3, bs: Pos2, dir: Option<Vec3>) {
    let t = Tokens::get();
    let to_span = dir.and_then(|n| {
        let plane = solvecraft_engine::geom::Plane::from_normal(base, n)?;
        reference_height(app, d, "To Object", &plane, base)
    });
    let all_span = if matches!(d.kind, Kind::Extrude { all: true, .. }) {
        dir.and_then(|n| {
            let state = app.session.world_state();
            let plane = match d.inputs.first()?.items.first()? {
                Sel::Profile { sketch, .. } => profile_plane(app, *sketch)?,
                _ => solvecraft_engine::geom::Plane::from_normal(base, n)?,
            };
            let targets: Vec<String> = d
                .extra
                .get("targets")
                .and_then(serde_json::Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            solvecraft_engine::doc::through_all_distance(&state, &plane, &targets).ok()
        })
    } else {
        None
    };
    let sign = match (to_span, &d.kind) {
        (Some(span), _) => {
            if span < 0.0 {
                -1.0
            } else {
                1.0
            }
        }
        (_, Kind::Extrude { direction: 1, .. }) => -1.0,
        _ => 1.0,
    };
    let symmetric = to_span.is_none() && matches!(d.kind, Kind::Extrude { direction: 2, .. });
    // Extrudes and moves go either way; radii and thicknesses stay positive.
    let signed =
        matches!(d.kind, Kind::Extrude { .. } | Kind::Move { .. } | Kind::Section { .. } | Kind::OffsetPlane { .. } | Kind::OffsetFaces { .. });
    let focus = std::mem::take(&mut d.focus);
    let half_height = app.cam.half_height();
    let axis = revolve_axis(app, d);
    if matches!(d.kind, Kind::Revolve { .. }) {
        if let Some(axis) = axis {
            draw_revolve(app, ui, painter, proj, d, base, bs, axis, focus);
        }
        return;
    }
    // Two sides: the second distance gets its own arrow the other way.
    if let (Kind::Extrude { direction: crate::dialogs::TWO_SIDES, distance2, .. }, Some(n)) = (&mut d.kind, dir)
        && let Some(l2) = app.session.doc.eval(distance2, ValueKind::Length).ok().filter(|v| v.is_finite())
    {
        drag_arrow(ui, painter, proj, base, bs, -n, l2, true, egui::Id::new("sc_manipulator2"), half_height, distance2);
    }
    // An offset plane shows where it will be.
    if let (Kind::OffsetPlane { offset }, Some(n)) = (&d.kind, dir)
        && let Ok(off) = app.session.doc.eval(offset, ValueKind::Length)
        && let Some(q) = offset_plane_quad(proj, base + n * off, n, half_height * 0.25)
    {
        painter.extend(projected_polygon(&q, t.construction_plane, Stroke::new(1.2, t.origin_plane_edge)));
    }
    // Holes: the depth arrow points into the material (empty depth: through all); the
    // diameter is typed in the box.
    let mut dir = dir;
    // Extrude taper: a handle beside the arrow tip; dragging it sideways tilts the walls.
    let cam_right = app.cam.basis().0;
    if let (Kind::Extrude { direction: 0 | 1, distance, taper, taper_present, to_offset, .. }, Some(n)) = (&mut d.kind, dir) {
        let n = n * sign;
        let span = to_span.map(|span| span.abs() + app.session.doc.eval(to_offset, ValueKind::Length).unwrap_or(0.0)).or(all_span);
        if let Ok(l) = span.map(Ok).unwrap_or_else(|| app.session.doc.eval(distance, ValueKind::Length))
            && l.is_finite()
            && l.abs() > 1e-9
            && let Some(e) = (cam_right - n * cam_right.dot(n)).normalized()
        {
            let px = 2.0 * half_height / f64::from(proj.rect.height().max(1.0));
            let ang = app.session.doc.eval(taper, ValueKind::Angle).ok().filter(|v| v.is_finite()).unwrap_or(0.0);
            let before = taper.clone();
            taper_handle(ui, painter, proj, base - e * (40.0 * px), n * l, -e, ang, taper);
            if *taper != before {
                *taper_present = true;
            }
        }
    }
    if let (Kind::Hole { depth, opts, .. }, Some(n)) = (&mut d.kind, dir) {
        let l = if opts.all { 0.0 } else { app.session.doc.eval(depth, ValueKind::Length).ok().filter(|v| v.is_finite()).unwrap_or(0.0) };
        let before = depth.clone();
        drag_arrow(ui, painter, proj, base, bs, -n, l, false, egui::Id::new("sc_manipulator_depth"), half_height, depth);
        // Dragging the depth turns "through all" into a distance.
        if *depth != before {
            opts.all = false;
        }
        dir = None;
    }
    let Some((label, kind, value)) = d.primary() else { return };
    let len = all_span.or_else(|| {
        if to_span.is_some() && value.trim().is_empty() { Some(0.0) } else { app.session.doc.eval(value, kind).ok().filter(|v| v.is_finite()) }
    });
    let before = value.clone();
    let mut box_at = bs + vec2(16.0, -30.0);
    if let (Some(n), Some(l)) = (dir, len.filter(|_| kind == ValueKind::Length)) {
        let n = n * sign;
        let baseline = to_span.map(f64::abs).unwrap_or(0.0);
        let mut shown_value = format_len(l + baseline);
        let original_shown = shown_value.clone();
        let dragged = if to_span.is_some() { &mut shown_value } else { &mut *value };
        if let Some(ts) = drag_arrow(ui, painter, proj, base, bs, n, l + baseline, signed, egui::Id::new("sc_manipulator"), half_height, dragged) {
            if to_span.is_some()
                && shown_value != original_shown
                && let Ok(v) = app.session.doc.eval(&shown_value, ValueKind::Length)
            {
                *value = format_len(v - baseline);
            }
            if symmetric && let Some(bt) = proj.to_screen(base - n * l) {
                arrow(painter, bs, bt, t.manipulator);
            }
            painter.circle(bs, 3.5, t.manipulator, Stroke::new(1.0, Color32::WHITE));
            box_at = ts + vec2(14.0, -12.0);
        }
    }
    let id = egui::Id::new("sc_canvas_value");
    crate::frame::area(egui::Id::new("sc_canvas_area")).fixed_pos(box_at).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).color(t.text_dim).size(11.5));
                let r = ui.add(egui::TextEdit::singleline(value).id(id).desired_width(72.0));
                crate::params_dialog::complete(ui, &r, value);
                if focus {
                    r.request_focus();
                    let n = value.chars().count();
                    let mut st = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                    st.cursor.set_char_range(Some(egui::text_selection::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                    st.store(ui.ctx(), id);
                }
            });
        });
    });
    if *value != before
        && let Kind::Extrude { all, .. } = &mut d.kind
    {
        *all = false;
    }
}

/// A draggable arrow from `base` along `n` showing `l` (at least a short stub); dragging writes
/// the snapped length into `value`. Returns the tip's screen position.
#[allow(clippy::too_many_arguments)]
fn drag_arrow(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    proj: &Proj,
    base: Vec3,
    bs: Pos2,
    n: Vec3,
    l: f64,
    signed: bool,
    id: egui::Id,
    half_height: f64,
    value: &mut String,
) -> Option<Pos2> {
    let t = Tokens::get();
    let min = 40.0 * 2.0 * half_height / f64::from(proj.rect.height().max(1.0));
    let shown = if l.abs() < min { min * if l < 0.0 { -1.0 } else { 1.0 } } else { l };
    let ts = proj.to_screen(base + n * shown)?;
    crate::scenario::publish_handle(if id == egui::Id::new("sc_manipulator") { "arrow" } else { "arrow2" }, ts);
    let r = ui.interact(egui::Rect::from_center_size(ts, vec2(20.0, 20.0)), id, egui::Sense::drag());
    let hot = r.hovered() || r.dragged();
    let col = if hot { t.accent } else { t.manipulator };
    arrow(painter, bs, ts, col);
    if hot {
        painter.circle_stroke(ts, 8.0, Stroke::new(1.5, col));
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if r.dragged()
        && let Some(p) = r.interact_pointer_pos()
        && let Some(v) = along(proj, p, base, n)
    {
        let free = ui.input(|i| i.modifiers.alt || i.modifiers.ctrl || i.modifiers.command);
        let step = snap_step(half_height);
        let mut v = if free { v } else { (v / step).round() * step };
        if !signed {
            v = v.max(if free { 1e-3 } else { step });
        }
        *value = format_len(v);
    }
    Some(ts)
}

/// The measured distance: a line between the closest points with its length.
fn measure_line(painter: &egui::Painter, proj: &Proj, r: &serde_json::Value) {
    let t = Tokens::get();
    let p = |k: &str| -> Option<Vec3> {
        let a = r.get(k)?.as_array()?;
        Some(Vec3::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?))
    };
    let (Some(a), Some(b), Some(d)) = (p("from"), p("to"), r.get("distance_mm").and_then(|v| v.as_f64())) else { return };
    let (Some(sa), Some(sb)) = (proj.to_screen(a), proj.to_screen(b)) else { return };
    painter.line_segment([sa, sb], Stroke::new(2.0, t.manipulator));
    for s in [sa, sb] {
        painter.circle(s, 3.5, t.manipulator, Stroke::new(1.0, Color32::WHITE));
    }
    let mid = sa + (sb - sa) * 0.5 + vec2(10.0, -14.0);
    let galley = painter.layout_no_wrap(format!("{d:.3} mm"), egui::FontId::proportional(12.5), t.text);
    let rect = egui::Rect::from_min_size(mid, galley.size() + vec2(10.0, 6.0));
    painter.rect(rect, 3.0, t.overlay, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    painter.galley(rect.min + vec2(5.0, 3.0), galley, t.text);
}

/// The taper handle: a line from `foot` along the extrude (`span`) leaning by `angle` towards
/// `e`, with a ring at its end; dragging the ring sideways sets the angle (1° steps, Alt/Ctrl:
/// free).
#[allow(clippy::too_many_arguments)]
fn taper_handle(ui: &mut egui::Ui, painter: &egui::Painter, proj: &Proj, foot: Vec3, span: Vec3, e: Vec3, angle: f64, value: &mut String) {
    let t = Tokens::get();
    let l = span.len();
    let end = foot + span + e * (l * angle.tan().clamp(-10.0, 10.0));
    let (Some(fs), Some(es)) = (proj.to_screen(foot), proj.to_screen(end)) else { return };
    let r = ui.interact(egui::Rect::from_center_size(es, vec2(16.0, 16.0)), egui::Id::new("sc_taper"), egui::Sense::drag());
    let hot = r.hovered() || r.dragged();
    let col = if hot { t.accent } else { t.manipulator };
    painter.add(egui::Shape::dashed_line(&[fs, es], Stroke::new(1.2, col), 5.0, 3.0));
    painter.circle(es, if hot { 6.5 } else { 5.0 }, t.overlay, Stroke::new(2.0, col));
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if r.dragged()
        && let Some(p) = r.interact_pointer_pos()
        && let Some(s) = along(proj, p, foot + span, e)
    {
        let mut deg = s.atan2(l).to_degrees();
        if !ui.input(|i| i.modifiers.alt || i.modifiers.ctrl || i.modifiers.command) {
            deg = deg.round();
        }
        let txt = format!("{deg:.1}");
        *value = format!("{} deg", txt.trim_end_matches('0').trim_end_matches('.'));
    }
}

/// The revolve axis (a point on it and its unit direction).
pub(crate) fn revolve_axis(app: &SolveApp, d: &Dialog) -> Option<(Vec3, Vec3)> {
    if !matches!(d.kind, Kind::Revolve { .. }) {
        return None;
    }
    if let Some(selected) = d.inputs.get(1)?.items.first() {
        if let Sel::Axis { name } = selected
            && matches!(name.as_str(), "X" | "Y" | "Z")
            && let Sel::Profile { sketch, .. } = d.inputs.first()?.items.first()?
        {
            let component = app.session.doc.feature(*sketch)?.component;
            let frame = crate::dialogs::feature_frame(&app.session, component);
            let dir = match name.as_str() {
                "X" => Vec3::X,
                "Y" => Vec3::Y,
                _ => Vec3::Z,
            };
            return Some((solvecraft_engine::doc::apply_point(&frame, Vec3::ZERO), solvecraft_engine::doc::apply_vector(&frame, dir)));
        }
        if let Sel::Axis { name } = selected
            && matches!(name.as_str(), "x" | "y")
            && let Sel::Profile { sketch, .. } = d.inputs.first()?.items.first()?
        {
            let plane = profile_plane(app, *sketch)?;
            return Some((plane.origin, if name == "x" { plane.x } else { plane.y }));
        }
        return crate::dialogs::axis_of(app, selected);
    }
    let Kind::Revolve { axis: Some(axis), .. } = &d.kind else { return None };
    let vector = |key: &str| -> Option<Vec3> {
        let p = axis.get(key)?.as_array()?;
        Some(Vec3::new(p.first()?.as_f64()?, p.get(1)?.as_f64()?, p.get(2)?.as_f64()?))
    };
    Some((vector("origin")?, vector("dir")?.normalized()?))
}

/// Draw angular extents in the same clockwise sense as the modeled solid.
#[allow(clippy::too_many_arguments)]
fn draw_revolve(
    app: &SolveApp,
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    proj: &Proj,
    d: &mut Dialog,
    base: Vec3,
    bs: Pos2,
    axis: (Vec3, Vec3),
    focus: bool,
) {
    let to_mode = d.inputs.iter().any(|input| input.label == "To Object");
    let extent = revolve_display_extent(app, d, axis).unwrap_or(solvecraft_engine::doc::RevolveExtent {
        start: 0.0,
        sweep: f64::NAN,
        rotation_axis: axis.1,
        display_start: 0.0,
        curved_to: false,
    });
    let mut box_at = bs + vec2(16.0, -30.0);
    if !extent.curved_to && extent.sweep.is_finite() {
        let (origin, direction) = axis;
        let centre = origin + direction * (base - origin).dot(direction);
        let radius = (base - centre).len();
        if radius > 1e-9 {
            let e1 = (base - centre) / radius;
            let e2 = extent.rotation_axis.cross(e1);
            let handed = match revolve_convention(d) {
                solvecraft_engine::doc::RevolveConvention::LegacyAxisStart => -direction.dot(extent.rotation_axis),
                solvecraft_engine::doc::RevolveConvention::SketchNormal => 1.0,
            };
            let at = |angle: f64| centre + (e1 * angle.cos() + e2 * angle.sin()) * radius;
            let points: Vec<_> = (0..=48).filter_map(|i| proj.to_screen(at(extent.display_start + extent.sweep * i as f64 / 48.0))).collect();
            painter.add(egui::Shape::line(points, Stroke::new(2.0, Tokens::get().manipulator)));
            if let Some(end) = proj.to_screen(at(extent.display_start + extent.sweep)) {
                box_at = end + vec2(14.0, -12.0);
            }
            if !to_mode {
                let symmetric = !d.extra.contains_key("angle2") && d.extra.get("direction").and_then(serde_json::Value::as_str) == Some("symmetric");
                let other_angle = d
                    .extra
                    .get("angle2")
                    .and_then(|value| value.as_str())
                    .and_then(|value| app.session.doc.eval(value, ValueKind::Angle).ok())
                    .unwrap_or(0.0);
                let range = if extent.sweep < 0.0 { (-std::f64::consts::TAU, 0.0) } else { (0.0, std::f64::consts::TAU) };
                if let Kind::Revolve { angle, .. } = &mut d.kind {
                    let boundary = extent.display_start + extent.sweep;
                    if let Some(end) = rotation_handle(
                        ui,
                        painter,
                        proj,
                        centre,
                        e1,
                        e2,
                        direction,
                        radius,
                        boundary,
                        if symmetric { 1.0 / (1.0 - handed * 0.5) } else { 1.0 },
                        if symmetric { 0.0 } else { -other_angle * (1.0 - handed) },
                        (range.0 - other_angle, range.1 - other_angle),
                        "revolve",
                        angle,
                    ) {
                        box_at = end + vec2(14.0, -12.0);
                    }
                    if symmetric {
                        rotation_handle(
                            ui,
                            painter,
                            proj,
                            centre,
                            e1,
                            e2,
                            direction,
                            radius,
                            extent.display_start,
                            -2.0 * handed,
                            0.0,
                            range,
                            "revolve2",
                            angle,
                        );
                    }
                }
                if let Some(other) = d.extra.get_mut("angle2") {
                    let mut value = other.as_str().map(str::to_owned).unwrap_or_else(|| other.to_string());
                    let before = value.clone();
                    let first_angle = extent.sweep - other_angle;
                    rotation_handle(
                        ui,
                        painter,
                        proj,
                        centre,
                        e1,
                        e2,
                        direction,
                        radius,
                        extent.display_start,
                        -handed,
                        0.0,
                        (range.0 - first_angle, range.1 - first_angle),
                        "revolve2",
                        &mut value,
                    );
                    if value != before {
                        *other = serde_json::Value::String(value);
                    }
                }
            }
        }
    }
    crate::frame::area(egui::Id::new("sc_canvas_area")).fixed_pos(box_at).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
            if to_mode {
                let text = if extent.curved_to {
                    "To surface".into()
                } else if extent.sweep.is_finite() {
                    format!("Sweep {:.1}°", extent.sweep.to_degrees())
                } else {
                    "Choose target".into()
                };
                ui.label(text).on_hover_text("Determined by To Object. Choose Angle to edit the angular extent.");
            } else if let Kind::Revolve { angle, .. } = &mut d.kind {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Angle").color(Tokens::get().text_dim).size(11.5));
                    let id = egui::Id::new("sc_canvas_value");
                    let response = ui.add(egui::TextEdit::singleline(angle).id(id).desired_width(72.0));
                    crate::params_dialog::complete(ui, &response, angle);
                    if focus {
                        response.request_focus();
                        let n = angle.chars().count();
                        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                        state
                            .cursor
                            .set_char_range(Some(egui::text_selection::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                        state.store(ui.ctx(), id);
                    }
                });
            }
        });
    });
}

fn revolve_convention(d: &Dialog) -> solvecraft_engine::doc::RevolveConvention {
    d.extra
        .get("convention")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or(solvecraft_engine::doc::RevolveConvention::SketchNormal)
}

fn revolve_display_extent(app: &SolveApp, d: &Dialog, axis: (Vec3, Vec3)) -> Option<solvecraft_engine::doc::RevolveExtent> {
    let Sel::Profile { sketch, .. } = d.inputs.first()?.items.first()? else { return None };
    let state = app.session.world_state();
    let ss = state.sketch(*sketch)?;
    let regions: Vec<_> = d
        .inputs
        .first()?
        .items
        .iter()
        .filter_map(|item| match item {
            Sel::Profile { sketch: id, index } if id == sketch => ss.profiles.get(*index).map(|profile| profile.region.clone()),
            _ => None,
        })
        .collect();
    let regions = solvecraft_engine::sketch::merge_regions(&regions);
    let plane = profile_plane(app, *sketch)?;
    let to_mode = d.inputs.iter().any(|input| input.label == "To Object");
    let to = if to_mode {
        let point = crate::dialogs::object_point(&d.inputs, "To Object")?;
        Some(Vec3::new(point.get(0)?.as_f64()?, point.get(1)?.as_f64()?, point.get(2)?.as_f64()?))
    } else {
        None
    };
    let Kind::Revolve { angle, .. } = &d.kind else { return None };
    let angles = if to_mode {
        (0.0, None)
    } else {
        let angle = app.session.doc.eval(angle, ValueKind::Angle).ok()?;
        let other = d.extra.get("angle2").map(|value| app.session.doc.eval(value.as_str().unwrap_or_default(), ValueKind::Angle)).transpose().ok()?;
        (angle, other)
    };
    solvecraft_engine::doc::revolve_extent_with_convention(
        &state,
        &plane,
        &regions,
        axis,
        angles,
        d.extra.get("direction").and_then(serde_json::Value::as_str) == Some("symmetric"),
        to,
        revolve_convention(d),
    )
    .ok()
}

#[allow(clippy::too_many_arguments)]
fn rotation_handle(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    proj: &Proj,
    centre: Vec3,
    e1: Vec3,
    e2: Vec3,
    axis: Vec3,
    radius: f64,
    boundary: f64,
    scale: f64,
    offset: f64,
    range: (f64, f64),
    name: &str,
    value: &mut String,
) -> Option<Pos2> {
    let end = proj.to_screen(centre + (e1 * boundary.cos() + e2 * boundary.sin()) * radius)?;
    crate::scenario::publish_handle(name, end);
    let response = ui.interact(egui::Rect::from_center_size(end, vec2(20.0, 20.0)), egui::Id::new(("sc_rotator", name)), egui::Sense::drag());
    let hot = response.hovered() || response.dragged();
    let color = if hot { Tokens::get().accent } else { Tokens::get().manipulator };
    painter.circle(end, if hot { 7.0 } else { 5.5 }, color, Stroke::new(1.5, Color32::WHITE));
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if response.dragged()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let (origin, direction) = proj.ray(pointer);
        let den = direction.dot(axis);
        let (a, b) = ((range.0 - offset) / scale, (range.1 - offset) / scale);
        let bounds = (a.min(b), a.max(b));
        // Near edge-on, a plane intersection magnifies subpixel pointer error.
        let raw = if den.abs() > 0.05 {
            let point = origin + direction * ((centre - origin).dot(axis) / den) - centre;
            Some(point.dot(e2).atan2(point.dot(e1)))
        } else {
            projected_rotation_angle(proj, pointer, centre, e1, e2, radius, boundary, bounds)
        };
        if let Some(raw) = raw {
            let boundary_angle = (-4..=4)
                .map(|k| raw + k as f64 * std::f64::consts::TAU)
                .filter(|angle| *angle >= bounds.0 - 1e-4 && *angle <= bounds.1 + 1e-4)
                .min_by(|a, b| (a - boundary).abs().total_cmp(&(b - boundary).abs()))
                .unwrap_or_else(|| raw.clamp(bounds.0, bounds.1));
            let mut angle = (boundary_angle.clamp(bounds.0, bounds.1) * scale + offset).to_degrees();
            if !ui.input(|input| input.modifiers.alt || input.modifiers.ctrl || input.modifiers.command) {
                angle = (angle / 5.0).round() * 5.0;
            }
            if angle.abs() < 0.05 {
                angle = 0.0;
            }
            let text = format!("{angle:.1}");
            *value = format!("{} deg", text.trim_end_matches('0').trim_end_matches('.'));
        }
    }
    Some(end)
}

/// An edge-on ring has two equally valid depths; retain the branch nearest the current handle.
#[allow(clippy::too_many_arguments)]
fn projected_rotation_angle(
    proj: &Proj,
    pointer: Pos2,
    centre: Vec3,
    e1: Vec3,
    e2: Vec3,
    radius: f64,
    current: f64,
    (lo, hi): (f64, f64),
) -> Option<f64> {
    let distance = |angle: f64| {
        proj.to_screen(centre + (e1 * angle.cos() + e2 * angle.sin()) * radius)
            .map(|point| f64::from(point.distance_sq(pointer)))
            .unwrap_or(f64::INFINITY)
    };
    let step = (hi - lo) / 720.0;
    let mut best = None;
    for i in 0..=720 {
        let angle = lo + step * i as f64;
        let score = distance(angle);
        if !score.is_finite() {
            continue;
        }
        if best.is_none_or(|(old_angle, old_score): (f64, f64)| {
            score < old_score - 1e-4 || (score - old_score).abs() <= 1e-4 && (angle - current).abs() < (old_angle - current).abs()
        }) {
            best = Some((angle, score));
        }
    }
    let (angle, _) = best?;
    let (mut a, mut b) = ((angle - step).max(lo), (angle + step).min(hi));
    for _ in 0..24 {
        let (left, right) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
        if distance(left) <= distance(right) {
            b = right;
        } else {
            a = left;
        }
    }
    Some((a + b) / 2.0)
}
fn arrow(painter: &egui::Painter, a: Pos2, b: Pos2, col: Color32) {
    let v = b - a;
    let l = v.length();
    if l < 1.0 {
        return;
    }
    let u = v / l;
    let n = vec2(-u.y, u.x);
    painter.line_segment([a, b], Stroke::new(2.0, col));
    let head = 11.0f32.min(l * 0.6);
    painter.add(egui::Shape::convex_polygon(vec![b, b - u * head + n * head * 0.45, b - u * head - n * head * 0.45], col, Stroke::NONE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_plane_preview_stays_within_its_square() {
        let rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let mut worst = f32::NEG_INFINITY;
        for n in [Vec3::X, Vec3::Y, Vec3::Z] {
            for dir in crate::viewport::orbit_directions() {
                let cam = solvecraft_engine::render::Camera::default().looking_from(dir);
                let proj = Proj { cam, vp: cam.view_proj(4.0 / 3.0, 100.0), rect };
                let h = cam.half_height() * 0.25;
                let (Some(q), Some(c), Some(side)) =
                    (offset_plane_quad(&proj, Vec3::ZERO, n, h), proj.to_screen(Vec3::ZERO), proj.to_screen(cam.basis().0 * h))
                else {
                    continue;
                };
                // The square's half diagonal on screen (an orthographic view), plus stroke and feathering.
                let limit = c.distance(side) * 2f32.sqrt() + 3.0;
                for p in crate::viewport::drawn_points(projected_polygon(&q, Color32::GRAY, Stroke::new(1.2, Color32::BLACK))) {
                    worst = worst.max(p.distance(c) - limit);
                }
            }
        }
        assert!(worst <= 0.0, "a vertex {worst} px past the square");
    }

    #[test]
    fn snapping_steps_and_formatting() {
        assert_eq!(snap_step(40.0), 1.0);
        assert_eq!(snap_step(100.0), 2.0);
        assert_eq!(snap_step(4.0), 0.1);
        assert_eq!(format_len(12.0), "12 mm");
        assert_eq!(format_len(-2.5), "-2.5 mm");
        assert_eq!(format_len(-0.0001), "0 mm");
    }
}
