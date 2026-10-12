//! Sketch dimensions drawn as dimensions: extension lines from the measured geometry, a
//! dimension line (or arc) with arrowheads, and the value centred on it; radius and diameter
//! leaders with R / Ø, angles as an arc between the two lines, arc lengths with the arc mark,
//! driven (reference) dimensions in parentheses in a muted colour. The geometry comes from
//! `solvecraft_sketch::dim_layout`.
//!
//! A click selects a dimension (Delete removes it), a drag moves its text (and so the offset of
//! its dimension line), a double-click edits its value in place with parameter completion.
//! Everything acts through commands: `sketch.dimension_text`, `sketch.delete`,
//! `parameters.change`.

use std::cell::RefCell;
use std::sync::Arc;

use egui::{FontId, Pos2, Rect, Shape, Stroke, vec2};
use serde_json::json;
use solvecraft_engine::Sel;
use solvecraft_engine::geom::Vec2;
use solvecraft_engine::sketch::{ConstraintKind, DimFrame, chain_centres, default_text, dim_frame, dim_layout};

use crate::SolveApp;
use crate::theme::Tokens;
use crate::viewport::Proj;

/// A dimension as drawn this frame (screen space), for hit testing.
#[derive(Clone, Debug)]
struct Drawn {
    id: String,
    param: Option<String>,
    text: Rect,
    segs: Vec<[Pos2; 2]>,
    /// Text centre in sketch coordinates.
    at: Vec2,
}

/// The inline value editor: parameter, expression, where it sits, first frame.
#[derive(Clone, Debug)]
struct Edit {
    param: String,
    expr: String,
    /// Where it sits (None until the dimension has been drawn).
    at: Option<Pos2>,
    /// Frames left in which the box (re)takes the keyboard: the rest of the double-click
    /// that opened it must not close it.
    fresh: u8,
    /// Frames left to wait for the dimension to be drawn.
    wait: u8,
    draft: Option<DimensionDraft>,
}

/// Creation stays in a scratch session until the user accepts the value. Retaining the
/// document also makes the editor's identity check safe across document replacements.
#[derive(Clone, Debug)]
struct DimensionDraft {
    baseline: Arc<solvecraft_engine::doc::Document>,
    sketch: u64,
    component: u64,
    path: Option<Vec<u64>>,
    pending: std::collections::BTreeMap<u64, solvecraft_engine::doc::Mat>,
    entities: Vec<String>,
    at: Vec2,
    checked_expr: String,
    preview_doc: Arc<solvecraft_engine::doc::Document>,
    preview: Arc<solvecraft_engine::doc::ModelState>,
    error: Option<String>,
}

impl DimensionDraft {
    fn valid(&self, app: &SolveApp) -> bool {
        Arc::ptr_eq(&self.baseline, &app.session.doc)
            && app.session.active_sketch == Some(self.sketch)
            && app.session.active_component == self.component
            && app.session.active_occurrence_path == self.path
            && app.session.pending_moves == self.pending
    }
    fn params(&self, expr: &str) -> serde_json::Value {
        json!({"entities": self.entities, "text_at": [self.at.x, self.at.y], "value": expr})
    }
    fn evaluate(&self, app: &SolveApp, expr: &str) -> Result<solvecraft_engine::Session, String> {
        let mut scratch = app.session.scratch();
        let result = scratch.execute("sketch.dimension", &self.params(expr)).map_err(|e| e.to_string())?;
        let param = result.get("param").and_then(serde_json::Value::as_str).ok_or("The dimension value is unavailable")?;
        let state = scratch.world_state();
        let solved = state.sketch(self.sketch).ok_or("The dimension sketch is unavailable")?;
        if !solved.report.ok() {
            return Err("That value conflicts with the sketch constraints".into());
        }
        if let Some(kind) = solved.sketch.constraints.iter().find(|c| c.param.as_deref() == Some(param)).map(|c| &c.kind)
            && !kind.is_angle()
            && kind.value().is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err("Enter a dimension greater than zero".into());
        }
        Ok(scratch)
    }
}

pub fn cancel() {
    EDIT.with(|edit| *edit.borrow_mut() = None);
    PLACEMENT.with(|preview| *preview.borrow_mut() = None);
}

/// Infer the measured value through the same command as scripts, without creating a real
/// parameter or an undo entry. Only accepting the editor executes the command on the design.
pub fn begin_dimension(app: &SolveApp, entities: Vec<String>, at: Vec2) -> Result<(), String> {
    let (param, expr, draft) = dimension_draft(app, entities, at)?;
    EDIT.with(|edit| *edit.borrow_mut() = Some(Edit { param, expr, at: None, fresh: FRESH, wait: 10, draft: Some(draft) }));
    PLACEMENT.with(|preview| *preview.borrow_mut() = None);
    Ok(())
}

fn dimension_draft(app: &SolveApp, entities: Vec<String>, at: Vec2) -> Result<(String, String, DimensionDraft), String> {
    let sketch = app.session.active_sketch.ok_or("Edit a sketch to add a dimension")?;
    if entities.is_empty() || entities.len() > 2 || !at.x.is_finite() || !at.y.is_finite() {
        return Err("Pick one edge or two endpoints, then place the dimension".into());
    }
    let mut scratch = app.session.scratch();
    let result = scratch.execute("sketch.dimension", &json!({"entities": entities, "text_at": [at.x, at.y]})).map_err(|e| e.to_string())?;
    let param = result.get("param").and_then(serde_json::Value::as_str).ok_or("This dimension has no editable value")?.to_string();
    let expr = scratch.doc.param(&param).ok_or("The dimension value is unavailable")?.expr.clone();
    let draft = DimensionDraft {
        baseline: app.session.doc.clone(),
        sketch,
        component: app.session.active_component,
        path: app.session.active_occurrence_path.clone(),
        pending: app.session.pending_moves.clone(),
        entities,
        at,
        checked_expr: expr.clone(),
        preview: scratch.world_state(),
        preview_doc: scratch.doc.clone(),
        error: None,
    };
    if !draft.preview.sketch(sketch).is_some_and(|solved| solved.report.ok()) {
        return Err("That dimension conflicts with the sketch constraints".into());
    }
    Ok((param, expr, draft))
}

/// Cache inference at a stationary pointer, including incomplete/unsupported picks.
#[derive(Clone)]
struct Placement {
    baseline: Arc<solvecraft_engine::doc::Document>,
    sketch: Option<u64>,
    component: u64,
    path: Option<Vec<u64>>,
    pending: std::collections::BTreeMap<u64, solvecraft_engine::doc::Mat>,
    entities: Vec<String>,
    at: Vec2,
    draft: Option<(String, DimensionDraft)>,
}

fn placement_preview(app: &SolveApp) -> Option<(String, DimensionDraft)> {
    let tool = app.tool.as_ref().filter(|t| t.kind == crate::tools::Kind::Dimension && !t.picks.is_empty())?;
    let at = tool.hover.as_ref()?.0;
    PLACEMENT.with(|preview| {
        let mut preview = preview.borrow_mut();
        let current = preview.as_ref().is_some_and(|p| {
            Arc::ptr_eq(&p.baseline, &app.session.doc)
                && p.sketch == app.session.active_sketch
                && p.component == app.session.active_component
                && p.path == app.session.active_occurrence_path
                && p.pending == app.session.pending_moves
                && p.entities == tool.picks
                && p.at == at
        });
        if !current {
            let draft = dimension_draft(app, tool.picks.clone(), at).ok().map(|(param, _, draft)| (param, draft));
            *preview = Some(Placement {
                baseline: app.session.doc.clone(),
                sketch: app.session.active_sketch,
                component: app.session.active_component,
                path: app.session.active_occurrence_path.clone(),
                pending: app.session.pending_moves.clone(),
                entities: tool.picks.clone(),
                at,
                draft,
            });
        }
        preview.as_ref().and_then(|p| p.draft.clone())
    })
}

thread_local! {
    static DRAWN: RefCell<Vec<Drawn>> = const { RefCell::new(Vec::new()) };
    /// A text being dragged: dimension id, undo depth at the start, grab offset (sketch).
    static DRAG: RefCell<Option<(String, usize, Vec2)>> = const { RefCell::new(None) };
    static EDIT: RefCell<Option<Edit>> = const { RefCell::new(None) };
    static PLACEMENT: RefCell<Option<Placement>> = const { RefCell::new(None) };
}

/// Frames the inline editor holds on to the keyboard after opening.
const FRESH: u8 = 3;

/// Pixels from a dimension line or the text box that still count as on it.
const HIT_PX: f32 = 4.0;

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// The dimension whose value text is under a screen position (its id).
pub fn hit_text(pos: Pos2) -> Option<String> {
    DRAWN.with(|d| d.borrow().iter().find(|x| x.text.expand(2.0).contains(pos)).map(|x| x.id.clone()))
}

/// The dimension under a screen position (its id).
pub fn hit(pos: Pos2) -> Option<String> {
    DRAWN.with(|d| {
        let d = d.borrow();
        // Text boxes first (they sit on top of the lines), then the lines.
        d.iter()
            .find(|x| x.text.expand(2.0).contains(pos))
            .or_else(|| d.iter().find(|x| x.segs.iter().any(|s| seg_dist(pos, s[0], s[1]) <= HIT_PX)))
            .map(|x| x.id.clone())
    })
}

/// The selected dimensions (constraint ids; they are in the session's selection).
pub fn selected(app: &SolveApp) -> Vec<String> {
    let dims: Vec<String> = DRAWN.with(|d| d.borrow().iter().map(|x| x.id.clone()).collect());
    app.session
        .selection
        .iter()
        .filter_map(|s| if let Sel::SketchConstraint { id } = s { Some(id.clone()) } else { None })
        .filter(|id| dims.contains(id))
        .collect()
}

/// Where the dimension driven by parameter `param` (or with constraint id `param`) shows its
/// value (screen), as drawn last frame.
pub fn text_at(param: &str) -> Option<Pos2> {
    DRAWN.with(|d| d.borrow().iter().find(|x| x.param.as_deref() == Some(param) || x.id == param).map(|x| x.text.center()))
}

/// Dimension text boxes drawn last frame (for box selection).
pub fn text_rects() -> Vec<(String, Rect)> {
    DRAWN.with(|d| d.borrow().iter().map(|x| (x.id.clone(), x.text)).collect())
}

/// A double-click on a dimension opens its value for editing (true: it was on one).
pub fn double_click(app: &mut SolveApp, pos: Pos2) -> bool {
    let Some(d) = DRAWN.with(|d| {
        let id = hit(pos)?;
        d.borrow().iter().find(|x| x.id == id).cloned()
    }) else {
        return false;
    };
    let Some(param) = d.param else {
        app.set_status("A driven dimension measures the sketch; it has no value to edit", false);
        return true;
    };
    let expr = app.session.doc.param(&param).map(|p| p.expr.clone()).unwrap_or_default();
    EDIT.with(|e| *e.borrow_mut() = Some(Edit { param, expr, at: Some(d.text.center()), fresh: FRESH, wait: 0, draft: None }));
    true
}

/// Start dragging the text of the dimension under `press` (true: a dimension took the drag).
pub fn drag_start(app: &SolveApp, press: Pos2, press_sketch: Vec2) -> bool {
    let Some(d) = DRAWN.with(|d| {
        let id = hit(press)?;
        d.borrow().iter().find(|x| x.id == id).cloned()
    }) else {
        return false;
    };
    DRAG.with(|g| *g.borrow_mut() = Some((d.id, app.session.undo.len(), d.at - press_sketch)));
    true
}

/// Is a dimension text being dragged?
pub fn dragging() -> bool {
    DRAG.with(|g| g.borrow().is_some())
}

/// Move the dragged text so it follows the pointer (at `to`, sketch coordinates); one undo
/// step for the whole drag.
pub fn drag_to(app: &mut SolveApp, to: Vec2) {
    let Some((id, depth, off)) = DRAG.with(|g| g.borrow().clone()) else { return };
    let at = to + off;
    if app.session.execute("sketch.dimension_text", &json!({"dimension": id, "at": [at.x, at.y]})).is_ok() {
        app.session.undo.truncate(depth + 1);
    }
}

pub fn drag_end() {
    DRAG.with(|g| *g.borrow_mut() = None);
}

/// The text shown for a dimension: the value (R / Ø for radius and diameter, degrees for
/// angles), "fx: value" when an expression drives it, in parentheses when driven.
pub fn label(k: &ConstraintKind, expr: Option<&str>, driven: bool) -> String {
    let v = k.value().unwrap_or(0.0);
    let num = if k.is_angle() { format!("{:.1}°", v.to_degrees()) } else { format!("{v:.2}") };
    let num = match k {
        ConstraintKind::Radius { .. } => format!("R{num}"),
        ConstraintKind::Diameter { .. } | ConstraintKind::LinearDiameter { .. } => format!("Ø{num}"),
        _ => num,
    };
    let plain = expr.is_none_or(|e| {
        let e = e.trim();
        let e = e.strip_suffix("mm").or_else(|| e.strip_suffix("deg")).or_else(|| e.strip_suffix('°')).unwrap_or(e);
        e.trim().parse::<f64>().is_ok()
    });
    let text = if plain { num } else { format!("fx: {num}") };
    if driven { format!("({text})") } else { text }
}

/// Draw the active sketch's dimensions and remember where they are for picking.
pub fn show(app: &mut SolveApp, ui: &egui::Ui, painter: &egui::Painter, proj: &Proj) {
    let mut drawn = Vec::new();
    let editor_draft = EDIT.with(|edit| {
        let mut edit = edit.borrow_mut();
        if edit.as_ref().and_then(|e| e.draft.as_ref()).is_some_and(|d| !d.valid(app)) {
            *edit = None;
        }
        if let Some(e) = edit.as_mut()
            && let Some(draft) = e.draft.as_mut()
        {
            if e.expr != draft.checked_expr {
                draft.checked_expr = e.expr.clone();
                match draft.evaluate(app, &e.expr) {
                    Ok(scratch) => {
                        draft.preview = scratch.world_state();
                        draft.preview_doc = scratch.doc.clone();
                        draft.error = None;
                    }
                    Err(error) => draft.error = Some(error),
                }
            }
            Some((e.param.clone(), draft.clone()))
        } else {
            None
        }
    });
    let placing = !editing();
    let draft = editor_draft.or_else(|| placing.then(|| placement_preview(app)).flatten());
    let st = app.session.world_state();
    let Some(ss) = app.session.active_sketch.and_then(|s| st.sketch(s)) else {
        DRAWN.with(|d| d.borrow_mut().clear());
        EDIT.with(|e| *e.borrow_mut() = None);
        return;
    };
    let sk = &ss.sketch;
    if sk.view.hide_dimensions {
        DRAWN.with(|d| d.borrow_mut().clear());
        if draft.is_none() {
            return;
        }
    }
    let tk = Tokens::get();
    let is_sel = |id: &str| app.session.selection.iter().any(|s| matches!(s, Sel::SketchConstraint { id: x } if x == id));
    let hover = ui.input(|i| i.pointer.hover_pos()).filter(|_| ui.rect_contains_pointer(proj.rect)).and_then(hit);
    let font = FontId::proportional(12.0);
    let centres = chain_centres(sk);
    let draft_sketch = draft.as_ref().and_then(|(_, d)| d.preview.sketch(d.sketch));
    let draft_centres = draft_sketch.map(|solved| chain_centres(&solved.sketch));
    let mut constraints = sk.constraints.iter().map(|c| (ss, c, false)).collect::<Vec<_>>();
    if let (Some(source), Some((param, _))) = (draft_sketch, draft.as_ref()) {
        constraints.extend(source.sketch.constraints.iter().filter(|c| c.param.as_deref() == Some(param.as_str())).map(|c| (source, c, true)));
    }
    for (source, c, is_draft) in constraints {
        if ss.sketch.view.hide_dimensions && !is_draft {
            continue;
        }
        let sk = &source.sketch;
        let centres = if is_draft { draft_centres.as_ref().unwrap_or(&centres) } else { &centres };
        let to = |p: Vec2| proj.to_screen(source.plane.to_world(p));
        let Some(frame) = dim_frame(sk, &c.kind) else { continue };
        let anchor = match frame {
            DimFrame::Linear { p0, p1, .. } => (p0 + p1) * 0.5,
            DimFrame::Radial { center, .. } | DimFrame::ArcLength { center, .. } => center,
            DimFrame::Angular { vertex, .. } => vertex,
        };
        // Millimetres per pixel at the dimension.
        let (Some(a), Some(b)) = (to(anchor), to(anchor + Vec2::X)) else { continue };
        let px = 1.0 / f64::from(a.distance(b)).max(1e-9);
        if !px.is_finite() {
            continue;
        }
        let display_doc =
            if is_draft { draft.as_ref().map(|(_, d)| d.preview_doc.as_ref()).unwrap_or(app.session.doc.as_ref()) } else { app.session.doc.as_ref() };
        let expr = c.param.as_ref().and_then(|p| display_doc.param(p)).map(|p| p.expr.as_str());
        let failing = source.report.failing.contains(&c.id);
        let col = if failing {
            tk.error
        } else if is_sel(&c.id) || hover.as_deref() == Some(c.id.as_str()) {
            tk.accent
        } else if c.driven {
            tk.dimension_driven
        } else {
            tk.dimension
        };
        let galley = painter.layout_no_wrap(label(&c.kind, expr, c.driven), font.clone(), col);
        let size = galley.size() + vec2(6.0, 2.0);
        let half = Vec2::new(f64::from(size.x) * 0.5 * px, f64::from(size.y) * 0.5 * px);
        // Unplaced linear dimensions sit outside the shape they measure.
        let away = first_point(sk, &c.kind).and_then(|p| centres.get(p).copied()).unwrap_or(anchor);
        let lay = dim_layout(&frame, c.text.or_else(|| default_text(&frame, away, px)), px, half);
        let stroke = Stroke::new(1.0, col);
        let mut segs = Vec::new();
        for line in &lay.lines {
            let pts: Vec<Pos2> = line.iter().filter_map(|q| to(*q)).collect();
            for w in pts.windows(2) {
                segs.push([w[0], w[1]]);
            }
            painter.add(Shape::line(pts, stroke));
        }
        for (tip, dir) in &lay.arrows {
            if let (Some(t), Some(back)) = (to(*tip), to(*tip - *dir * px)) {
                let d = (t - back).normalized();
                let n = vec2(-d.y, d.x);
                painter.add(Shape::convex_polygon(vec![t, t - d * 9.0 + n * 3.0, t - d * 9.0 - n * 3.0], col, Stroke::NONE));
            }
        }
        let Some(tc) = to(lay.text) else { continue };
        let r = Rect::from_center_size(tc, size);
        painter.rect_filled(r, 2.0, tk.viewport_top.gamma_multiply(0.85));
        painter.galley(r.min + vec2(3.0, 1.0), galley, col);
        if matches!(frame, DimFrame::ArcLength { .. }) {
            // The arc-length mark over the value.
            let c0 = pos2_mid_top(r);
            let pts: Vec<Pos2> = (0..=12)
                .map(|i| c0 + vec2(5.0 * (std::f32::consts::PI * i as f32 / 12.0).cos(), -3.0 * (std::f32::consts::PI * i as f32 / 12.0).sin()))
                .collect();
            painter.add(Shape::line(pts, stroke));
        }
        if is_draft && !placing {
            EDIT.with(|edit| {
                if let Some(e) = edit.borrow_mut().as_mut() {
                    e.at = Some(tc);
                }
            });
        } else if !is_draft {
            drawn.push(Drawn { id: c.id.clone(), param: c.param.clone(), text: r, segs, at: lay.text });
        }
    }
    DRAWN.with(|d| *d.borrow_mut() = drawn);
    edit_box(app, ui.ctx());
}

/// A point a dimension measures (for the shape it belongs to).
fn first_point(sk: &solvecraft_engine::sketch::Sketch, k: &ConstraintKind) -> Option<usize> {
    use ConstraintKind::*;
    match *k {
        Length { l, .. } => sk.curves.get(l).and_then(|c| c.kind.point_ids().first().copied()),
        Distance { p, .. } | DistanceX { p, .. } | DistanceY { p, .. } | PointLineDistance { p, .. } | LinearDiameter { p, .. } => Some(p),
        _ => None,
    }
}

fn pos2_mid_top(r: Rect) -> Pos2 {
    Pos2::new(r.center().x, r.top() - 1.0)
}

/// The inline value editor: Enter applies, Esc (or a click elsewhere) cancels.
/// Edit a dimension's value in place by its parameter (a new dimension from the Dimension tool:
/// its value is selected, so typing replaces it).
pub fn edit_param(app: &SolveApp, param: &str) {
    let expr = app.session.doc.param(param).map(|p| p.expr.clone()).unwrap_or_default();
    EDIT.with(|e| *e.borrow_mut() = Some(Edit { param: param.to_string(), expr, at: None, fresh: FRESH, wait: 10, draft: None }));
}

fn edit_box(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut e) = EDIT.with(|x| x.borrow_mut().take()) else { return };
    if e.at.is_none() {
        e.at = DRAWN.with(|d| d.borrow().iter().find(|x| x.param.as_deref() == Some(e.param.as_str())).map(|x| x.text.center()));
    }
    let Some(at) = e.at else {
        // Not drawn yet (this frame's solve): try again next frame, for a few frames.
        if e.wait > 0 {
            e.wait -= 1;
            EDIT.with(|x| *x.borrow_mut() = Some(e));
        }
        return;
    };
    let id = egui::Id::new("sc_dim_edit");
    let mut done: Option<bool> = None;
    let mut anchor = at - vec2(45.0, 11.0);
    let mut popup_width = 220.0;
    if let Some(layout) = &app.viewport.layout
        && layout.drawable.is_finite()
        && layout.drawable.is_positive()
    {
        let rect = layout.drawable;
        popup_width = (rect.width() - 8.0).clamp(24.0, 220.0);
        anchor.x = anchor.x.clamp(rect.left(), (rect.right() - popup_width - 6.0).max(rect.left()));
        anchor.y = anchor.y.clamp(rect.top(), (rect.bottom() - 80.0).max(rect.top()));
    }
    crate::frame::area(egui::Id::new("sc_dim_edit_area")).fixed_pos(anchor).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(3, 1)).show(ui, |ui| {
            ui.set_max_width(popup_width);
            let r = ui.add(egui::TextEdit::singleline(&mut e.expr).id(id).desired_width(popup_width.min(90.0)));
            crate::params_dialog::complete(ui, &r, &mut e.expr);
            if let Some(error) = e.draft.as_ref().and_then(|d| d.error.as_ref()) {
                ui.add(egui::Label::new(egui::RichText::new(error).color(Tokens::get().error)).wrap());
            }
            if e.fresh == FRESH {
                // The whole value is selected, so typing replaces it.
                let n = e.expr.chars().count();
                let mut s = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                s.cursor.set_char_range(Some(egui::text_selection::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                s.store(ui.ctx(), id);
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if r.lost_focus() && enter {
                done = Some(true);
            } else if e.fresh > 0 {
                if !r.has_focus() {
                    r.request_focus();
                }
            } else if r.lost_focus() {
                done = Some(false);
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
    });
    e.fresh = e.fresh.saturating_sub(1);
    match done {
        Some(true) => {
            let result = if let Some(draft) = &e.draft {
                if draft.valid(app) {
                    draft.evaluate(app, &e.expr).and_then(|_| app.run("sketch.dimension", draft.params(&e.expr)))
                } else {
                    Err("The sketch changed; place the dimension again".into())
                }
            } else {
                app.run("parameters.change", json!({"name": e.param, "expression": e.expr}))
            };
            if let Err(err) = result {
                app.set_status(err, true);
                EDIT.with(|x| *x.borrow_mut() = Some(Edit { fresh: FRESH, ..e }));
            } else {
                app.set_status("Dimension applied", false);
            }
        }
        Some(false) => {}
        None => EDIT.with(|x| *x.borrow_mut() = Some(e)),
    }
}

/// Is the inline editor open (it has the keyboard)?
pub fn editing() -> bool {
    EDIT.with(|e| e.borrow().is_some())
}

/// Dimensions drawn last frame: (id, text centre on screen), for tests and the control channel.
pub fn drawn() -> Vec<(String, Pos2)> {
    DRAWN.with(|d| d.borrow().iter().map(|x| (x.id.clone(), x.text.center())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_previews_radial_and_angular_dimensions_and_invalidates_on_document_change() {
        cancel();
        let mut h = crate::scenario::Harness::new();
        h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
        h.app.run("sketch.circle.center", json!({"center":[30,30],"radius":5})).unwrap();
        h.app.run("sketch.line", json!({"points":[[10,10],[20,10]],"infer":false,"connect":false})).unwrap();
        h.app.run("sketch.line", json!({"points":[[10,10],[10,20]],"infer":false,"connect":false})).unwrap();
        h.call("ui.view", json!({"view":"top"}));
        for (picks, at, angular) in [(vec![[35, 30]], [40, 40], false), (vec![[15, 10], [10, 15]], [18, 18], true)] {
            h.step(&json!({"start":"sketch.dimension"})).unwrap();
            for pick in picks {
                h.step(&json!({"click":{"sketch":pick}})).unwrap();
            }
            h.step(&json!({"move":{"sketch":at}})).unwrap();
            let (param, draft) = placement_preview(&h.app).unwrap();
            let constraint =
                draft.preview.sketch(draft.sketch).unwrap().sketch.constraints.iter().find(|c| c.param.as_deref() == Some(&param)).unwrap();
            assert_eq!(constraint.kind.is_angle(), angular);
            assert!(!editing());
            let (_, cached) = placement_preview(&h.app).unwrap();
            assert!(Arc::ptr_eq(&draft.preview_doc, &cached.preview_doc), "stationary pointer reuses inference");
            h.app.run("sketch.point", json!({"point":[50,50]})).unwrap();
            let (_, refreshed) = placement_preview(&h.app).unwrap();
            assert!(!Arc::ptr_eq(&draft.preview_doc, &refreshed.preview_doc));
            h.step(&json!({"key":"Escape"})).unwrap();
        }
        cancel();
    }

    #[test]
    fn placement_follows_real_pointer_without_editing_or_mutating_design() {
        cancel();
        let mut h = crate::scenario::Harness::new();
        h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
        h.app.run("sketch.line", json!({"points":[[10,10],[30,10]],"infer":false})).unwrap();
        h.call("ui.view", json!({"view":"top"}));
        let baseline = h.app.session.doc.to_json();
        let undo = h.app.session.undo.len();
        h.step(&json!({"start":"sketch.dimension"})).unwrap();
        h.step(&json!({"click":{"sketch":[20,10]}})).unwrap();
        for at in [[20., 20.], [20., -5.]] {
            h.step(&json!({"move":{"sketch":at}})).unwrap();
            h.settle();
            let (param, draft) = placement_preview(&h.app).expect("a line has a dimension preview");
            assert!((draft.at - Vec2::new(at[0], at[1])).len() < 1e-5);
            let sk = &draft.preview.sketch(draft.sketch).unwrap().sketch;
            let c = sk.constraints.iter().find(|c| c.param.as_deref() == Some(&param)).unwrap();
            assert!(matches!(c.kind, ConstraintKind::Length {value,..} if (value-20.).abs()<1e-6));
            assert!(!editing());
            assert!(drawn().is_empty(), "preview must not become a selectable committed dimension");
            assert_eq!(h.app.session.doc.to_json(), baseline);
            assert_eq!(h.app.session.undo.len(), undo);
        }
        h.step(&json!({"click":{"sketch":[20,-5]}})).unwrap();
        assert!(editing());
        assert_eq!(h.app.session.doc.to_json(), baseline);
        h.step(&json!({"key":"Escape"})).unwrap();
        h.step(&json!({"key":"Escape"})).unwrap();
        assert!(placement_preview(&h.app).is_none());
        assert_eq!(h.app.session.doc.to_json(), baseline);
        assert_eq!(h.app.session.undo.len(), undo);
        cancel();
    }

    #[test]
    fn labels_show_the_value_or_fx() {
        let k = ConstraintKind::Length { l: 0, value: 25.0 };
        assert_eq!(label(&k, Some("25 mm"), false), "25.00");
        assert_eq!(label(&k, Some("width * 2"), false), "fx: 25.00");
        assert_eq!(label(&k, None, true), "(25.00)");
        assert_eq!(label(&ConstraintKind::Radius { c: 0, value: 5.0 }, Some("5 mm"), false), "R5.00");
        assert_eq!(label(&ConstraintKind::Diameter { c: 0, value: 10.0 }, Some("10"), false), "Ø10.00");
        let a = ConstraintKind::Angle { a: 0, b: 1, value: std::f64::consts::FRAC_PI_4, flip: false };
        assert_eq!(label(&a, Some("45 deg"), false), "45.0°");
    }
}
