//! The 3D viewport: navigation, GPU (or CPU fallback) rendering, picking, overlays, the axis
//! view and the navigation bar.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use serde_json::json;
use solvecraft_engine::Sel;
use solvecraft_engine::geom::{Vec2, Vec3};
use solvecraft_engine::render::{Camera, Mat4, Rgb};
use solvecraft_engine::sketch::CurveKind;
use solvecraft_engine::view::{colors, grid_step, sketch_consumed, sketch_lines_with_sampler};
use std::hash::{Hash, Hasher};

use crate::gpu::{GpuScene, GpuTarget, SceneSlot, ViewportCallback};
use crate::selection::{BoxSel, origin_quads, origin_size, ray_quad};
use crate::theme::Tokens;
use crate::{SolveApp, icons};

#[cfg(test)]
#[path = "viewport_appearance_tests.rs"]
mod appearance_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavMode {
    Orbit,
    Pan,
    Zoom,
}

#[derive(Default)]
pub struct ViewportState {
    /// Per-body bounding boxes for fast picking, keyed by the model they were taken from.
    pick_bounds: std::sync::Mutex<Option<(usize, std::sync::Arc<Vec<(String, solvecraft_engine::geom::Aabb3)>>)>>,
    /// The last pick (its key and hits): reused while nothing it depends on changed.
    last_pick: Option<(u64, Vec<Hit>)>,
    pub rect: Option<Rect>,
    pub(crate) layout: Option<crate::viewport_layout::Layout>,
    pub(crate) palette_rect: Option<Rect>,
    pub(crate) dialog_rect: Option<Rect>,
    pub gpu: Option<GpuTarget>,
    slot: SceneSlot,
    key: u64,
    sketch_slot: SceneSlot,
    sketch_key: u64,
    physical_scale: f32,
    pub(crate) display: std::sync::Mutex<crate::sketch_display::DisplayCache>,
    hl_slot: SceneSlot,
    hl_key: u64,
    cpu: Option<(u64, egui::TextureHandle)>,
    /// Nothing will look at the frame (the headless scenario harness): skip the software render.
    pub no_pixels: bool,
    pub nav: Option<NavMode>,
    /// What a click would pick now (after the active command's filter).
    pub hover: Option<Hit>,
    pub mouse: Option<Pos2>,
    /// Box selection being dragged.
    pub boxsel: Option<BoxSel>,
    /// Timeline item under the cursor: its bodies or resulting faces are highlighted.
    pub hover_feature: Option<u64>,
    /// Bodies whose browser row is under the cursor: highlighted the same way.
    pub hover_bodies: Vec<String>,
    /// The model point an orbit turns around (Preferences: orbit around the cursor).
    pub orbit_pivot: Option<Vec3>,
    /// Continuous orbit: the view turns about the vertical until the next click or key.
    pub spin: bool,
    /// After immediate zoom, ignore egui's outstanding smoothing until the gesture ends.
    wheel_zoom_raw_until_idle: bool,
    /// A primary press on the axis gizmo: the pointer position used by its last orbit step.
    pub axis_drag: Option<Pos2>,
    /// Unsnapped camera while Shift-orbit previews a standard or diagonal view.
    pub orbit_free: Option<Camera>,
    /// Spring toward the chosen orientation, including after the drag ends.
    pub(crate) orbit_snap: Option<OrbitSnap>,
    pub build_ms: f64,
    /// The right-click menu, open at this screen position.
    pub context_menu: Option<Pos2>,
    /// A sketch point or curve being dragged (its id, the undo depth when the drag began: the
    /// whole drag is one undo step, and for a curve where it was last under the cursor).
    pub point_drag: Option<(String, usize, Option<Vec2>)>,
}

/// Something under the cursor.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    SketchPoint {
        sketch: u64,
        id: String,
        at: Vec2,
    },
    /// A sketch curve; `straight` for lines (usable as an axis).
    SketchCurve {
        sketch: u64,
        id: String,
        straight: bool,
    },
    Vertex {
        body: String,
        point: Vec3,
    },
    Edge {
        body: String,
        index: usize,
        mid: Vec3,
    },
    Profile {
        sketch: u64,
        index: usize,
    },
    Face {
        body: String,
        index: usize,
        point: Vec3,
    },
    /// An origin plane (XY, XZ, YZ) or a construction plane (by name).
    Plane {
        name: String,
        point: Vec3,
    },
    /// An origin axis (X, Y, Z).
    Axis {
        name: String,
    },
}

impl ViewportState {
    pub fn pixels_per_point(&self) -> f32 {
        if self.physical_scale > 0.0 { self.physical_scale } else { 1.0 }
    }
    #[cfg(test)]
    pub(crate) fn set_pixels_per_point(&mut self, scale: f32) {
        self.physical_scale = scale;
    }
}

fn display_proj(app: &SolveApp) -> Proj {
    projection(app, app.viewport.rect.unwrap_or_else(|| Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))))
}

/// World ↔ screen mapping for the current frame.
#[derive(Clone, Copy)]
pub struct Proj {
    pub cam: Camera,
    pub vp: Mat4,
    pub rect: Rect,
}

impl Proj {
    pub fn to_screen(&self, p: Vec3) -> Option<Pos2> {
        let c = self.vp.apply(p);
        if c[3] <= 1e-9 {
            return None;
        }
        let (x, y) = (c[0] / c[3], c[1] / c[3]);
        Some(pos2(
            self.rect.left() + ((x + 1.0) * 0.5 * self.rect.width() as f64) as f32,
            self.rect.top() + ((1.0 - y) * 0.5 * self.rect.height() as f64) as f32,
        ))
    }
    pub fn ray(&self, p: Pos2) -> (Vec3, Vec3) {
        self.cam.ray((p.x - self.rect.left()) as f64, (p.y - self.rect.top()) as f64, self.rect.width() as f64, self.rect.height() as f64)
    }
}

/// A projected 3D face (convex): an unfeathered triangle fan, outlined one edge at a time (no
/// joins between the edges). Unlike `Shape::convex_polygon`, it stays inside its outline when
/// the face turns edge-on.
pub(crate) fn projected_polygon(pts: &[Pos2], fill: Color32, stroke: Stroke) -> Vec<Shape> {
    let mut mesh = egui::Mesh::default();
    for (i, p) in (0u32..).zip(pts) {
        mesh.colored_vertex(*p, fill);
        if i >= 2 {
            mesh.add_triangle(0, i - 1, i);
        }
    }
    let mut out = vec![Shape::mesh(mesh)];
    out.extend(pts.iter().zip(pts.iter().skip(1).chain(pts.first())).map(|(a, b)| Shape::line_segment([*a, *b], stroke)));
    out
}

/// View directions all round, a few degrees apart, none exactly along an axis.
#[cfg(test)]
pub(crate) fn orbit_directions() -> impl Iterator<Item = Vec3> {
    (0..90).flat_map(|i| {
        (0..45).map(move |j| {
            let (yaw, pitch) = (f64::from(i) * 4.0f64.to_radians() + 0.003, f64::from(j) * 4.0f64.to_radians() - 1.5);
            Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin())
        })
    })
}

/// Every vertex egui draws for `shapes`.
#[cfg(test)]
pub(crate) fn drawn_points(shapes: Vec<Shape>) -> Vec<Pos2> {
    let mut tess = egui::epaint::Tessellator::new(1.0, egui::epaint::TessellationOptions::default(), [1, 1], vec![]);
    let mut mesh = egui::Mesh::default();
    for shape in shapes {
        tess.tessellate_shape(shape, &mut mesh);
    }
    mesh.vertices.iter().map(|v| v.pos).collect()
}

fn rgba(c: Rgb) -> [u8; 4] {
    [c.0, c.1, c.2, 255]
}

pub fn scene_radius(app: &SolveApp) -> f64 {
    let mut b = model_bounds(app);
    if !app.preview.bounds.is_empty() {
        b = b.union(&app.preview.bounds);
    }
    let r = if b.is_empty() { 50.0 } else { b.diagonal() * 0.5 + b.center().dist(app.cam.target) };
    r.max(app.cam.half_height() * 2.0).max(10.0)
}

pub fn projection(app: &SolveApp, rect: Rect) -> Proj {
    let mut cam = app.cam;
    cam.fov = if app.ui.perspective { 0.6 } else { 0.0 };
    let aspect = rect.width() as f64 / rect.height().max(1.0) as f64;
    Proj { cam, vp: cam.view_proj(aspect, scene_radius(app)), rect }
}

/// Sketches drawn in the viewport: the active one, and finished ones not yet used by a feature.
fn visible_sketches(app: &SolveApp) -> Vec<u64> {
    let st = app.session.world_state();
    st.sketches
        .iter()
        .filter(|s| {
            app.session.active_sketch == Some(s.feature)
                || (!app.session.visibility.hidden_sketches.contains(&s.feature)
                    && (app.session.visibility.shown_sketches.contains(&s.feature)
                        || (app.ui.show_sketches && !sketch_consumed(&app.session, s.feature))))
        })
        .map(|s| s.feature)
        .collect()
}

fn scene_key(app: &SolveApp) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    app.session.revision.hash(&mut h);
    (std::sync::Arc::as_ptr(&app.session.world_state()) as usize).hash(&mut h);
    app.ui.show_grid.hash(&mut h);
    app.ui.visual_style.hash(&mut h);
    app.ui.ground_shadow.hash(&mut h);
    // The section plane (its cut outlines are part of the model scene).
    section_plane(app).map(|(o, n)| [o.x, o.y, o.z, n.x, n.y, n.z].map(f64::to_bits)).hash(&mut h);
    app.cam.up.hash(&mut h);
    (app.cam.pitch > 0.0).hash(&mut h);
    app.ui.show_sketches.hash(&mut h);
    app.session.visibility.hidden_bodies.hash(&mut h);
    app.session.visibility.hidden_occurrences.hash(&mut h);
    app.session.visibility.hidden_sketches.hash(&mut h);
    app.session.visibility.shown_sketches.hash(&mut h);
    app.ui.hidden_profiles.hash(&mut h);
    app.session.active_component.hash(&mut h);
    app.session.active_occurrence_path.hash(&mut h);
    app.preview.replaced.hash(&mut h);
    app.tree.redefine.hash(&mut h);
    crate::theme::is_dark().hash(&mut h);
    app.session.active_sketch.hash(&mut h);
    h.finish()
}

fn sketch_scene_key(app: &SolveApp, proj: &Proj) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    scene_key(app).hash(&mut h);
    (std::sync::Arc::as_ptr(&app.session.world_state()) as usize).hash(&mut h);
    crate::sketch_display::projection_key(proj, app.viewport.pixels_per_point()).hash(&mut h);
    h.finish()
}

#[cfg(test)]
pub(crate) fn display_scene_keys(app: &SolveApp, proj: &Proj) -> (u64, u64) {
    (scene_key(app), sketch_scene_key(app, proj))
}

/// A sketch curve colour from the engine, in the current theme.
fn sketch_color(t: &Tokens, c: Rgb) -> [u8; 4] {
    if c == colors::SKETCH {
        c4(t.sketch)
    } else if c == colors::SKETCH_DONE || c == colors::SKETCH_FIXED {
        c4(t.sketch_fixed)
    } else if c == colors::SKETCH_LOCKED {
        c4(t.sketch_locked)
    } else {
        rgba(c)
    }
}

fn build_scene(app: &SolveApp) -> GpuScene {
    let tk = Tokens::get();
    let mut sc = GpuScene::default();
    let s = &app.session;
    // Bodies where their occurrences place them.
    let st = s.world_state();
    let active_bodies = s.active_occurrence_path.as_ref().and_then(|p| s.occurrence_bodies(p, true).ok());
    for b in &st.bodies {
        if !app.session.body_visible(&b.name) || app.preview.replaced.contains(&b.name) {
            continue;
        }
        // With a component active, bodies outside it are drawn faded.
        let faded = if let Some(names) = &active_bodies {
            !names.contains(&b.name)
        } else {
            s.active_component != 0 && !s.doc.component_within(b.component, s.active_component)
        };
        let base = body_color(&s.doc, b).unwrap_or(tk.body);
        let col = if faded { c4(base.gamma_multiply(0.35)) } else { c4(base) };
        let m = b.mesh();
        let (shaded, edges) = match app.ui.visual_style {
            1 => (true, false),
            2 => (false, true),
            _ => (true, true),
        };
        let edge_col = if shaded { c4(tk.body_edge) } else { c4(tk.text) };
        // Shaded with hidden edges: edges behind the model show faintly.
        let hidden_edges = app.ui.visual_style == 3;
        let hidden_col = c4(tk.text.gamma_multiply(0.35));
        // Appearances: the body's opacity, and faces with looks of their own.
        let opacity = s.doc.model_body_opacity(b);
        let face_looks: std::collections::HashMap<usize, solvecraft_engine::doc::appearance::Look> =
            if shaded { s.doc.face_colors(b).into_iter().collect() } else { Default::default() };
        for (ti, t) in m.triangles.iter().enumerate().filter(|_| shaded) {
            let (mut col, a) = match m.tri_face.get(ti).and_then(|f| face_looks.get(&(*f as usize))) {
                Some(l) => {
                    let c = Color32::from_rgb(l.color[0], l.color[1], l.color[2]);
                    (if faded { c4(c.gamma_multiply(0.35)) } else { c4(c) }, l.opacity)
                }
                None => (col, opacity),
            };
            let glass = !faded && a < 0.999;
            if glass {
                col[3] = (a.clamp(0.05, 1.0) * 255.0).round() as u8;
            }
            for k in t {
                let i = *k as usize;
                if let (Some(p), Some(n)) = (m.positions.get(i), m.normals.get(i)) {
                    if faded {
                        sc.trans_tri(p.to_f32(), n.to_f32(), col);
                    } else if glass {
                        sc.glass_tri(p.to_f32(), n.to_f32(), col);
                    } else {
                        sc.tri(p.to_f32(), n.to_f32(), col);
                    }
                }
            }
        }
        for (ei, e) in m.edges.iter().enumerate().filter(|_| edges) {
            if !m.edge_pickable(ei) || (shaded && !m.shaded_edge_visible(ei)) {
                continue;
            }
            for w in e.windows(2) {
                sc.line(w[0].to_f32(), w[1].to_f32(), edge_col, 1.3, false);
                if hidden_edges {
                    sc.hidden_line(w[0].to_f32(), w[1].to_f32(), hidden_col, 1.0);
                }
            }
        }
    }
    if app.ui.ground_shadow && app.ui.visual_style != 2 && app.cam.pitch > 0.0 {
        ground_shadow(&mut sc, app, &st);
    }
    // Section: the outline of each body where the plane cuts it.
    if let Some((o, n)) = section_plane(app) {
        let col = c4(tk.text.gamma_multiply(0.9));
        for b in st.bodies.iter().filter(|b| app.session.body_visible(&b.name)) {
            for (a, c) in solvecraft_engine::view::section_segments(&b.mesh(), o, n) {
                sc.line(a.to_f32(), c.to_f32(), col, 1.8, false);
            }
        }
    }

    sc
}

/// A soft dark ellipse on the ground under the shown bodies (darkest in the middle).
fn build_sketch_scene(app: &SolveApp, proj: &Proj) -> GpuScene {
    let tk = Tokens::get();
    let mut sc = GpuScene::default();
    let s = &app.session;
    let st = s.world_state();
    let mut fill_budget = 131_072;
    if app.ui.show_grid && !active_view(app).hide_grid {
        let (minor, major) = grid_step(app.cam.half_height());
        let ext = (app.cam.half_height() * 3.0 / major).ceil() * major;
        // The ground plane: XY with Z up, XZ with Y up (laid out in the camera's Z-up frame).
        let tf = app.cam.to_frame(app.cam.target);
        let c = Vec3::new((tf.x / major).round() * major, (tf.y / major).round() * major, 0.0);
        let w3 = |p: Vec3| app.cam.from_frame(p).to_f32();
        let n = ((ext / minor) as i64).clamp(1, 300);
        for i in -n..=n {
            let t = i as f64 * minor;
            let maj = (t / major).round() * major == t;
            let col = if maj { c4(tk.grid_major) } else { c4(tk.grid) };
            let w = if maj { 1.0 } else { 0.7 };
            sc.under_line(w3(c + Vec3::new(t, -ext, 0.0)), w3(c + Vec3::new(t, ext, 0.0)), col, w);
            sc.under_line(w3(c + Vec3::new(-ext, t, 0.0)), w3(c + Vec3::new(ext, t, 0.0)), col, w);
        }
    }
    for sid in visible_sketches(app) {
        let Some(ss) = st.sketch(sid) else { continue };
        let active = s.active_sketch == Some(sid);
        // Closed profiles are shaded (lifted off a face they may lie on).
        let lift = ss.plane.normal() * 0.01;
        let profiles = if app.ui.hidden_profiles.contains(&sid) || ss.sketch.view.hide_profile { &[][..] } else { &ss.profiles[..] };
        for (index, _) in profiles.iter().enumerate() {
            let display = crate::sketch_display::profile_region(app, sid, index, &ss.plane, proj);
            for tri in display.as_ref().as_ref().map_or_else(Vec::new, |region| crate::sketch_display::fill_triangles(region, &mut fill_budget)) {
                for q in tri {
                    sc.trans_tri((ss.plane.to_world(q) + lift).to_f32(), [0.0; 3], c4(tk.profile_fill));
                }
            }
        }
        // Construction curves are dashed, centerlines dash-dot (lengths follow the grid).
        let unit = grid_step(app.cam.half_height()).0;
        for (pts, col, cons) in sketch_lines_with_sampler(&ss.sketch, &ss.plane, active, &ss.report.curve_determined, |_, index, plane| {
            crate::sketch_display::curve_sample(app, sid, index, plane, proj).parts.clone()
        }) {
            let segs = if !cons {
                pts.windows(2).map(|w| (w[0], w[1])).collect()
            } else if col == colors::SKETCH_CENTERLINE {
                dashes(&pts, &[0.6 * unit, 0.15 * unit, 0.05 * unit, 0.15 * unit])
            } else {
                dashes(&pts, &[0.3 * unit, 0.2 * unit])
            };
            for (a, b) in segs {
                sc.line(a.to_f32(), b.to_f32(), sketch_color(&tk, col), if cons { 1.2 } else { 2.0 }, active);
            }
        }
    }
    let guide_unit = grid_step(app.cam.half_height()).0;
    for points in crate::sketch_attachment_ui::display_guide_lines(app, proj) {
        for (a, b) in dashes(&points, &[guide_unit * 0.25, guide_unit * 0.15]) {
            sc.line(a.to_f32(), b.to_f32(), c4(tk.warning), 1.5, true);
        }
    }
    sc
}

fn ground_shadow(sc: &mut GpuScene, app: &SolveApp, st: &solvecraft_engine::doc::ModelState) {
    // The bodies' box in the camera's Z-up frame (the ground is its XY plane).
    let mut b = solvecraft_engine::geom::Aabb3::EMPTY;
    for x in st.bodies.iter().filter(|x| app.session.body_visible(&x.name)) {
        let bb = x.mesh().bounds();
        if bb.is_empty() {
            continue;
        }
        for i in 0..8 {
            let p = Vec3::new(
                if i & 1 == 0 { bb.min.x } else { bb.max.x },
                if i & 2 == 0 { bb.min.y } else { bb.max.y },
                if i & 4 == 0 { bb.min.z } else { bb.max.z },
            );
            b.add(app.cam.to_frame(p));
        }
    }
    if b.is_empty() {
        return;
    }
    let size = b.size();
    let c = b.center();
    let z = b.min.z - 1e-3 * (1.0 + b.diagonal());
    let (rx, ry) = (size.x * 0.62 + size.z * 0.15, size.y * 0.62 + size.z * 0.15);
    if !(rx > 0.0 && ry > 0.0) {
        return;
    }
    let dark = if crate::theme::is_dark() { 150 } else { 80 };
    // Rings from the middle out, fading to nothing.
    let rings = [(0.0, dark), (0.55, dark * 3 / 4), (0.85, dark / 3), (1.0, 0)];
    let n = 48;
    let at = |r: f64, k: usize| {
        let a = k as f64 / n as f64 * std::f64::consts::TAU;
        app.cam.from_frame(Vec3::new(c.x + rx * r * a.cos(), c.y + ry * r * a.sin(), z)).to_f32()
    };
    for w in rings.windows(2) {
        let ((r0, a0), (r1, a1)) = (w[0], w[1]);
        let (c0, c1) = ([0, 0, 0, a0 as u8], [0, 0, 0, a1 as u8]);
        for k in 0..n {
            let (p00, p01, p10, p11) = (at(r0, k), at(r0, k + 1), at(r1, k), at(r1, k + 1));
            sc.trans_tri(p00, [0.0; 3], c0);
            sc.trans_tri(p10, [0.0; 3], c1);
            sc.trans_tri(p11, [0.0; 3], c1);
            if r0 > 0.0 {
                sc.trans_tri(p00, [0.0; 3], c0);
                sc.trans_tri(p11, [0.0; 3], c1);
                sc.trans_tri(p01, [0.0; 3], c0);
            }
        }
    }
}

/// The drawn pieces of a polyline in a dash pattern (on, off, on, off… lengths).
pub(crate) fn dashes(pts: &[Vec3], pattern: &[f64]) -> Vec<(Vec3, Vec3)> {
    let mut out = Vec::new();
    if pattern.is_empty() || pattern.iter().any(|l| !(l.is_finite() && *l > 0.0)) {
        return pts.windows(2).map(|w| (w[0], w[1])).collect();
    }
    let (mut k, mut left) = (0usize, pattern[0]);
    for w in pts.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut len = (b - a).len();
        // Cap the pieces per segment so a huge zoom-out can't explode the scene.
        let mut guard = 0;
        while len > 1e-12 && guard < 10_000 {
            guard += 1;
            let step = left.min(len);
            let q = a + (b - a) * (step / len);
            if k % 2 == 0 {
                out.push((a, q));
            }
            left -= step;
            len -= step;
            a = q;
            if left <= 1e-12 {
                k = (k + 1) % pattern.len();
                left = pattern.get(k).copied().unwrap_or(1.0);
            }
        }
    }
    out
}

/// Colour as straight (unpremultiplied) sRGBA bytes for the GPU.
fn c4(c: Color32) -> [u8; 4] {
    c.to_srgba_unmultiplied()
}

/// Origin planes and axes currently shown, sized for the view: (name, normal, quad).
pub fn origin_planes(app: &SolveApp) -> Vec<(&'static str, Vec3, [Vec3; 4])> {
    if !app.origin_visible() {
        return Vec::new();
    }
    // While the Origin is hidden, Create Sketch may bring the planes back (empty design): then
    // every plane; otherwise only those not hidden one by one.
    let creating = !app.ui.show_origin && app.dialog.as_ref().is_some_and(|d| matches!(d.kind, crate::dialogs::Kind::Sketch));
    origin_quads(origin_size(app.cam.half_height()))
        .into_iter()
        .filter(|(n, _, _)| creating || !app.session.visibility.hidden_origin.iter().any(|h| h == n))
        .collect()
}

pub fn origin_axes(app: &SolveApp) -> Vec<(&'static str, Vec3)> {
    if !app.origin_visible() {
        return Vec::new();
    }
    [("X", Vec3::X), ("Y", Vec3::Y), ("Z", Vec3::Z)]
        .into_iter()
        .filter(|(n, _)| !app.session.visibility.hidden_origin.iter().any(|h| h == n))
        .collect()
}

/// Construction axes shown, as segments: across the model where the axis passes it (a little
/// past it either side), at least the origin axes' length around the axis's origin: (name,
/// from, to).
pub fn construction_axis_segments(app: &SolveApp) -> Vec<(String, Vec3, Vec3)> {
    let h = origin_size(app.cam.half_height()) * AXIS_LEN;
    let bb = solvecraft_engine::view::bounds(&app.session);
    solvecraft_engine::view::construction_axes(&app.session)
        .into_iter()
        .filter(|(_, n, _, _)| !app.session.visibility.hidden_origin.contains(n))
        .map(|(_, name, o, d)| {
            let (mut lo, mut hi) = (-h, h);
            if !bb.is_empty() {
                for i in 0..8 {
                    let c = Vec3::new(
                        if i & 1 == 0 { bb.min.x } else { bb.max.x },
                        if i & 2 == 0 { bb.min.y } else { bb.max.y },
                        if i & 4 == 0 { bb.min.z } else { bb.max.z },
                    );
                    let t = (c - o).dot(d);
                    lo = lo.min(t - h * 0.3);
                    hi = hi.max(t + h * 0.3);
                }
            }
            (name, o + d * lo, o + d * hi)
        })
        .collect()
}

/// Construction points shown: (name, position).
pub fn construction_points(app: &SolveApp) -> Vec<(String, Vec3)> {
    solvecraft_engine::view::construction_points(&app.session)
        .into_iter()
        .filter(|(_, n, _)| !app.session.visibility.hidden_origin.contains(n))
        .map(|(_, n, p)| (n, p))
        .collect()
}

/// Construction planes drawn as squares around their origin: (name, normal, quad).
pub fn construction_quads(app: &SolveApp) -> Vec<(String, Vec3, [Vec3; 4])> {
    let h = origin_size(app.cam.half_height()) * 1.2;
    solvecraft_engine::view::construction_planes(&app.session)
        .into_iter()
        .filter(|(_, n, _)| !app.session.visibility.hidden_origin.contains(n))
        .map(|(_, name, pl)| {
            let (o, x, y) = (pl.origin, pl.x * h, pl.y * h);
            (name, pl.normal(), [o - x - y, o + x - y, o + x + y, o - x + y])
        })
        .collect()
}

/// Pick what is under `pos`: sketch points, sketch curves, vertices, edges and axes first, then
/// the surfaces along the ray (profiles, faces, planes) nearest first.
/// Bounding boxes of the shown model's bodies (cached until the model changes).
fn body_bounds(
    app: &SolveApp,
    st: &std::sync::Arc<solvecraft_engine::doc::ModelState>,
) -> std::sync::Arc<Vec<(String, solvecraft_engine::geom::Aabb3)>> {
    let key = std::sync::Arc::as_ptr(st) as usize;
    if let Ok(mut g) = app.viewport.pick_bounds.lock() {
        if let Some((k, b)) = g.as_ref()
            && *k == key
        {
            return b.clone();
        }
        let b = std::sync::Arc::new(st.bodies.iter().map(|b| (b.name.clone(), b.mesh().bounds())).collect::<Vec<_>>());
        *g = Some((key, b.clone()));
        return b;
    }
    std::sync::Arc::new(Vec::new())
}

/// The model's extent: bodies (from the cached boxes) and sketches.
pub fn model_bounds(app: &SolveApp) -> solvecraft_engine::geom::Aabb3 {
    let st = app.session.world_state();
    let mut b = body_bounds(app, &st).iter().fold(solvecraft_engine::geom::Aabb3::EMPTY, |a, (_, x)| a.union(x));
    for ss in &st.sketches {
        if let Some((lo, hi)) = ss.sketch.bounds() {
            for p in [lo, hi] {
                b.add(ss.plane.to_world(p));
            }
        }
    }
    for point in crate::sketch_attachment_ui::guide_lines(app).into_iter().flatten() {
        b.add(point);
    }
    b
}

/// Where a ray enters a box (None: it misses).
fn ray_box(o: Vec3, d: Vec3, b: &solvecraft_engine::geom::Aabb3, pad: f64) -> Option<f64> {
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for (oo, dd, lo, hi) in [(o.x, d.x, b.min.x, b.max.x), (o.y, d.y, b.min.y, b.max.y), (o.z, d.z, b.min.z, b.max.z)] {
        let (lo, hi) = (lo - pad, hi + pad);
        if dd.abs() < 1e-15 {
            if oo < lo || oo > hi {
                return None;
            }
        } else {
            let (a, c) = ((lo - oo) / dd, (hi - oo) / dd);
            t0 = t0.max(a.min(c));
            t1 = t1.min(a.max(c));
        }
    }
    (t0 <= t1).then_some(t0)
}

/// A box's outline on screen, grown by `px` (None: behind the eye).
fn screen_box(proj: &Proj, b: &solvecraft_engine::geom::Aabb3, px: f32) -> Option<Rect> {
    let mut r = Rect::NOTHING;
    for i in 0..8 {
        let p = Vec3::new(
            if i & 1 == 0 { b.min.x } else { b.max.x },
            if i & 2 == 0 { b.min.y } else { b.max.y },
            if i & 4 == 0 { b.min.z } else { b.max.z },
        );
        r.extend_with(proj.to_screen(p)?);
    }
    Some(r.expand(px))
}

/// What is under `pos`, reusing the last answer while the cursor, the camera and the model are
/// unchanged.
fn pick_cached(app: &mut SolveApp, proj: &Proj, pos: Pos2) -> Vec<Hit> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    pos.x.to_bits().hash(&mut h);
    pos.y.to_bits().hash(&mut h);
    for v in [app.cam.yaw, app.cam.pitch, app.cam.roll, app.cam.distance, app.cam.target.x, app.cam.target.y, app.cam.target.z] {
        v.to_bits().hash(&mut h);
    }
    app.session.revision.hash(&mut h);
    (std::sync::Arc::as_ptr(&app.session.world_state()) as usize).hash(&mut h);
    app.session.active_sketch.hash(&mut h);
    app.dialog.as_ref().and_then(|d| d.active_input()).map(|i| i.accept).hash(&mut h);
    app.session.visibility.hidden_bodies.hash(&mut h);
    app.session.visibility.hidden_occurrences.hash(&mut h);
    app.ui.perspective.hash(&mut h);
    (proj.rect.width().to_bits(), proj.rect.height().to_bits()).hash(&mut h);
    crate::sketch_display::projection_key(proj, app.viewport.pixels_per_point()).hash(&mut h);
    let key = h.finish();
    if let Some((k, hits)) = &app.viewport.last_pick
        && *k == key
    {
        return hits.clone();
    }
    let hits = pick(app, proj, pos);
    app.viewport.last_pick = Some((key, hits.clone()));
    hits
}

pub fn pick(app: &SolveApp, proj: &Proj, pos: Pos2) -> Vec<Hit> {
    let s = &app.session;
    let st = s.world_state();
    let bounds = body_bounds(app, &st);
    let bound = |name: &str| bounds.iter().find(|(n, _)| n == name).map(|(_, b)| *b);
    let mut hits = Vec::new();
    let (o, d) = proj.ray(pos);
    // Sketch points of the active sketch (of every shown sketch while a command takes points).
    let wants_points = app.dialog.as_ref().and_then(|d| d.active_input()).is_some_and(|i| i.accept & crate::selection::POINTS != 0);
    let point_sketches: Vec<u64> = if wants_points { visible_sketches(app) } else { s.active_sketch.into_iter().collect() };
    let mut best: Option<(f32, Hit)> = None;
    for ss in point_sketches.iter().filter_map(|sid| st.sketch(*sid)) {
        let sid = ss.feature;
        for p in &ss.sketch.points {
            if let Some(sp) = proj.to_screen(ss.plane.to_world(p.pos)) {
                let dd = sp.distance(pos);
                if dd < 8.0 && best.as_ref().is_none_or(|(b, _)| dd < *b) {
                    best = Some((dd, Hit::SketchPoint { sketch: sid, id: p.id.clone(), at: p.pos }));
                }
            }
        }
    }
    if let Some((_, h)) = best {
        hits.push(h);
    }
    // Sketch curves (the active sketch wins ties).
    let mut bestc: Option<(f32, Hit)> = None;
    for sid in visible_sketches(app) {
        let Some(ss) = st.sketch(sid) else { continue };
        let bonus = if s.active_sketch == Some(sid) { 2.0 } else { 0.0 };
        for (i, c) in ss.sketch.curves.iter().enumerate() {
            if c.link.is_some() && ss.sketch.view.hide_projected {
                continue;
            }
            let straight = matches!(c.kind, CurveKind::Line { .. });
            for part in &crate::sketch_display::curve_sample(app, sid, i, &ss.plane, proj).parts {
                let pts: Vec<Pos2> = part.iter().filter_map(|q| proj.to_screen(*q)).collect();
                for w in pts.windows(2) {
                    let dd = seg_dist(pos, w[0], w[1]) - bonus;
                    if dd < 6.0 && bestc.as_ref().is_none_or(|(b, _)| dd < *b) {
                        bestc = Some((dd, Hit::SketchCurve { sketch: sid, id: c.id.clone(), straight }));
                    }
                }
            }
        }
        // 3D sketch curves (paths for sweeps and pipes).
        for w in &ss.sketch.wires {
            let pts: Vec<Pos2> = w.pts.iter().filter_map(|q| proj.to_screen(*q)).collect();
            for seg in pts.windows(2) {
                let dd = seg_dist(pos, seg[0], seg[1]) - bonus;
                if dd < 6.0 && bestc.as_ref().is_none_or(|(b, _)| dd < *b) {
                    bestc = Some((dd, Hit::SketchCurve { sketch: sid, id: w.id.clone(), straight: false }));
                }
            }
        }
    }
    if let Some((_, h)) = bestc {
        hits.push(h);
    }
    // Nearest face along the ray (hides what is behind it).
    let mut bestf: Option<(f64, Hit)> = None;
    for b in &st.bodies {
        if !app.session.body_visible(&b.name) {
            continue;
        }
        // Bodies whose box the ray misses (or enters behind the best hit so far) are skipped.
        if let Some(bb) = bound(&b.name) {
            let pad = bb.diagonal() * 1e-6 + 1e-9;
            match ray_box(o, d, &bb, pad) {
                None => continue,
                Some(t0) if bestf.as_ref().is_some_and(|(bt, _)| t0 > *bt) => continue,
                _ => {}
            }
        }
        let m = b.mesh();
        if let Some((t, ti)) = m.raycast(o, d)
            && bestf.as_ref().is_none_or(|(bt, _)| t < *bt)
        {
            let fi = m.tri_face.get(ti).copied().unwrap_or(0) as usize;
            bestf = Some((t, Hit::Face { body: b.name.clone(), index: fi, point: o + d * t }));
        }
    }
    let slack = scene_radius(app) * 0.02;
    let visible = |p: Vec3| bestf.as_ref().is_none_or(|(t, _)| (p - o).dot(d) <= *t + slack);
    // Vertices (edge end points) and edges.
    let mut bestv: Option<(f32, Hit)> = None;
    let mut beste: Option<(f32, Hit)> = None;
    for b in &st.bodies {
        if !app.session.body_visible(&b.name) {
            continue;
        }
        // Only bodies whose outline on screen is near the cursor can have an edge under it.
        if let Some(bb) = bound(&b.name)
            && screen_box(proj, &bb, 8.0).is_some_and(|r| !r.contains(pos))
        {
            continue;
        }
        let m = b.mesh();
        for (ei, e) in m.edges.iter().enumerate() {
            if !m.edge_pickable(ei) {
                continue;
            }
            for v in [e.first(), e.last()].into_iter().flatten() {
                if let Some(sp) = proj.to_screen(*v) {
                    let dd = sp.distance(pos);
                    if dd < 7.0 && visible(*v) && bestv.as_ref().is_none_or(|(bd, _)| dd < *bd) {
                        bestv = Some((dd, Hit::Vertex { body: b.name.clone(), point: *v }));
                    }
                }
            }
            let pts: Vec<Pos2> = e.iter().filter_map(|q| proj.to_screen(*q)).collect();
            for (k, w) in pts.windows(2).enumerate() {
                let dd = seg_dist(pos, w[0], w[1]);
                if dd < 6.0 && beste.as_ref().is_none_or(|(bd, _)| dd < *bd) {
                    let mid = polyline_mid(e);
                    // Visible where the cursor is (not just at the middle).
                    let at = e.get(k).copied().unwrap_or(mid);
                    if visible(at) || visible(mid) {
                        beste = Some((dd, Hit::Edge { body: b.name.clone(), index: ei, mid }));
                    }
                }
            }
        }
    }
    hits.extend(bestv.map(|x| x.1));
    hits.extend(beste.map(|x| x.1));
    // Origin axes.
    let size = origin_size(app.cam.half_height());
    let mut besta: Option<(f32, Hit)> = None;
    for (name, dir) in origin_axes(app) {
        if let (Some(a), Some(b)) = (proj.to_screen(Vec3::ZERO), proj.to_screen(dir * (size * AXIS_LEN))) {
            let dd = seg_dist(pos, a, b);
            if dd < 5.0 && besta.as_ref().is_none_or(|(bd, _)| dd < *bd) {
                besta = Some((dd, Hit::Axis { name: name.into() }));
            }
        }
    }
    // Construction axes, picked like the origin axes.
    for (name, a, b) in construction_axis_segments(app) {
        if let (Some(a), Some(b)) = (proj.to_screen(a), proj.to_screen(b)) {
            let dd = seg_dist(pos, a, b);
            if dd < 5.0 && besta.as_ref().is_none_or(|(bd, _)| dd < *bd) {
                besta = Some((dd, Hit::Axis { name }));
            }
        }
    }
    hits.extend(besta.map(|x| x.1));
    // Surfaces by depth: profiles (slightly preferred over a face they lie on), faces, planes.
    let mut surf: Vec<(f64, Hit)> = Vec::new();
    let mut bestp: Option<(f64, f64, Hit)> = None;
    for sid in visible_sketches(app) {
        let Some(ss) = st.sketch(sid) else { continue };
        let n = ss.plane.normal();
        let den = n.dot(d);
        if den.abs() < 1e-12 {
            continue;
        }
        let t = n.dot(ss.plane.origin - o) / den;
        if t < 0.0 {
            continue;
        }
        let lp = ss.plane.to_local(o + d * t);
        // Show Profile off: the sketch being edited offers no profiles.
        let profiles = if ss.sketch.view.hide_profile && s.active_sketch == Some(sid) { &[][..] } else { &ss.profiles[..] };
        for (pi, p) in profiles.iter().enumerate() {
            if p.region.contains(lp) && bestp.as_ref().is_none_or(|(_, a, _)| p.area < *a) {
                bestp = Some((t, p.area, Hit::Profile { sketch: sid, index: pi }));
            }
        }
    }
    if let Some((t, _, h)) = bestp {
        surf.push((t - slack, h));
    }
    if let Some(f) = bestf {
        surf.push(f);
    }
    // Planes give way to a face or profile at the same depth (a floor on the XZ plane).
    for (name, _, q) in origin_planes(app) {
        if let Some(t) = ray_quad(o, d, &q) {
            surf.push((t + slack, Hit::Plane { name: name.into(), point: o + d * t }));
        }
    }
    for (name, _, q) in construction_quads(app) {
        if let Some(t) = ray_quad(o, d, &q) {
            surf.push((t + slack, Hit::Plane { name, point: o + d * t }));
        }
    }
    surf.sort_by(|a, b| a.0.total_cmp(&b.0));
    hits.extend(surf.into_iter().map(|x| x.1));
    hits
}

/// The section cut shown now: the open Section Analysis dialog's plane, else the session's.
/// The active sketch's display options (defaults when not sketching).
pub fn active_view(app: &SolveApp) -> solvecraft_engine::sketch::SketchView {
    let st = app.session.world_state();
    app.session.active_sketch.and_then(|s| st.sketch(s)).map(|ss| ss.sketch.view.clone()).unwrap_or_default()
}

pub fn section_plane(app: &SolveApp) -> Option<(Vec3, Vec3)> {
    if let Some(d) = app.dialog.as_ref().filter(|d| matches!(d.kind, crate::dialogs::Kind::Section { .. })) {
        let cmds = crate::dialogs::apply_commands(app, d).ok()?;
        let (_, p) = cmds.into_iter().next()?;
        let mut scratch = app.session.scratch();
        scratch.execute("inspect.section", &p).ok()?;
        return scratch.section;
    }
    // Slice (Sketch Palette): cut the model at the plane of the sketch being edited.
    if app.session.section.is_none() {
        let st = app.session.world_state();
        if let Some(ss) = app.session.active_sketch.and_then(|s| st.sketch(s)).filter(|ss| ss.sketch.view.slice) {
            return Some((ss.plane.origin, ss.plane.normal()));
        }
    }
    app.session.section
}

/// Visible geometry belonging to a timeline feature. Body creators retain their whole-body
/// feedback; modifiers use persistent face names, which survive later edits and placement.
/// Do not expand these faces into surface patches: neighbouring faces can belong to another
/// feature even when they share a surface.
fn feature_geometry(app: &SolveApp, id: u64) -> Vec<Sel> {
    if !app.session.model.result(id).is_some_and(|r| !r.skipped && r.error.is_none()) {
        return Vec::new();
    }
    let st = app.session.world_state();
    let prefix = format!("F{id}:");
    let mut out = Vec::new();
    for b in &st.bodies {
        if !app.session.body_visible(&b.name) || app.preview.replaced.contains(&b.name) {
            continue;
        }
        if b.feature == id {
            out.push(Sel::Body { name: b.name.clone() });
            continue;
        }
        let names = solvecraft_engine::doc::naming::face_names(b);
        let m = b.mesh();
        for (index, _) in names.iter().enumerate().filter(|(_, n)| n.starts_with(&prefix)) {
            if let Some(point) = m.triangles.iter().zip(&m.tri_face).find_map(|(t, f)| {
                if *f as usize != index {
                    return None;
                }
                m.tri(t).map(|[a, b, c]| (a + b + c) / 3.0)
            }) {
                out.push(Sel::Face { body: b.name.clone(), index, point });
            }
        }
    }
    out
}

fn displayed_selection(app: &SolveApp, item: &Sel) -> Vec<Sel> {
    match item {
        Sel::Feature { id } => feature_geometry(app, *id),
        Sel::Occurrence { path } => app
            .session
            .occurrence_bodies(path, true)
            .unwrap_or_default()
            .into_iter()
            .filter(|name| app.session.body_visible(name))
            .map(|name| Sel::Body { name })
            .collect(),
        _ => grouped_selection(&app.session, item),
    }
}

/// The box around what is selected (bodies, faces, edges, vertices, sketch curves and the
/// bodies of selected features), where it is shown.
pub fn selection_bounds(app: &SolveApp) -> Option<solvecraft_engine::geom::Aabb3> {
    let st = app.session.world_state();
    let mut b = solvecraft_engine::geom::Aabb3::EMPTY;
    for s in app.session.selection.iter().flat_map(|item| displayed_selection(app, item)) {
        match &s {
            Sel::Body { name } => {
                if let Some(x) = st.body(name) {
                    b = b.union(&x.mesh().bounds());
                }
            }
            Sel::Face { body, index, .. } => {
                if let Some(x) = st.body(body) {
                    let m = x.mesh();
                    for (t, f) in m.triangles.iter().zip(&m.tri_face) {
                        if *f as usize == *index {
                            for p in m.tri(t).into_iter().flatten() {
                                b.add(p);
                            }
                        }
                    }
                }
            }
            Sel::Edge { body, index, .. } => {
                if let Some(e) = st.body(body).and_then(|x| x.mesh().edges.get(*index).cloned()) {
                    for p in e {
                        b.add(p);
                    }
                }
            }
            Sel::Vertex { point, .. } => b.add(*point),
            Sel::SketchCurve { id } => {
                for ss in &st.sketches {
                    if let Some(i) = ss.sketch.curve_index(id) {
                        for q in ss.sketch.polyline(i) {
                            b.add(ss.plane.to_world(q));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (!b.is_empty()).then_some(b)
}

/// The section as the shader's clip plane (normal, d), zero when there is none.
/// The surface analysis for the shader (see `gpu::ViewportCallback::analysis`).
fn analysis_uniform(app: &SolveApp) -> [f32; 8] {
    use solvecraft_engine::SurfaceAnalysis as A;
    match app.session.analysis {
        Some(A::Zebra { stripes }) => [1.0, stripes as f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        Some(A::Draft { pull, angle }) => [2.0, angle as f32, 0.0, 0.0, pull.x as f32, pull.y as f32, pull.z as f32, 0.0],
        Some(A::Curvature { radius }) => [3.0, radius as f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        Some(A::Environment) => [4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        Some(A::Access { dir }) => {
            let (scale, size, eps) = crate::access::map(app).map(|m| (m.scale, m.size as f32, 1.5 / m.scale.max(1e-9))).unwrap_or((0.0, 0.0, 0.0));
            [5.0, eps, scale, size, dir.x as f32, dir.y as f32, dir.z as f32, 0.0]
        }
        None => [0.0; 8],
    }
}

fn section_clip(app: &SolveApp) -> [f32; 4] {
    match section_plane(app) {
        Some((o, n)) => [n.x as f32, n.y as f32, n.z as f32, n.dot(o) as f32],
        None => [0.0; 4],
    }
}

/// Space kept free at the top right of the viewport for the axis view (docked dialogs start below).
pub const AXIS_VIEW_CLEARANCE: f32 = 168.0;

/// Axis length as a multiple of the origin plane size (solid part 1, dashed beyond).
const AXIS_LEN: f64 = 1.8;

/// The hit as a plain selection (no command filter).
pub fn hit_sel(h: &Hit) -> Option<Sel> {
    Some(match h {
        Hit::SketchPoint { id, .. } => Sel::SketchPoint { id: id.clone() },
        Hit::SketchCurve { id, .. } => Sel::SketchCurve { id: id.clone() },
        Hit::Vertex { body, point } => Sel::Vertex { body: body.clone(), point: *point },
        Hit::Edge { body, index, mid } => Sel::Edge { body: body.clone(), index: *index, point: *mid },
        Hit::Profile { sketch, index } => Sel::Profile { sketch: *sketch, index: *index },
        Hit::Face { body, index, point } => Sel::Face { body: body.clone(), index: *index, point: *point },
        Hit::Plane { name, .. } => Sel::Plane { name: name.clone() },
        Hit::Axis { name } => Sel::Axis { name: name.clone() },
    })
}

/// What a click would pick: the first hit the open dialog's active input accepts, or with no
/// dialog the first hit (sketch entities only belong to the active sketch).
pub fn candidate(app: &SolveApp, hits: &[Hit]) -> Option<(Hit, Sel)> {
    if let Some(d) = app.dialog.as_ref().filter(|d| d.wants_picks()) {
        return hits.iter().find_map(|h| d.candidate(&app.session, h).map(|s| (h.clone(), s)));
    }
    let off = |k: &str| app.ui.pick_off.iter().any(|x| x == k);
    hits.iter()
        .filter(|h| match h {
            Hit::SketchCurve { sketch, .. } => app.session.active_sketch == Some(*sketch) && !off("sketch") && !off("sketch_curves"),
            Hit::SketchPoint { .. } => !off("sketch") && !off("sketch_points"),
            Hit::Profile { .. } => !off("sketch"),
            Hit::Vertex { .. } => !off("vertices") && !app.ui.pick_bodies,
            Hit::Edge { .. } => !off("edges") && !app.ui.pick_bodies,
            Hit::Face { .. } => !off("faces") || app.ui.pick_bodies,
            Hit::Plane { .. } | Hit::Axis { .. } => true,
        })
        .find_map(|h| match h {
            Hit::Face { body, .. } if app.ui.pick_bodies => Some((h.clone(), Sel::Body { name: body.clone() })),
            _ => hit_sel(h).map(|s| (h.clone(), s)),
        })
}

/// The edge a selection refers to now: by index if it still passes through the stored point,
/// otherwise the edge nearest the point.
fn edge_of(m: &solvecraft_engine::geom::Mesh, index: usize, point: Vec3) -> Option<&Vec<Vec3>> {
    let tol = m.bounds().diagonal().max(1.0) * 1e-6;
    let near = |e: &Vec<Vec3>| e.windows(2).map(|w| point.dist_to_segment(w[0], w[1])).fold(f64::INFINITY, f64::min);
    if let Some(e) = m.edges.get(index)
        && near(e) < tol
    {
        return Some(e);
    }
    m.edges.iter().min_by(|a, b| near(a).total_cmp(&near(b))).filter(|e| near(e) < tol * 1e3)
}

fn face_tris(sc: &mut GpuScene, m: &solvecraft_engine::geom::Mesh, face: usize, col: [u8; 4], lit: bool) {
    for (t, f) in m.triangles.iter().zip(&m.tri_face) {
        if *f as usize != face {
            continue;
        }
        for k in t {
            let i = *k as usize;
            if let (Some(p), Some(n)) = (m.positions.get(i), m.normals.get(i)) {
                sc.tri(p.to_f32(), if lit { n.to_f32() } else { [0.0; 3] }, col);
            }
        }
    }
}

/// A highlighted edge: a halo and a core, and where the model hides it, the core dimmed (so a
/// pick on the far side still shows).
fn edge_lines(sc: &mut GpuScene, e: &[Vec3], core: Color32, halo: Color32, w: f32) {
    for (col, width) in [(halo, w + 3.0), (core, w)] {
        for p in e.windows(2) {
            sc.line(p[0].to_f32(), p[1].to_f32(), c4(col), width, false);
        }
    }
    for p in e.windows(2) {
        sc.hidden_line(p[0].to_f32(), p[1].to_f32(), c4(core.gamma_multiply(0.45)), (w * 0.7).max(1.0));
    }
}

/// The outline of a highlighted face where the model hides it (dimmed).
fn hidden_face_outline(sc: &mut GpuScene, m: &solvecraft_engine::geom::Mesh, face: usize, col: Color32) {
    for (ei, e) in m.edges.iter().enumerate() {
        let on = m.edge_faces.get(ei).is_some_and(|f| f.iter().any(|x| *x as usize == face));
        if on && m.edge_pickable(ei) {
            for p in e.windows(2) {
                sc.hidden_line(p[0].to_f32(), p[1].to_f32(), c4(col.gamma_multiply(0.45)), 1.4);
            }
        }
    }
}

pub(crate) fn profile_fill(sc: &mut GpuScene, app: &SolveApp, sketch: u64, index: usize, fill: Color32, edge: Color32) {
    let st = app.session.world_state();
    let Some(ss) = st.sketch(sketch) else { return };
    let Some(p) = ss.profiles.get(index) else { return };
    let plane = match crate::dialogs::emboss_profile_plane(app, sketch) {
        Ok(Some(plane)) => plane,
        Ok(None) => ss.plane,
        Err(_) => return,
    };
    // Lift toward the eye so a profile on a face shows over it.
    let lift = app.cam.back() * (app.cam.half_height() * 0.002);
    let w = |q: solvecraft_engine::geom::Vec2| (plane.to_world(q) + lift).to_f32();
    let col = c4(fill);
    // In the translucent pass (after the grid and edges) so it covers them.
    let proj = display_proj(app);
    let display = crate::sketch_display::profile_region(app, sketch, index, &plane, &proj);
    for [a, b, c] in display.as_ref().as_ref().map_or_else(Vec::new, |region| crate::sketch_display::fill_triangles(region, &mut 65_536)) {
        sc.trans_tri(w(a), [0.0; 3], col);
        sc.trans_tri(w(b), [0.0; 3], col);
        sc.trans_tri(w(c), [0.0; 3], col);
    }
    for lp in std::iter::once(&p.region.outer).chain(&p.region.holes) {
        for part in crate::sketch_display::boundary_parts(lp, &plane, &proj, app.viewport.pixels_per_point()) {
            for pair in part.windows(2) {
                sc.line((pair[0] + lift).to_f32(), (pair[1] + lift).to_f32(), c4(edge), 2.0, false);
            }
        }
    }
}

fn quad(sc: &mut GpuScene, q: &[Vec3; 4], fill: Color32, edge: Color32) {
    let col = c4(fill);
    for i in [0, 1, 2, 0, 2, 3] {
        if let Some(p) = q.get(i) {
            sc.trans_tri(p.to_f32(), [0.0; 3], col);
        }
    }
    for i in 0..4 {
        sc.line(q[i].to_f32(), q[(i + 1) % 4].to_f32(), c4(edge), 1.2, false);
    }
}

fn highlight_key(app: &SolveApp) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    app.session.revision.hash(&mut h);
    serde_json::to_string(&app.highlighted()).unwrap_or_default().hash(&mut h);
    format!("{:?}", app.viewport.hover).hash(&mut h);
    app.viewport.hover_feature.hash(&mut h);
    app.viewport.hover_bodies.hash(&mut h);
    app.ui.pick_bodies.hash(&mut h);
    crate::theme::is_dark().hash(&mut h);
    app.preview.replaced.hash(&mut h);
    app.origin_visible().hash(&mut h);
    app.session.visibility.hidden_origin.hash(&mut h);
    app.session.visibility.hidden_bodies.hash(&mut h);
    app.session.visibility.hidden_occurrences.hash(&mut h);
    app.cam.half_height().to_bits().hash(&mut h);
    app.cam.back().x.to_bits().hash(&mut h);
    crate::sketch_display::projection_key(&display_proj(app), app.viewport.pixels_per_point()).hash(&mut h);
    h.finish()
}

/// The highlight scene: origin widget, construction planes, selection and hover.
fn build_highlight(app: &SolveApp) -> GpuScene {
    let t = Tokens::get();
    let mut sc = GpuScene::default();
    let st = app.session.world_state();
    let mut sel = Vec::new();
    for item in app.highlighted() {
        for piece in displayed_selection(app, &item) {
            if !sel.iter().any(|x| crate::selection::same_item(x, &piece)) {
                sel.push(piece);
            }
        }
    }
    let hover = app.viewport.hover.as_ref();
    let size = origin_size(app.cam.half_height());
    // Origin planes and construction planes.
    let plane_state = |name: &str| {
        let selected = sel.iter().any(|x| matches!(x, Sel::Plane { name: n } if n == name));
        let hovered = matches!(hover, Some(Hit::Plane { name: n, .. }) if n == name);
        (selected, hovered)
    };
    let mut planes: Vec<(String, [Vec3; 4], Color32)> = origin_planes(app).into_iter().map(|(n, _, q)| (n.to_string(), q, t.origin_plane)).collect();
    planes.extend(construction_quads(app).into_iter().map(|(n, _, q)| (n, q, t.construction_plane)));
    for (name, q, base) in &planes {
        let (selected, hovered) = plane_state(name);
        let fill = if hovered {
            t.origin_plane_hover
        } else if selected {
            Color32::from_rgba_unmultiplied(t.sel_face.r(), t.sel_face.g(), t.sel_face.b(), 150)
        } else {
            *base
        };
        quad(&mut sc, q, fill, if selected { t.sel_edge } else { t.origin_plane_edge });
    }
    // Origin axes: solid near the origin, dashed beyond.
    for (name, dir) in origin_axes(app) {
        let col = match name {
            "X" => t.axis_x,
            "Y" => t.axis_y,
            _ => t.axis_z,
        };
        let on = sel.iter().any(|x| matches!(x, Sel::Axis { name: n } if n == name)) || matches!(hover, Some(Hit::Axis { name: n }) if n == name);
        let w = if on { 4.0 } else { 2.0 };
        sc.line([0.0; 3], (dir * size).to_f32(), c4(col), w, false);
        let dash = size * 0.08;
        let mut a = size + dash;
        while a < size * AXIS_LEN {
            sc.line((dir * a).to_f32(), (dir * (a + dash)).to_f32(), c4(col), w * 0.75, false);
            a += dash * 2.0;
        }
    }
    // Construction axes (dashed, the length of the origin axes either side) and points (a
    // small star), in the construction colour.
    for (name, a, b) in construction_axis_segments(app) {
        let on = sel.iter().any(|x| matches!(x, Sel::Axis { name: n } if *n == name)) || matches!(hover, Some(Hit::Axis { name: n }) if *n == name);
        let col = c4(if on { t.sel_edge } else { t.construction_geom });
        let w = if on { 3.0 } else { 1.6 };
        let len = a.dist(b);
        let dash = size * 0.08;
        let mut s = 0.0;
        while s < len {
            let e = (s + dash).min(len);
            sc.line(a.lerp(b, s / len).to_f32(), a.lerp(b, e / len).to_f32(), col, w, false);
            s += dash * 1.6;
        }
    }
    for (_, p) in construction_points(app) {
        let r = size * 0.04;
        for d in [Vec3::X, Vec3::Y, Vec3::Z] {
            sc.line((p - d * r).to_f32(), (p + d * r).to_f32(), c4(t.construction_geom), 2.0, true);
        }
    }
    // Selected things.
    for x in &sel {
        match x {
            Sel::Face { body, index, .. } => {
                // A live preview stands in for the body: its old faces would fight with it.
                if let Some(b) = st.body(body).filter(|_| app.session.body_visible(body) && !app.preview.replaced.contains(body)) {
                    face_tris(&mut sc, &b.mesh(), *index, c4(t.sel_face), false);
                    hidden_face_outline(&mut sc, &b.mesh(), *index, t.sel_edge);
                }
            }
            Sel::Body { name } => {
                if let Some(b) = st.body(name)
                    && app.session.body_visible(name)
                {
                    let m = b.mesh();
                    for f in 0..b.body.face_count() {
                        face_tris(&mut sc, &m, f, c4(t.body_selected), true);
                    }
                }
            }
            Sel::Edge { body, index, point } => {
                if let Some(b) = st.body(body)
                    && app.session.body_visible(body)
                    && let Some(e) = edge_of(&b.mesh(), *index, *point)
                {
                    edge_lines(&mut sc, e, t.sel_edge, t.sel_edge_rim, 3.0);
                }
            }
            Sel::Profile { sketch, index } => profile_fill(&mut sc, app, *sketch, *index, t.sel_profile, t.sel_profile_edge),
            // A 3D sketch curve, in whichever sketch has it.
            Sel::SketchCurve { id } if st.sketches.iter().any(|ss| ss.sketch.wire_index(id).is_some()) => {
                for ss in st.sketches.iter() {
                    if let Some(w) = ss.sketch.wire_index(id).and_then(|i| ss.sketch.wires.get(i)) {
                        for q in w.pts.windows(2) {
                            sc.line(q[0].to_f32(), q[1].to_f32(), c4(t.sel_edge), 3.5, true);
                        }
                    }
                }
            }
            Sel::SketchCurve { id } => {
                if let Some(ss) = app.session.active_sketch.and_then(|sid| st.sketch(sid))
                    && let Some(ci) = ss.sketch.curve_index(id)
                {
                    for pts in &crate::sketch_display::curve_sample(app, ss.feature, ci, &ss.plane, &display_proj(app)).parts {
                        for w in pts.windows(2) {
                            sc.line(w[0].to_f32(), w[1].to_f32(), c4(t.sel_edge), 3.5, true);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // Timeline modifiers light only their resulting faces; browser rows light whole bodies.
    let mut hovered = app.viewport.hover_feature.map(|id| feature_geometry(app, id)).unwrap_or_default();
    hovered.extend(app.viewport.hover_bodies.iter().map(|name| Sel::Body { name: name.clone() }));
    for b in st.bodies.iter().filter(|b| app.session.body_visible(&b.name) && !app.preview.replaced.contains(&b.name)) {
        let whole = hovered.iter().any(|x| matches!(x, Sel::Body { name } if name == &b.name));
        let faces: Vec<_> = if whole {
            (0..b.body.face_count()).collect()
        } else {
            hovered
                .iter()
                .filter_map(|x| match x {
                    Sel::Face { body, index, .. } if body == &b.name => Some(*index),
                    _ => None,
                })
                .collect()
        };
        if faces.is_empty() {
            continue;
        }
        let k = t.hover_face_lift;
        let c = body_rgb(app, &b.name);
        let m = b.mesh();
        for &f in &faces {
            if !sel.iter().any(|x| {
                matches!(x, Sel::Face { body, index, .. } if body == &b.name && *index == f) || matches!(x, Sel::Body { name } if name == &b.name)
            }) {
                face_tris(&mut sc, &m, f, [c.0.saturating_add(k), c.1.saturating_add(k), c.2.saturating_add(k), 255], true);
            }
        }
        for (ei, e) in m.edges.iter().enumerate() {
            if m.edge_pickable(ei) && (whole || m.edge_faces.get(ei).is_some_and(|fs| fs.iter().any(|f| faces.contains(&(*f as usize))))) {
                edge_lines(&mut sc, e, t.hover_edge, t.hover_edge_halo, 1.5);
            }
        }
    }
    // Hover (pre-highlight).
    match hover {
        // Body selection: the whole body lights up.
        Some(Hit::Face { body, .. }) if app.ui.pick_bodies && app.dialog.is_none() && app.tool.is_none() => {
            if let Some(b) = st.body(body) {
                let k = t.hover_face_lift;
                let c = body_rgb(app, body);
                let m = b.mesh();
                for f in 0..b.body.face_count() {
                    face_tris(&mut sc, &m, f, [c.0.saturating_add(k), c.1.saturating_add(k), c.2.saturating_add(k), 255], true);
                }
            }
        }
        Some(Hit::Face { body, index, .. })
            if !app.preview.replaced.contains(body)
                && !sel.iter().any(|x| matches!(x, Sel::Face { body: b, index: i, .. } if b == body && i == index)) =>
        {
            if let Some(b) = st.body(body) {
                let k = t.hover_face_lift;
                let c = body_rgb(app, body);
                let m = b.mesh();
                let patch = m.surface_patch(*index);
                for &f in &patch {
                    face_tris(&mut sc, &m, f, [c.0.saturating_add(k), c.1.saturating_add(k), c.2.saturating_add(k), 255], true);
                }
                for (e, _) in m.edge_faces.iter().enumerate().filter(|(_, fs)| fs.iter().any(|f| patch.contains(&(*f as usize)))) {
                    if let Some(p) = m.edges.get(e)
                        && m.edge_pickable(e)
                    {
                        edge_lines(&mut sc, p, t.hover_edge, t.hover_edge_halo, 1.5);
                    }
                }
            }
        }
        Some(Hit::Edge { body, index, mid }) => {
            if let Some(b) = st.body(body) {
                let m = b.mesh();
                for i in m.circular_rim(*index) {
                    let selected = sel.iter().any(|x| matches!(x, Sel::Edge { body: b, index: j, .. } if b == body && *j == i));
                    if !selected && let Some(e) = if i == *index { edge_of(&m, i, *mid) } else { m.edges.get(i) } {
                        edge_lines(&mut sc, e, t.hover_edge, t.hover_edge_halo, 2.0);
                    }
                }
            }
        }
        Some(Hit::Profile { sketch, index })
            if !sel.iter().any(|x| matches!(x, Sel::Profile { sketch: s2, index: i2 } if s2 == sketch && i2 == index)) =>
        {
            profile_fill(&mut sc, app, *sketch, *index, t.hover_profile, t.hover_profile_edge);
        }
        _ => {}
    }
    sc
}

/// A body's display colour (appearance, material or imported colour), if it has one.
fn body_color(doc: &solvecraft_engine::doc::Document, b: &solvecraft_engine::doc::ModelBody) -> Option<Color32> {
    let to8 = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    doc.body_color(b).map(|[r, g, b]| Color32::from_rgb(to8(r), to8(g), to8(b)))
}

/// A body's display colour (else the theme's) as RGB.
fn body_rgb(app: &SolveApp, name: &str) -> (u8, u8, u8) {
    let st = app.session.world_state();
    let c = st.body(name).and_then(|b| body_color(&app.session.doc, b)).unwrap_or(Tokens::get().body);
    (c.r(), c.g(), c.b())
}

pub(crate) fn polyline_mid(pts: &[Vec3]) -> Vec3 {
    let len: f64 = pts.windows(2).map(|w| w[0].dist(w[1])).sum();
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let l = w[0].dist(w[1]);
        if acc + l >= len * 0.5 && l > 0.0 {
            return w[0].lerp(w[1], (len * 0.5 - acc) / l);
        }
        acc += l;
    }
    pts.first().copied().unwrap_or_default()
}

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let l2 = ab.length_sq();
    let t = if l2 > 0.0 { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// Point on the active sketch plane under the cursor (sketch coordinates), snapped to sketch
/// points within a few pixels and otherwise to a fine grid. Returns the snapped point and the
/// id of the sketch point it snapped to.
pub fn sketch_point_at(app: &SolveApp, proj: &Proj, pos: Pos2) -> Option<(Vec2, Option<String>)> {
    let sid = app.session.active_sketch?;
    let st = app.session.world_state();
    let ss = st.sketch(sid)?;
    for h in pick(app, proj, pos) {
        if let Hit::SketchPoint { id, at, .. } = h {
            return Some((at, Some(id)));
        }
    }
    let (o, d) = proj.ray(pos);
    let w = ss.plane.intersect_ray(o, d)?;
    let lp = ss.plane.to_local(w);
    if crate::drag_snap::suppressed() {
        return Some((lp, None));
    }
    let (minor, _) = grid_step(app.cam.half_height());
    let step = minor / 10.0;
    if let Some(p) = crate::sketch_tools::refine_snap(app, proj, &ss.sketch, &ss.plane, lp) {
        return Some(p);
    }
    // With Snap off (Sketch Palette) the point is where the cursor is.
    if ss.sketch.view.no_snap {
        return Some((lp, None));
    }
    Some((Vec2::new((lp.x / step).round() * step, (lp.y / step).round() * step), None))
}

pub fn delete_selection(app: &mut SolveApp) {
    crate::delete::delete_selection(app);
}

/// Match egui's wheel units and modifier routing, without its temporal smoothing.
fn immediate_wheel_delta(input: &egui::InputState, options: egui::InputOptions, page_height: f32) -> f32 {
    input
        .events
        .iter()
        .filter_map(|event| {
            let egui::Event::MouseWheel { unit, delta, modifiers, phase: egui::TouchPhase::Move } = event else { return None };
            if modifiers.matches_any(options.zoom_modifier) {
                return None;
            }
            let delta = match unit {
                egui::MouseWheelUnit::Point => *delta,
                egui::MouseWheelUnit::Line => *delta * options.line_scroll_speed,
                egui::MouseWheelUnit::Page => *delta * page_height,
            };
            let horizontal = modifiers.matches_any(options.horizontal_scroll_modifier);
            let vertical = modifiers.matches_any(options.vertical_scroll_modifier);
            Some(if horizontal && !vertical {
                0.0
            } else if vertical && !horizontal {
                delta.x + delta.y
            } else {
                delta.y
            })
        })
        .sum()
}

pub fn show(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    let rect = ui.max_rect();
    app.viewport.rect = Some(rect);
    refresh_layout(app, ui.ctx().pixels_per_point());
    let resp = ui.interact(rect, ui.id().with("viewport"), Sense::click_and_drag());
    let (hover, scroll, mods, middle, secondary_down, primary_down, delta) = ui.input(|i| {
        (
            i.pointer.hover_pos(),
            i.smooth_scroll_delta.y,
            i.modifiers,
            i.pointer.middle_down(),
            i.pointer.secondary_down(),
            i.pointer.primary_down(),
            i.pointer.delta(),
        )
    });
    // The right button belongs to the marking menu while it is open.
    let secondary_down = secondary_down && app.viewport.context_menu.is_none();
    // The axis view handles its own clicks; the model underneath must not see them.
    let axes = axis_rect(app, rect);
    // Floating windows and menus own input over the model, including wheel navigation.
    let inside = resp.contains_pointer() && hover.is_some_and(|p| !axes.contains(p));
    app.viewport.mouse = hover.filter(|_| inside);
    // Ctrl or Alt held: drawing and dragging don't snap.
    crate::drag_snap::set_suppressed(mods.ctrl || mods.alt || mods.command);

    // ---- navigation ----
    if !app.preferences.smooth_wheel_zoom {
        app.viewport.wheel_zoom_raw_until_idle = true;
    } else if !ui.input(|i| i.is_scrolling()) {
        app.viewport.wheel_zoom_raw_until_idle = false;
    }
    let scroll = if app.viewport.wheel_zoom_raw_until_idle {
        let options = ui.ctx().options(|o| o.input_options);
        let page_height = ui.ctx().viewport_rect().height();
        ui.input(|i| immediate_wheel_delta(i, options, page_height))
    } else {
        scroll
    };
    let h = rect.height().max(1.0) as f64;
    if inside && scroll != 0.0 {
        app.viewport.orbit_snap = None;
        app.viewport.orbit_free = None;
        let f = (-scroll as f64 * 0.0015 * crate::prefs::zoom_sign(app)).exp();
        // Preferences › Navigation › Zoom to the cursor (off: toward the view's centre).
        let anchor = hover.filter(|_| app.preferences.zoom_to_cursor).map(|p| {
            let proj = projection(app, rect);
            // Toward the model's surface under the cursor, else the target's depth.
            if let Some(q) = pick_cached(app, &proj, p).iter().find_map(|h| if let Hit::Face { point, .. } = h { Some(*point) } else { None }) {
                return q;
            }
            let (o, d) = proj.ray(p);
            let n = app.cam.back();
            let den = d.dot(n);
            if den.abs() > 1e-9 { o + d * ((app.cam.target - o).dot(n) / den) } else { app.cam.target }
        });
        app.cancel_view_animation();
        app.cam.zoom_at(f, anchor);
    }
    let dragging = resp.dragged() || (inside && (middle || secondary_down));
    let mode = if dragging && app.viewport.axis_drag.is_none() {
        if middle || secondary_down {
            crate::prefs::nav_mode(app, middle, secondary_down, mods)
        } else if primary_down {
            app.viewport.nav
        } else {
            None
        }
    } else {
        None
    };
    if mode == Some(NavMode::Orbit) {
        let press = ui.input(|i| i.pointer.press_origin());
        app.cancel_view_animation();
        // Shift can be required to enter orbit (Fusion/Inventor middle drag), rather
        // than an additional request for snapping. Only an extra modifier snaps.
        let without_shift = egui::Modifiers { shift: false, ..mods };
        let snap = mods.shift && (!middle || crate::prefs::nav_mode(app, middle, secondary_down, without_shift) == Some(NavMode::Orbit));
        orbit_drag(app, rect, f64::from(delta.x), f64::from(delta.y), press, snap, true);
    } else {
        if matches!(mode, Some(NavMode::Pan | NavMode::Zoom)) {
            app.viewport.orbit_snap = None;
        }
        if app.viewport.axis_drag.is_none() {
            app.viewport.orbit_free = None;
        }
        if delta != egui::Vec2::ZERO {
            if mode.is_some() {
                app.cancel_view_animation();
            }
            match mode {
                Some(NavMode::Pan) => app.cam.pan(f64::from(delta.x), f64::from(delta.y), h),
                Some(NavMode::Zoom) => app.cam.zoom_at((f64::from(delta.y) * 0.01).exp(), None),
                _ => {}
            }
        }
    }
    if !dragging {
        app.viewport.orbit_pivot = None;
    }
    if app.viewport.spin {
        // A click on the navigation bar (its own button) is not a stop.
        let bar = app
            .viewport
            .layout
            .as_ref()
            .map(|l| l.navigation)
            .unwrap_or_else(|| Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 22.0), vec2(320.0, 30.0)));
        let (pressed, key, dt) = ui.input(|i| {
            (
                i.pointer.any_pressed() && inside && !i.pointer.press_origin().is_some_and(|p| bar.contains(p)),
                i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })),
                i.stable_dt,
            )
        });
        if pressed || key || dragging {
            app.viewport.spin = false;
        } else {
            app.cam.orbit(f64::from(dt.min(0.1)) * 30.0, 0.0);
            ui.ctx().request_repaint();
        }
    }
    if !app.cam.is_valid() {
        app.viewport.orbit_free = None;
        app.viewport.orbit_snap = None;
        app.cam = Camera::default();
        app.fit_view();
    }

    step_orbit_snap(app, rect, ui.input(|i| f64::from(i.stable_dt)));
    if app.viewport.orbit_snap.is_some() {
        ui.ctx().request_repaint();
    }
    app.viewport.physical_scale = ui.ctx().pixels_per_point();
    let proj = projection(app, rect);
    if let Ok(mut cache) = app.viewport.display.lock() {
        cache.prepare(app.session.world_state(), &proj, app.viewport.pixels_per_point());
    }
    // Suppress hover for the whole navigation gesture, including frames between pointer
    // events. Testing delta made the highlight reappear whenever a held drag paused.
    // Resolve it before rendering so the GPU scene and screen overlays use the same hit.
    let navigating = scroll != 0.0
        || middle
        || secondary_down
        || (app.viewport.nav.is_some() && primary_down)
        || app.viewport.axis_drag.is_some()
        || app.viewport.spin
        || app.viewport.orbit_snap.is_some()
        || app.cam_anim.is_some();
    let hits = hover.filter(|_| inside && app.viewport.boxsel.is_none() && !navigating).map(|p| pick_cached(app, &proj, p)).unwrap_or_default();
    let cand = if app.tool.is_some() { hits.first().cloned().map(|h| (h, None)) } else { candidate(app, &hits).map(|(h, s)| (h, Some(s))) };
    app.viewport.hover = cand.as_ref().map(|c| c.0.clone());
    let painter = ui.painter_at(rect);
    // Background gradient.
    let (top, bot) = (t.viewport_top, t.viewport_bottom);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bot);
    mesh.colored_vertex(rect.left_bottom(), bot);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));

    // ---- model ----
    let key = scene_key(app);
    let t0 = crate::now_ms();
    if let Some(_g) = app.viewport.gpu {
        if key != app.viewport.key {
            let sc = build_scene(app);
            if let Ok(mut slot) = app.viewport.slot.lock() {
                *slot = Some(sc);
            }
            app.viewport.key = key;
            app.viewport.build_ms = crate::now_ms() - t0;
        }
        let sk = sketch_scene_key(app, &proj);
        if sk != app.viewport.sketch_key {
            let scene = build_sketch_scene(app, &proj);
            if let Ok(mut slot) = app.viewport.sketch_slot.lock() {
                *slot = Some(scene);
            }
            app.viewport.sketch_key = sk;
        }
        let hk = highlight_key(app);
        if hk != app.viewport.hl_key {
            let hl = build_highlight(app);
            if let Ok(mut slot) = app.viewport.hl_slot.lock() {
                *slot = Some(hl);
            }
            app.viewport.hl_key = hk;
        }
        let ppp = ui.ctx().pixels_per_point();
        let cb = ViewportCallback {
            key: app.viewport.key,
            slot: app.viewport.slot.clone(),
            sketch_key: app.viewport.sketch_key,
            sketch_slot: app.viewport.sketch_slot.clone(),
            hl_key: app.viewport.hl_key,
            hl_slot: app.viewport.hl_slot.clone(),
            pv_key: app.preview.key,
            pv_slot: app.preview.slot.clone(),
            clip: section_clip(app),
            cap: t.section_cap.to_normalized_gamma_f32(),
            view_proj: proj.vp.to_f32(),
            back: proj.cam.back().to_f32(),
            size_px: [rect.width() * ppp, rect.height() * ppp],
            images: crate::ref_images::gpu_images(app),
            analysis: analysis_uniform(app),
            access: crate::access::map(app),
            outline: matches!(app.ui.visual_style, 0 | 3),
            eye: {
                let e = proj.cam.eye().to_f32();
                [e[0], e[1], e[2], if proj.cam.fov > 0.0 { 1.0 } else { 0.0 }]
            },
        };
        painter.add(egui_wgpu::Callback::new_paint_callback(rect, cb));
    } else {
        cpu_render(app, ui.ctx(), &painter, rect, &proj);
    }

    // ---- interaction ----
    let add = mods.shift || mods.command || mods.ctrl;
    if let Some(p) = hover.filter(|_| inside) {
        if app.tool.is_some() {
            crate::tools::on_hover(app, &proj, p);
        }
        if resp.clicked() {
            if app.tool.is_some() {
                crate::tools::on_click(app, &proj, p);
            } else if app.dialog.as_ref().is_some_and(|d| d.wants_picks()) {
                if let Some(sel) = cand.and_then(|c| c.1)
                    && let Some(mut d) = app.dialog.take()
                {
                    d.pick(&app.session, sel);
                    app.dialog = Some(d);
                }
            } else {
                // Dimensions and constraint glyphs sit on top of the geometry.
                let off = |k: &str| app.ui.pick_off.iter().any(|x| x == k);
                let sketching = app.session.active_sketch.is_some();
                // A dimension's text wins; its lines only where no sketch point or curve is (a
                // diameter's line runs through the centre point).
                let on_geometry = matches!(cand.as_ref().map(|c| &c.0), Some(Hit::SketchPoint { .. } | Hit::SketchCurve { .. }));
                let on_dim = crate::dim_view::hit_text(p)
                    .or_else(|| if on_geometry { None } else { crate::dim_view::hit(p) })
                    .filter(|_| sketching && !off("dimensions"));
                let on_glyph = crate::sketch_tools::glyph_at(p).filter(|_| sketching && !off("constraints"));
                if !crate::sketch3d::click(app, &proj, p) {
                    match on_dim.or(on_glyph) {
                        Some(id) => select(app, Some(Sel::SketchConstraint { id }), add),
                        None => select(app, cand.and_then(|c| c.1), add),
                    }
                }
            }
        }
        // Double-click a dimension: edit its value in place.
        let dim_edit = resp.double_clicked() && app.tool.is_none() && app.dialog.is_none() && crate::dim_view::double_click(app, p);
        // Double-click a sketch curve: select the chain joined to it end to end.
        if resp.double_clicked()
            && !dim_edit
            && app.tool.is_none()
            && app.dialog.is_none()
            && let Some(Hit::SketchCurve { sketch, id, .. }) = app.viewport.hover.clone()
            && app.session.active_sketch == Some(sketch)
            && let Ok(v) = app.session.execute("sketch.chain", &json!({ "curve": id }))
        {
            let items: Vec<Sel> =
                v["curves"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).map(|id| Sel::SketchCurve { id: id.into() }).collect();
            let _ = app.run("select.set", json!({ "items": items, "add": add }));
        }
        // Double-click an edge: select the edges that continue it smoothly (the loop).
        if resp.double_clicked()
            && !dim_edit
            && app.tool.is_none()
            && app.dialog.is_none()
            && let Some(Hit::Edge { body, index, .. }) = app.viewport.hover.clone()
            && let Some(b) = app.session.world_state().body(&body).cloned()
        {
            let m = b.mesh();
            let items: Vec<Sel> = m
                .tangent_chain(index, 2f64.to_radians())
                .into_iter()
                .filter_map(|i| m.edges.get(i).map(|e| Sel::Edge { body: body.clone(), index: i, point: polyline_mid(e) }))
                .collect();
            let _ = app.run("select.set", json!({ "items": items, "add": add }));
        }
        if resp.secondary_clicked() && delta == egui::Vec2::ZERO {
            if app.tool.is_some() {
                crate::tools::finish(app);
            } else if app.dialog.is_none() {
                crate::context_menu::open(app, p);
            }
        }
    }
    // Dragging a sketch point of the active sketch moves it; the solver keeps the constraints.
    if app.tool.is_none() && app.dialog.is_none() && app.viewport.nav.is_none() && app.session.active_sketch.is_some() {
        let on_plane = |app: &SolveApp, p: Pos2| {
            let ss = app.session.active_sketch.and_then(|sid| app.session.world_state().sketch(sid).cloned())?;
            let (o, d) = proj.ray(p);
            ss.plane.intersect_ray(o, d).map(|w| ss.plane.to_local(w))
        };
        // The 3D triad, then a dimension's text, follow a drag that starts on them.
        if resp.drag_started_by(egui::PointerButton::Primary)
            && let Some(a) = ui.input(|i| i.pointer.press_origin())
            && !crate::sketch3d::drag_start(app, &proj, a)
            && let Some(q) = on_plane(app, a)
        {
            crate::dim_view::drag_start(app, a, q);
        }
        if crate::sketch3d::dragging()
            && (resp.dragged_by(egui::PointerButton::Primary) || resp.drag_stopped())
            && let Some(p) = hover
        {
            crate::sketch3d::drag_to(app, &proj, p);
        }
        if crate::dim_view::dragging()
            && (resp.dragged_by(egui::PointerButton::Primary) || resp.drag_stopped())
            && let Some(q) = hover.and_then(|p| on_plane(app, p))
        {
            crate::dim_view::drag_to(app, q);
        }
        // A point or a curve of the sketch being edited follows a drag that starts on it.
        if resp.drag_started_by(egui::PointerButton::Primary)
            && !crate::dim_view::dragging()
            && !crate::sketch3d::dragging()
            && let Some(a) = ui.input(|i| i.pointer.press_origin())
        {
            let active = app.session.active_sketch;
            match pick(app, &proj, a).into_iter().next() {
                Some(Hit::SketchPoint { id, .. }) => app.viewport.point_drag = Some((id, app.session.undo.len(), None)),
                Some(Hit::SketchCurve { sketch, id, .. }) if Some(sketch) == active => {
                    app.viewport.point_drag = on_plane(app, a).map(|q| (id, app.session.undo.len(), Some(q)));
                }
                _ => {}
            }
        }
        if let Some((id, depth, grab)) = app.viewport.point_drag.clone()
            && resp.dragged_by(egui::PointerButton::Primary)
            && let Some(mut q) = hover.and_then(|p| on_plane(app, p))
        {
            let from = grab.unwrap_or(q);
            // Snap the dragged point (or the line's ends) to other geometry, like drawing.
            let snap = {
                let st = app.session.world_state();
                app.session.active_sketch.and_then(|s| st.sketch(s)).and_then(|ss| {
                    let sk = &ss.sketch;
                    let (own, exclude) = crate::drag_snap::moving_points(sk, &id);
                    if own.iter().any(|&p| sk.point_locked(p)) {
                        return crate::drag_snap::find(sk, &[], &[], 0.0);
                    }
                    let a = proj.to_screen(ss.plane.to_world(q))?;
                    let b = proj.to_screen(ss.plane.to_world(q + Vec2::X))?;
                    let mm_px = 1.0 / (a.distance(b) as f64).max(1e-9);
                    let d = q - from;
                    let moving: Vec<(usize, Vec2)> =
                        own.iter().filter_map(|&p| if grab.is_some() { sk.point(p).map(|x| (p, x + d)) } else { Some((p, q)) }).collect();
                    crate::drag_snap::find(sk, &moving, &exclude, mm_px)
                })
            };
            if let Some(s) = snap {
                q += s.offset;
            }
            if app.session.execute("sketch.drag", &json!({"entity": id, "from": [from.x, from.y], "to": [q.x, q.y]})).is_ok() {
                app.session.undo.truncate(depth + 1);
            }
            if grab.is_some() {
                app.viewport.point_drag = Some((id, depth, Some(q)));
            }
        }
        if resp.drag_stopped() {
            if let Some((_, depth, _)) = app.viewport.point_drag.clone() {
                // Let go on a point, midpoint or curve: the matching constraint, in the drag's undo step.
                crate::drag_snap::release(app);
                app.session.undo.truncate(depth + 1);
            }
            crate::drag_snap::clear();
            app.viewport.point_drag = None;
            crate::dim_view::drag_end();
            crate::sketch3d::drag_end();
        }
    }
    // Box selection: a primary drag on the model when no navigation mode is on.
    if app.tool.is_none()
        && app.viewport.nav.is_none()
        && app.viewport.point_drag.is_none()
        && !crate::dim_view::dragging()
        && !crate::sketch3d::dragging()
    {
        let origin = ui.input(|i| i.pointer.press_origin());
        if resp.dragged_by(egui::PointerButton::Primary)
            && let (Some(a), Some(b)) = (origin, hover)
            && rect.contains(a)
            && !axes.contains(a)
        {
            app.viewport.boxsel = Some(BoxSel::from_drag(a, b));
        }
        if resp.drag_stopped()
            && let Some(bx) = app.viewport.boxsel.take()
        {
            box_select(app, &proj, bx, add);
        }
    } else {
        app.viewport.boxsel = None;
    }
    crate::ref_images::show(app, ui, &painter, &proj);
    crate::dialogs_assembly::origins(app, &painter, &proj);
    overlays(app, &painter, &proj);
    crate::dim_view::show(app, ui, &painter, &proj);
    crate::sketch3d::show(app, &painter, &proj);
    crate::inference::show(app, &painter, &proj);
    crate::drag_snap::show(app, &painter, &proj);
    hover_highlight(app, &painter, &proj);
    points_2d(app, &painter, &proj);
    if let Some((plane, sketch)) = crate::sketch_attachment_ui::lost_guide(app)
        && let Some((low, high)) = sketch.bounds()
        && let Some(at) = proj.to_screen(plane.to_world(solvecraft_engine::geom::Vec2::new((low.x + high.x) * 0.5, high.y)))
    {
        painter.text(at - vec2(0.0, 12.0), Align2::CENTER_BOTTOM, "Saved sketch plane — attachment lost", FontId::proportional(12.0), t.warning);
    }
    if let Some(bx) = app.viewport.boxsel {
        let col = if bx.crossing { t.box_crossing } else { t.box_window };
        painter.rect_filled(bx.rect, 0.0, col.gamma_multiply(0.12));
        if bx.crossing {
            let c = [bx.rect.left_top(), bx.rect.right_top(), bx.rect.right_bottom(), bx.rect.left_bottom()];
            for i in 0..4 {
                painter.add(Shape::dashed_line(&[c[i], c[(i + 1) % 4]], Stroke::new(1.2, col), 6.0, 4.0));
            }
        } else {
            painter.rect_stroke(bx.rect, 0.0, Stroke::new(1.2, col), egui::StrokeKind::Inside);
        }
    }
    if let Some(tl) = app.tool.as_ref() {
        crate::tools::preview(app, tl, &painter, &proj);
    }
    crate::canvas::show(app, ui, &painter, &proj);
    crate::sketch_tools::show(app, ui, &painter, &proj);
    crate::sketch_dims::show(app, ui, &proj);
    axis_view(app, ui, rect);
    nav_bar(app, ui, rect);
    // Status chip.
    if let Some((msg, at, err)) = app.status.clone() {
        if crate::now_ms() - at < 6000.0 {
            let pos = pos2(rect.left() + 12.0, rect.bottom() - 46.0);
            let galley = painter.layout_no_wrap(msg, FontId::proportional(12.5), if err { t.error } else { t.text });
            let r = Rect::from_min_size(pos, galley.size() + vec2(16.0, 8.0));
            painter.rect_filled(r, 4.0, t.overlay);
            painter.galley(pos + vec2(8.0, 4.0), galley, t.text);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        } else {
            app.status = None;
        }
    }
}

/// A click without a command: select the candidate (replacing the selection, or toggling it
/// in with Shift/Ctrl); a click on nothing clears.
fn select(app: &mut SolveApp, sel: Option<Sel>, add: bool) {
    match sel {
        Some(x) => {
            let items = grouped_selection(&app.session, &x);
            if add && items.iter().all(|x| app.session.selection.iter().any(|y| crate::selection::same_item(x, y))) {
                let rest: Vec<Sel> =
                    app.session.selection.iter().filter(|y| !items.iter().any(|x| crate::selection::same_item(x, y))).cloned().collect();
                let _ = app.run("select.set", json!({ "items": rest }));
            } else {
                let _ = app.run("select.set", json!({"items": items, "add": add}));
            }
        }
        None if !add => {
            let _ = app.run("select.clear", json!({}));
        }
        None => {}
    }
}

/// Expand a visible rim or surface into the kernel pieces a command needs. The original
/// face pick keeps its point (used by tools such as Hole and construction geometry).
pub(crate) fn grouped_selection(s: &solvecraft_engine::Session, sel: &Sel) -> Vec<Sel> {
    let st = s.world_state();
    match sel {
        Sel::Edge { body, index, .. } => {
            if let Some(b) = st.body(body) {
                let m = b.mesh();
                return m
                    .circular_rim(*index)
                    .into_iter()
                    .filter_map(|i| m.edges.get(i).map(|e| Sel::Edge { body: body.clone(), index: i, point: polyline_mid(e) }))
                    .collect();
            }
        }
        Sel::Face { body, index, .. } => {
            if let Some(b) = st.body(body) {
                let m = b.mesh();
                let mut patch = m.surface_patch(*index);
                patch.sort_by_key(|i| usize::from(i != index));
                return patch
                    .into_iter()
                    .filter_map(|i| {
                        if i == *index {
                            return Some(sel.clone());
                        }
                        m.triangles.iter().zip(&m.tri_face).find_map(|(t, f)| {
                            if *f as usize != i {
                                return None;
                            }
                            let [a, b, c] = m.tri(t)?;
                            Some(Sel::Face { body: body.clone(), index: i, point: (a + b + c) / 3.0 })
                        })
                    })
                    .collect();
            }
        }
        _ => {}
    }
    vec![sel.clone()]
}

/// Measure keeps one entry per visible item, rather than using its two slots for kernel halves.
pub(crate) fn measurement_selection(s: &solvecraft_engine::Session, sel: &Sel) -> Sel {
    let pieces = grouped_selection(s, sel);
    let mut representative = sel.clone();
    match &mut representative {
        Sel::Edge { index, .. } => {
            if let Some(Sel::Edge { index: i, .. }) = pieces.first() {
                *index = *i;
            }
        }
        Sel::Face { index, .. } => {
            if let Some(i) = pieces
                .iter()
                .filter_map(|p| match p {
                    Sel::Face { index, .. } => Some(*index),
                    _ => None,
                })
                .min()
            {
                *index = i;
            }
        }
        _ => {}
    }
    representative
}

/// What a box selects: sketch entities while sketching, otherwise what the open dialog's
/// active input takes (edges, faces or bodies), otherwise bodies.
fn box_select(app: &mut SolveApp, proj: &Proj, bx: BoxSel, add: bool) {
    use crate::selection::{BODIES, EDGES, FACES, PROFILES};
    let st = app.session.world_state();
    let mut out: Vec<Sel> = Vec::new();
    let to2 = |pts: &[Vec3]| -> Vec<Pos2> { pts.iter().filter_map(|p| proj.to_screen(*p)).collect() };
    if app.dialog.is_none()
        && let Some(ss) = app.session.active_sketch.and_then(|sid| st.sketch(sid))
    {
        // What the selection filter lets through (window: wholly inside; crossing: touching).
        let off = |k: &str| app.ui.pick_off.iter().any(|x| x == k);
        for (i, c) in ss.sketch.curves.iter().enumerate().filter(|_| !off("sketch") && !off("sketch_curves")) {
            if c.link.is_some() && ss.sketch.view.hide_projected {
                continue;
            }
            let sampled = if bx.crossing {
                crate::sketch_display::curve_sample(app, ss.feature, i, &ss.plane, proj)
            } else {
                std::sync::Arc::new(crate::sketch_display::curve_unclipped(&ss.sketch, i, &ss.plane, proj, app.viewport.pixels_per_point()))
            };
            let parts: Vec<Vec<Pos2>> = sampled.parts.iter().map(|part| to2(part)).collect();
            let takes = if bx.crossing {
                parts.iter().any(|part| bx.takes(part))
            } else {
                sampled.quality == solvecraft_engine::render::SamplingQuality::Complete
                    && !parts.is_empty()
                    && parts.iter().all(|part| bx.takes(part))
            };
            if takes {
                out.push(Sel::SketchCurve { id: c.id.clone() });
            }
        }
        for (i, p) in ss.sketch.points.iter().enumerate().filter(|_| !off("sketch") && !off("sketch_points")) {
            if i != 0 && solvecraft_engine::view::sketch_point_visible(&ss.sketch, i) && bx.takes(&to2(&[ss.plane.to_world(p.pos)])) {
                out.push(Sel::SketchPoint { id: p.id.clone() });
            }
        }
        if !off("dimensions") {
            for (id, r) in crate::dim_view::text_rects() {
                if bx.takes(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]) {
                    out.push(Sel::SketchConstraint { id });
                }
            }
        }
        if !off("constraints") {
            for (id, c) in crate::sketch_tools::glyph_positions() {
                if bx.takes(&[c]) {
                    out.push(Sel::SketchConstraint { id });
                }
            }
        }
    } else {
        let accept = app.dialog.as_ref().and_then(|d| d.active_input()).map(|i| i.accept).unwrap_or(BODIES);
        for b in &st.bodies {
            if !app.session.body_visible(&b.name) {
                continue;
            }
            let m = b.mesh();
            if accept & EDGES != 0 {
                for (ei, e) in m.edges.iter().enumerate() {
                    let rim = m.circular_rim(ei);
                    let takes = if bx.crossing { bx.takes(&to2(e)) } else { rim.iter().all(|&i| m.edges.get(i).is_some_and(|p| bx.takes(&to2(p)))) };
                    if m.edge_pickable(ei) && takes {
                        out.push(Sel::Edge { body: b.name.clone(), index: ei, point: polyline_mid(e) });
                    }
                }
            } else if accept & FACES != 0 {
                for f in 0..b.body.face_count() {
                    let mut pts: Vec<Vec3> = Vec::new();
                    for e in m.face_edges(u32::try_from(f).unwrap_or(u32::MAX)) {
                        pts.extend(m.edges.get(e).into_iter().flatten().copied());
                    }
                    let point = m
                        .triangles
                        .iter()
                        .zip(&m.tri_face)
                        .find(|(_, tf)| **tf as usize == f)
                        .and_then(|(t, _)| m.tri(t))
                        .map(|[a, bb, c]| (a + bb + c) / 3.0);
                    let takes = bx.takes(&to2(&pts))
                        && (bx.crossing
                            || m.surface_patch(f).iter().all(|&face| {
                                m.face_edges(u32::try_from(face).unwrap_or(u32::MAX))
                                    .iter()
                                    .all(|&e| m.edges.get(e).is_some_and(|p| bx.takes(&to2(p))))
                            }));
                    if let Some(point) = point
                        && takes
                    {
                        out.push(Sel::Face { body: b.name.clone(), index: f, point });
                    }
                }
            } else if accept & BODIES != 0 {
                let crossing_hit = m.edges.iter().any(|e| bx.takes(&to2(e)));
                let all_in = m.edges.iter().all(|e| bx.takes(&to2(e)));
                if (bx.crossing && crossing_hit) || (!bx.crossing && all_in && !m.edges.is_empty()) {
                    out.push(Sel::Body { name: b.name.clone() });
                }
            } else if accept & PROFILES != 0 {
                for sid in visible_sketches(app) {
                    let Some(ss) = st.sketch(sid) else { continue };
                    for (pi, p) in ss.profiles.iter().enumerate() {
                        let parts = crate::sketch_display::boundary_parts(&p.region.outer, &ss.plane, proj, app.viewport.pixels_per_point());
                        let takes = if bx.crossing {
                            parts.iter().any(|part| bx.takes(&to2(part)))
                        } else {
                            crate::sketch_display::profile_region(app, sid, pi, &ss.plane, proj).as_ref().as_ref().is_some_and(|display| {
                                let points: Vec<_> = display.outer.segs.iter().map(|segment| ss.plane.to_world(segment.start())).collect();
                                bx.takes(&to2(&points))
                            })
                        };
                        if takes {
                            out.push(Sel::Profile { sketch: sid, index: pi });
                        }
                    }
                }
                break;
            }
        }
    }
    let mut grouped = Vec::new();
    for item in out {
        for piece in grouped_selection(&app.session, &item) {
            if !grouped.iter().any(|x| crate::selection::same_item(x, &piece)) {
                grouped.push(piece);
            }
        }
    }
    let out = grouped;
    if let Some(mut d) = app.dialog.take() {
        d.take_box(&app.session, out, add);
        app.dialog = Some(d);
        return;
    }
    let items = serde_json::to_value(&out).unwrap_or_default();
    let _ = app.run("select.set", json!({"items": items, "add": add}));
}

/// Screen-space dots: the origin point, and hovered or selected body vertices.
fn points_2d(app: &SolveApp, painter: &egui::Painter, proj: &Proj) {
    let t = Tokens::get();
    if app.origin_visible()
        && !app.session.visibility.hidden_origin.iter().any(|h| h == "O")
        && let Some(o) = proj.to_screen(Vec3::ZERO)
    {
        painter.circle(o, 4.5, t.origin_point, Stroke::new(1.0, Color32::from_gray(110)));
    }
    for x in app.highlighted() {
        if let Sel::Vertex { point, .. } = x
            && let Some(p) = proj.to_screen(point)
        {
            painter.circle(p, 4.0, t.sel_vertex, Stroke::new(1.5, t.sel_edge));
        }
    }
    if let Some(Hit::Vertex { point, .. }) = &app.viewport.hover
        && let Some(p) = proj.to_screen(*point)
    {
        painter.circle(p, 4.5, t.hover_edge_halo, Stroke::new(1.5, t.hover_edge));
    }
}

/// Hover feedback for the active sketch's points and curves (drawn on top of everything).
fn hover_highlight(app: &SolveApp, painter: &egui::Painter, proj: &Proj) {
    let col = Tokens::get().hover_profile_edge;
    let st = app.session.world_state();
    match &app.viewport.hover {
        Some(Hit::SketchCurve { sketch, id, .. }) => {
            if let Some(ss) = st.sketch(*sketch)
                && let Some(ci) = ss.sketch.curve_index(id)
            {
                for part in &crate::sketch_display::curve_sample(app, *sketch, ci, &ss.plane, proj).parts {
                    let pts: Vec<Pos2> = part.iter().filter_map(|q| proj.to_screen(*q)).collect();
                    painter.add(Shape::line(pts, Stroke::new(3.0, col)));
                }
            }
        }
        Some(Hit::SketchPoint { sketch, at, .. }) => {
            if let Some(ss) = st.sketch(*sketch)
                && let Some(p) = proj.to_screen(ss.plane.to_world(*at))
            {
                painter.circle_stroke(p, 6.0, Stroke::new(2.0, col));
            }
        }
        _ => {}
    }
}

/// Sketch points and the status of the active sketch (constraint glyphs: `sketch_tools`).
fn overlays(app: &SolveApp, painter: &egui::Painter, proj: &Proj) {
    let t = Tokens::get();
    let st = app.session.world_state();
    let Some(sid) = app.session.active_sketch else { return };
    let Some(ss) = st.sketch(sid) else { return };
    let sk = &ss.sketch;
    let locked = sk.locked_points();
    let virtual_corners: Vec<usize> = sk.virtual_corners().into_iter().map(|(i, _)| i).collect();
    for (i, p) in sk.points.iter().enumerate() {
        if !solvecraft_engine::view::sketch_point_visible(sk, i) {
            continue;
        }
        let hidden = (sk.view.hide_points && i != 0) || (sk.view.hide_projected && p.link.is_some());
        let Some(sp) = proj.to_screen(ss.plane.to_world(p.pos)).filter(|_| !hidden) else { continue };
        let det = ss.report.point_determined.get(i).copied().unwrap_or(false);
        let c = if p.link.is_none() && locked.get(i).copied().unwrap_or(false) {
            t.sketch_locked
        } else if det {
            t.sketch_point
        } else {
            t.sketch_point_free
        };
        if i == 0 {
            painter.circle(sp, 3.5, Color32::from_rgb(240, 200, 60), Stroke::new(1.0, t.body_edge));
        } else if virtual_corners.binary_search(&i).is_ok() {
            // A virtual sharp corner: an open square, it is not on any drawn curve.
            painter.rect_stroke(Rect::from_center_size(sp, vec2(6.0, 6.0)), 0.0, Stroke::new(1.2, c), egui::StrokeKind::Middle);
        } else {
            painter.rect_filled(Rect::from_center_size(sp, vec2(5.0, 5.0)), 0.0, c);
        }
    }
    // Sketch status.
    if let Some(r) = app.viewport.rect {
        let msg = if !ss.report.ok() {
            format!("{} — constraints conflict", ss.name)
        } else if ss.report.dof == 0 {
            format!("{} — fully constrained", ss.name)
        } else {
            format!("{} — {} degrees of freedom", ss.name, ss.report.dof)
        };
        let c = if ss.report.ok() { t.sketch_accent } else { t.error };
        painter.text(pos2(r.left() + 12.0, r.top() + 12.0), Align2::LEFT_TOP, msg, FontId::proportional(13.0), c);
    }
}

fn cpu_render(app: &mut SolveApp, ctx: &egui::Context, painter: &egui::Painter, rect: Rect, proj: &Proj) {
    if app.viewport.no_pixels {
        return;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    sketch_scene_key(app, proj).hash(&mut h);
    for v in [proj.cam.yaw, proj.cam.pitch, proj.cam.roll, proj.cam.distance, proj.cam.target.x, proj.cam.target.y, proj.cam.target.z] {
        v.to_bits().hash(&mut h);
    }
    (rect.width() as u32, rect.height() as u32).hash(&mut h);
    let key = h.finish();
    if app.viewport.cpu.as_ref().is_none_or(|(k, _)| *k != key) {
        let mut budget = solvecraft_engine::render::SamplingBudget::new(131_072, 1_048_576);
        let scene =
            solvecraft_engine::view::scene_with_section_and_sketch_sampler(&app.session, &proj.cam, section_plane(app), |sk, index, plane| {
                crate::sketch_display::curve_with_budget(sk, index, plane, proj, app.viewport.pixels_per_point(), &mut budget).parts
            });
        let w = (rect.width() as usize).clamp(16, 4096);
        let hh = (rect.height() as usize).clamp(16, 4096);
        let c = solvecraft_engine::render::render(&scene, &proj.cam, w, hh);
        let img = egui::ColorImage::from_rgba_unmultiplied([c.w, c.h], &c.rgba);
        let tex = ctx.load_texture("sc_cpu_viewport", img, egui::TextureOptions::LINEAR);
        app.viewport.cpu = Some((key, tex));
    }
    if let Some((_, tex)) = &app.viewport.cpu {
        painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

pub(crate) fn refresh_layout(app: &mut SolveApp, scale: f32) {
    if let Some(rect) = app.viewport.rect {
        let docks: Vec<_> = [
            app.viewport.palette_rect.filter(|_| app.session.active_sketch.is_some() && app.dialog.is_none()),
            app.viewport.dialog_rect.filter(|_| app.dialog.is_some()),
        ]
        .into_iter()
        .flatten()
        .collect();
        app.viewport.layout = Some(crate::viewport_layout::Layout::new(rect, &docks, scale));
    }
}

/// Compact orientation gizmo, drawn and hit-tested in world coordinates.
const AXIS_LENGTH: f64 = 38.0;
const AXIS_RADIUS: f32 = 11.0;

fn axis_center(app: &SolveApp, rect: Rect) -> Pos2 {
    axis_rect(app, rect).center()
}

fn axis_rect(app: &SolveApp, rect: Rect) -> Rect {
    app.viewport
        .layout
        .as_ref()
        .filter(|l| l.viewport == rect)
        .map(|l| l.axis_view)
        .unwrap_or_else(|| Rect::from_center_size(pos2(rect.right() - 68.0, rect.top() + 68.0), vec2(116.0, 116.0)))
}

/// Six signed world axes, sorted back to front so overlapping tips have one owner.
fn axis_tips(cam: &Camera, c: Pos2) -> Vec<(&'static str, Vec3, Pos2, f64)> {
    let (r, u, b) = cam.basis();
    let mut tips: Vec<_> = [("x", Vec3::X), ("-x", -Vec3::X), ("y", Vec3::Y), ("-y", -Vec3::Y), ("z", Vec3::Z), ("-z", -Vec3::Z)]
        .into_iter()
        .map(|(name, d)| (name, d, c + vec2((d.dot(r) * AXIS_LENGTH) as f32, -(d.dot(u) * AXIS_LENGTH) as f32), d.dot(b)))
        .collect();
    tips.sort_by(|a, b| a.3.total_cmp(&b.3));
    tips
}

fn axis_hit(tips: &[(&str, Vec3, Pos2, f64)], p: Pos2) -> Option<usize> {
    tips.iter().enumerate().rev().find(|(_, (_, _, at, _))| at.distance(p) <= AXIS_RADIUS).map(|(i, _)| i)
}

/// Six principal views, twelve edge diagonals and eight isometric corner directions.
fn snap_directions() -> impl Iterator<Item = Vec3> {
    [-1.0, 0.0, 1.0].into_iter().flat_map(|x| {
        [-1.0, 0.0, 1.0].into_iter().flat_map(move |y| [-1.0, 0.0, 1.0].into_iter().filter_map(move |z| Vec3::new(x, y, z).normalized()))
    })
}

/// Snap to the nearest standard or diagonal direction in the user's up-axis frame.
fn snapped_axis_camera(cam: Camera) -> Camera {
    let b = cam.to_frame(cam.back());
    let dir = snap_directions().max_by(|a, c| a.dot(b).total_cmp(&c.dot(b))).unwrap_or(Vec3::Z);
    let mut to = cam.looking_from(cam.from_frame(dir));
    if dir.x == 0.0 && dir.y == 0.0 {
        to.yaw = nearest_quarter_turn(cam.yaw);
    }
    to
}

pub(crate) struct OrbitSnap {
    target: Camera,
    yaw_velocity: f64,
    pitch_velocity: f64,
    pivot: Option<(Vec3, Pos2)>,
}

/// Pointer motion accumulates independently of the spring preview, so many small deltas
/// still cross snap boundaries. Keep a small angular margin to avoid boundary flicker.
fn orbit_drag(app: &mut SolveApp, rect: Rect, dx: f64, dy: f64, press: Option<Pos2>, snap: bool, around_cursor: bool) {
    let displayed = app.cam;
    if let Some(free) = app.viewport.orbit_free.take() {
        app.cam = free;
    }
    if around_cursor {
        crate::prefs::orbit(app, rect, dx, dy, press);
    } else {
        app.cam.orbit(dx, dy);
    }
    let free = app.cam;
    app.viewport.orbit_free = Some(free);
    if snap || !around_cursor || app.viewport.orbit_snap.is_some() {
        let mut target = if snap { snapped_axis_camera(free) } else { free };
        if snap && let Some(previous) = app.viewport.orbit_snap.as_ref() {
            let b = free.back();
            if previous.target.back().dot(b) + 0.025 >= target.back().dot(b) {
                target.yaw = previous.target.yaw;
                target.pitch = previous.target.pitch;
            }
        }
        let pivot = app.viewport.orbit_pivot.filter(|_| around_cursor).and_then(|p| {
            app.cam = displayed;
            projection(app, rect).to_screen(p).map(|at| (p, at))
        });
        let spring = app.viewport.orbit_snap.get_or_insert(OrbitSnap { target, yaw_velocity: 0.0, pitch_velocity: 0.0, pivot });
        spring.target = target;
        spring.pivot = pivot;
        app.cam = displayed;
    }
}

/// A lightly damped spring: mostly settled in about a third of a second, with a small
/// elastic finish. Bounded substeps keep the behavior consistent across frame rates.
fn step_orbit_snap(app: &mut SolveApp, rect: Rect, dt: f64) {
    let Some(mut spring) = app.viewport.orbit_snap.take() else { return };
    let dt = if dt.is_finite() { dt.clamp(0.0, 0.05) } else { 1.0 / 60.0 };
    let steps = (dt * 240.0).ceil().max(1.0) as usize;
    let h = dt / steps as f64;
    let omega = 16.0;
    let damping = 0.82;
    let yaw_error = |yaw: f64| (spring.target.yaw - yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    for _ in 0..steps.min(12) {
        spring.yaw_velocity += (omega * omega * yaw_error(app.cam.yaw) - 2.0 * damping * omega * spring.yaw_velocity) * h;
        spring.pitch_velocity += (omega * omega * (spring.target.pitch - app.cam.pitch) - 2.0 * damping * omega * spring.pitch_velocity) * h;
        app.cam.yaw += spring.yaw_velocity * h;
        app.cam.pitch = (app.cam.pitch + spring.pitch_velocity * h).clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
    }
    app.cam.target = spring.target.target;
    let done = yaw_error(app.cam.yaw).abs() < 1e-5
        && (spring.target.pitch - app.cam.pitch).abs() < 1e-5
        && spring.yaw_velocity.abs() < 1e-4
        && spring.pitch_velocity.abs() < 1e-4;
    if done {
        app.cam = spring.target;
    }
    if let Some((pivot, at)) = spring.pivot
        && let Some(after) = projection(app, rect).to_screen(pivot)
    {
        app.cam.pan(f64::from(at.x - after.x), f64::from(at.y - after.y), f64::from(rect.height().max(1.0)));
    }
    if !done {
        app.viewport.orbit_snap = Some(spring);
    }
}

/// Dragging anywhere on the axis gizmo orbits about the view centre. A stationary press
/// on a tip chooses its view. Floating windows own their input over the gizmo.
fn axis_view_orbit(app: &mut SolveApp, ui: &mut egui::Ui, rect: Rect) -> bool {
    let (pressed, down, decided, origin, pos, shift) = ui.input(|i| {
        let p = &i.pointer;
        (p.primary_pressed(), p.primary_down(), p.is_decidedly_dragging(), p.press_origin(), p.interact_pos(), i.modifiers.shift)
    });
    if pressed && ui.rect_contains_pointer(rect) {
        app.viewport.axis_drag = origin.filter(|p| axis_rect(app, rect).contains(*p));
        if app.viewport.axis_drag.is_some() {
            app.viewport.orbit_free = None;
        }
    }
    if !down && app.viewport.axis_drag.take().is_some() {
        app.viewport.orbit_free = None;
    }
    let Some(last) = app.viewport.axis_drag.filter(|_| decided) else { return false };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    if let Some(p) = pos {
        app.cancel_view_animation();
        orbit_drag(app, rect, f64::from(p.x - last.x), f64::from(p.y - last.y), None, shift, false);
        app.viewport.axis_drag = Some(p);
        ui.ctx().request_repaint();
    }
    true
}

fn axis_view(app: &mut SolveApp, ui: &mut egui::Ui, rect: Rect) {
    if !axis_rect(app, rect).is_positive() {
        app.viewport.axis_drag = None;
        crate::scenario::publish_count("axis_view_highlights", 0.0);
        return;
    }
    crate::scenario::publish_rect("axis_view:outer", axis_rect(app, rect));
    let t = Tokens::get();
    let c = axis_center(app, rect);
    let orbiting = axis_view_orbit(app, ui, rect);
    let tips = axis_tips(&app.cam, c);
    let hover = ui.input(|i| i.pointer.hover_pos()).filter(|_| ui.rect_contains_pointer(rect));
    let hovered = if orbiting { None } else { hover.and_then(|p| axis_hit(&tips, p)) };
    let painter = ui.painter_at(rect);
    if orbiting || hover.is_some_and(|p| axis_rect(app, rect).contains(p)) {
        painter.circle_filled(c, 55.0, t.overlay);
    }
    for (i, (name, d, at, depth)) in tips.iter().enumerate() {
        let col = if d.x != 0.0 {
            colors::AXIS_X
        } else if d.y != 0.0 {
            colors::AXIS_Y
        } else {
            colors::AXIS_Z
        };
        let col = Color32::from_rgb(col.0, col.1, col.2);
        let positive = !name.starts_with('-');
        let ink = if *depth < -0.01 { col.gamma_multiply(0.55) } else { col };
        painter.line_segment([c, *at], Stroke::new(if positive { 2.0 } else { 1.0 }, ink));
        painter.circle_filled(*at, AXIS_RADIUS, if hovered == Some(i) { t.accent_soft } else { t.overlay });
        painter.circle_stroke(
            *at,
            AXIS_RADIUS,
            Stroke::new(if hovered == Some(i) { 2.0 } else { 1.0 }, if hovered == Some(i) { t.accent } else { ink }),
        );
        painter.text(*at, Align2::CENTER_CENTER, name.to_uppercase(), FontId::proportional(11.0), ink);
        crate::scenario::publish_handle(&format!("axis_view:{name}"), *at);
    }
    crate::scenario::publish_count("axis_view_highlights", f64::from(u8::from(hovered.is_some())));
    if let Some(i) = hovered
        && let Some((name, d, _, _)) = tips.get(i)
    {
        crate::scenario::publish_handle(&format!("axis_view_hover:{name}"), hover.unwrap_or(c));
        ui.interact(Rect::from_center_size(hover.unwrap_or(c), vec2(2.0, 2.0)), ui.id().with("axis_view_tip"), Sense::hover())
            .on_hover_text(format!("View from {}. Click again to reverse. Drag to orbit; hold Shift to snap.", name.to_uppercase()));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if ui.input(|i| i.pointer.primary_clicked()) {
            let frame = app.cam.to_frame(*d);
            let dir = if app.cam.back().dot(*d) > 0.9999 { -*d } else { *d };
            let mut to = app.cam.looking_from(dir);
            if frame.z.abs() > 0.5 {
                to.yaw = nearest_quarter_turn(app.cam.yaw);
            }
            if app.preferences.look_at_anim {
                app.cancel_view_animation();
                app.viewport.orbit_free = None;
                app.viewport.orbit_snap = Some(OrbitSnap { target: to, yaw_velocity: 0.0, pitch_velocity: 0.0, pivot: None });
                ui.ctx().request_repaint();
            } else {
                app.animate_to(to);
            }
        }
    }
    let home = Rect::from_center_size(pos2(c.x - 44.0, c.y - 44.0), vec2(18.0, 18.0));
    crate::scenario::publish_rect("axis_view:home", home);
    crate::scenario::publish_handle("axis_view:home", home.center());
    let hr = ui.interact(home, ui.id().with("axis_view_home"), Sense::click());
    icons::paint(&painter, home, "home", t.icon, if hr.hovered() { t.accent_soft } else { t.overlay }, t.accent);
    if hr.on_hover_text("Home view").clicked() {
        app.animate_view("home");
    }
}

/// At a halfway tie, prefer the square heading nearer the standard plan view.
fn nearest_quarter_turn(yaw: f64) -> f64 {
    if !yaw.is_finite() {
        return 0.0;
    }
    let q = std::f64::consts::FRAC_PI_2;
    let lo = (yaw / q).floor() * q;
    let hi = lo + q;
    let (dlo, dhi) = (yaw - lo, hi - yaw);
    if (dlo - dhi).abs() < 1e-6 {
        let off_front = |a: f64| ((a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI).abs();
        if off_front(lo) <= off_front(hi) { lo } else { hi }
    } else if dlo < dhi {
        lo
    } else {
        hi
    }
}

/// Navigation bar (bottom centre).
/// Turn the view to face the selected planar face or plane, else the active sketch.
pub fn look_at_selection(app: &mut SolveApp) {
    let target = app.session.selection.iter().find_map(|s| match s {
        Sel::Face { body, index, point } => crate::dialogs::planar_face(&app.session, body, *index).map(|(_, n)| (*point, n)),
        Sel::Plane { name } => solvecraft_engine::geom::Plane::named(name).map(|p| (p.origin, p.normal())),
        _ => None,
    });
    match target {
        Some((p, n)) => {
            let mut to = app.cam.looking_from(n);
            to.target = p;
            app.animate_to(to);
        }
        None => crate::dialogs::look_at_sketch(app),
    }
}

fn nav_bar(app: &mut SolveApp, ui: &mut egui::Ui, rect: Rect) {
    if app.viewport.layout.as_ref().is_some_and(|l| !l.navigation.is_positive()) {
        return;
    }
    let t = Tokens::get();
    let items: [(&str, &str); 10] = [
        ("orbit", "Orbit (right drag; hold Shift to snap)"),
        ("spin", "Continuous orbit (stops at the next click or key)"),
        ("pan", "Pan (drag; or middle drag)"),
        ("zoom", "Zoom (drag; or wheel)"),
        ("fit", "Fit (F6)"),
        ("lookat", "Look At the selected face or plane (or the active sketch)"),
        ("home", "Home view"),
        ("perspective", "Perspective / orthographic"),
        ("style", "Visual style: shaded with edges, shaded, wireframe, shaded with hidden edges"),
        ("settings", "View settings: grid, ground shadow, agent cursor"),
    ];
    let menu_id = ui.id().with("nav_view_menu");
    let mut menu_open = ui.data(|d| d.get_temp::<bool>(menu_id)).unwrap_or(false);
    let w = items.len() as f32 * 30.0 + 10.0;
    let bar = app
        .viewport
        .layout
        .as_ref()
        .map(|l| l.navigation)
        .unwrap_or_else(|| Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 22.0), vec2(w, 30.0)));
    crate::scenario::publish_rect("nav:outer", bar);
    let painter = ui.painter_at(rect);
    painter.rect(bar, 6.0, t.overlay, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let columns = ((bar.width() - 10.0) / 30.0).round().clamp(1.0, 10.0) as usize;
    for (i, (icon, tip)) in items.iter().enumerate() {
        let br = Rect::from_min_size(
            pos2(bar.left() + 5.0 + (i % columns) as f32 * 30.0, bar.top() + 2.0 + (i / columns) as f32 * 30.0),
            vec2(26.0, 26.0),
        );
        let resp = ui.interact(br, ui.id().with(("nav", *icon)), Sense::click());
        let active = match *icon {
            "orbit" => app.viewport.nav == Some(NavMode::Orbit),
            "pan" => app.viewport.nav == Some(NavMode::Pan),
            "zoom" => app.viewport.nav == Some(NavMode::Zoom),
            "perspective" => app.ui.perspective,
            "settings" => menu_open,
            "spin" => app.viewport.spin,
            _ => false,
        };
        if active || resp.hovered() {
            painter.rect_filled(br, 4.0, if active { t.accent_soft } else { t.hover });
        }
        icons::paint(&painter, br.shrink(4.0), icon, t.icon, t.icon_fill, t.accent);
        if resp.on_hover_text(*tip).clicked() {
            let toggle = |m: NavMode, cur: Option<NavMode>| if cur == Some(m) { None } else { Some(m) };
            match *icon {
                "orbit" => app.viewport.nav = toggle(NavMode::Orbit, app.viewport.nav),
                "pan" => app.viewport.nav = toggle(NavMode::Pan, app.viewport.nav),
                "zoom" => app.viewport.nav = toggle(NavMode::Zoom, app.viewport.nav),
                "fit" => app.animate_view("fit"),
                "spin" => {
                    app.cancel_view_animation();
                    app.viewport.spin = !app.viewport.spin;
                }
                "home" => app.animate_view("home"),
                "lookat" => look_at_selection(app),
                "style" => app.ui.visual_style = (app.ui.visual_style + 1) % 4,
                "perspective" => app.ui.perspective = !app.ui.perspective,
                _ => menu_open = !menu_open,
            }
        }
        if *icon == "settings" {
            crate::scenario::publish_handle("nav:view", br.center());
            if menu_open {
                menu_open = view_menu(app, ui, br);
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(menu_id, menu_open));
}

/// The View settings above the navigation bar's last button: grid, ground shadow and the agent
/// cursor (#35). False once it should close (a click outside it).
fn view_menu(app: &mut SolveApp, ui: &mut egui::Ui, button: Rect) -> bool {
    let area = crate::frame::area(egui::Id::new("sc_view_menu"))
        .order(egui::Order::Foreground)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(button.right_top() - vec2(0.0, 6.0));
    let r = area.show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(220.0);
            ui.label(egui::RichText::new("VIEW").strong().size(11.5));
            ui.checkbox(&mut app.ui.show_grid, "Grid");
            ui.checkbox(&mut app.ui.ground_shadow, "Ground shadow");
            ui.separator();
            let s = &mut app.ui.agent_cursor;
            let show = ui.checkbox(&mut s.show, "Show Agent Cursor");
            crate::scenario::publish_handle("view:agent_cursor", show.rect.center());
            ui.add_enabled_ui(s.show, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Speed");
                    crate::agent_cursor::speed_slider(ui, &mut s.speed);
                });
                ui.checkbox(&mut s.follow_camera, "Follow camera (Look At sketches)");
            });
        });
    });
    let clicked_out = ui.input(|i| i.pointer.any_pressed())
        && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !r.response.rect.contains(p) && !button.contains(p));
    !clicked_out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_viewport_uses_section_preview_and_sketch_slice() {
        fn pixels(h: &mut crate::scenario::Harness, cam: Camera) -> Vec<Color32> {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(320.0, 240.0));
            let proj = Proj { cam, vp: cam.view_proj(4.0 / 3.0, 100.0), rect };
            h.app.viewport.no_pixels = false;
            // Force a texture upload so this test reads the actual CPU rasterizer result.
            h.app.viewport.cpu = None;
            let mut out = h.ctx.run_ui(egui::RawInput { screen_rect: Some(rect), ..Default::default() }, |ui| {
                cpu_render(&mut h.app, ui.ctx(), ui.painter(), rect, &proj);
            });
            let id = h.app.viewport.cpu.as_ref().unwrap().1.id();
            let upload = out.textures_delta.set.get(&id).unwrap().iter().find(|delta| delta.pos.is_none()).unwrap();
            let egui::ImageData::Color(image) = &upload.image;
            assert_eq!(image.size, [320, 240]);
            let pixels = image.pixels.clone();
            out.textures_delta.clear();
            pixels
        }
        let mut h = crate::scenario::Harness::new();
        h.app.run("solid.box", json!({"length":40,"width":30,"height":20})).unwrap();
        let cam = solvecraft_engine::view::home_camera(&h.app.session);
        let whole = pixels(&mut h, cam);
        let before = h.app.session.doc.clone();
        let undo = h.app.session.undo.len();
        h.app.session.selection = vec![Sel::Plane { name: "XZ".into() }];
        h.app.start("inspect.section");
        let Some(crate::dialogs::Kind::Section { offset, .. }) = h.app.dialog.as_mut().map(|d| &mut d.kind) else { panic!("section dialog") };
        *offset = "15 mm".into();
        assert!(h.app.session.section.is_none());
        let preview = pixels(&mut h, cam);
        assert!(whole.iter().zip(&preview).filter(|(a, b)| a != b).count() > 100, "CPU viewport ignored uncommitted section preview");
        assert!(std::sync::Arc::ptr_eq(&before, &h.app.session.doc));
        assert_eq!(h.app.session.undo.len(), undo);
        h.app.run("inspect.section", json!({"plane":"XZ","offset":15})).unwrap();
        h.app.dialog = None;
        assert_eq!(preview, pixels(&mut h, cam), "preview and committed section must rasterize identically");

        h.app.session.section = None;
        h.app.run("sketch.create", json!({"plane":{"origin":[0,0,10],"x_dir":[1,0,0],"y_dir":[0,1,0]}})).unwrap();
        let whole = pixels(&mut h, cam);
        h.app.run("sketch.options", json!({"slice":true})).unwrap();
        assert!(h.app.session.section.is_none());
        let sliced = pixels(&mut h, cam);
        assert!(whole.iter().zip(&sliced).filter(|(a, b)| a != b).count() > 100, "CPU viewport ignored active Sketch Slice");
        h.app.run("sketch.options", json!({"slice":false})).unwrap();
        assert_eq!(whole, pixels(&mut h, cam), "disabling Slice restores the whole-body pixels");
    }

    #[test]
    fn timeline_modifiers_highlight_on_hover_and_click() {
        for command in ["solid.fillet", "solid.chamfer"] {
            let mut h = crate::scenario::Harness::new();
            h.app.ui.show_origin = false;
            let base = h.app.run("solid.box", json!({"length":20,"width":20,"height":20})).unwrap()["feature"].as_u64().unwrap();
            let id = h.app.run(command, json!({"edges":[[0,0,10]],"radius":2,"distance":2,"name":"Blend"})).unwrap()["feature"].as_u64().unwrap();
            h.app.run("solid.box", json!({"length":5,"width":5,"height":5,"corner":[40,0,0]})).unwrap();
            h.frames(2);
            let geometry = feature_geometry(&h.app, id);
            assert_eq!(geometry.len(), 1, "{command}: {geometry:?}");
            let Sel::Face { body, index, .. } = &geometry[0] else { panic!("a modifier must highlight its face") };
            let st = h.app.session.world_state();
            let m = st.body(body).unwrap().mesh();
            let triangles = m.tri_face.iter().filter(|f| **f as usize == *index).count();
            assert!(triangles > 0 && triangles < m.triangles.len());
            let p = crate::scenario::handle_at("timeline:Blend").unwrap();
            h.step(&json!({"move":[p.x,p.y]})).unwrap();
            assert_eq!(h.app.viewport.hover_feature, Some(id));
            assert_eq!(build_highlight(&h.app).tris.len(), triangles * 3 * crate::gpu::TRI_SIZE);
            assert!(!build_highlight(&h.app).lines.is_empty());
            h.step(&json!({"click":[p.x,p.y]})).unwrap();
            assert_eq!(h.app.session.selection, vec![Sel::Feature { id }]);
            // Moving away removes hover but leaves the selected blend highlighted.
            h.step(&json!({"move":[5,5]})).unwrap();
            assert_eq!(h.app.viewport.hover_feature, None);
            assert_eq!(build_highlight(&h.app).tris.len(), triangles * 3 * crate::gpu::TRI_SIZE);
            let bounds = selection_bounds(&h.app).unwrap();
            assert!(bounds.max.x <= 2.01 && bounds.max.y <= 2.01 && bounds.max.z > 19.99, "{bounds:?}");
            // Creator features retain whole-body feedback, while hidden bodies stay hidden.
            assert_eq!(feature_geometry(&h.app, base), vec![Sel::Body { name: body.clone() }]);
            h.app.session.visibility.hidden_bodies.push(body.clone());
            assert!(feature_geometry(&h.app, id).is_empty());
            assert!(selection_bounds(&h.app).is_none());
            assert!(build_highlight(&h.app).tris.is_empty());
        }
    }

    #[test]
    fn timeline_modifier_highlight_follows_edits_and_rollback() {
        let mut app = SolveApp::new(solvecraft_engine::Session::default(), crate::Services::default());
        app.run("solid.box", json!({"length":20,"width":20,"height":20})).unwrap();
        let id = app.run("solid.fillet", json!({"edges":[[0,0,10]],"radius":2})).unwrap()["feature"].as_u64().unwrap();
        // A later chamfer changes the same body without consuming the fillet.
        app.run("solid.chamfer", json!({"edges":[[20,20,10]],"distance":1})).unwrap();
        app.run("solid.move", json!({"bodies":["Body1"],"translate":[30,0,0]})).unwrap();
        app.run("body.rename", json!({"body":"Body1","name":"Moved"})).unwrap();
        app.session.selection = vec![Sel::Feature { id }];
        let bounds = selection_bounds(&app).unwrap();
        assert!(bounds.min.x >= 29.99 && bounds.max.x <= 32.01, "{bounds:?}");
        assert!(feature_geometry(&app, id).iter().all(|x| matches!(x, Sel::Face { body, .. } if body == "Moved")));
        app.run("timeline.roll_to", json!({"position":1})).unwrap();
        assert!(feature_geometry(&app, id).is_empty());
        app.run("timeline.roll_to", json!({})).unwrap();
        assert!(!feature_geometry(&app, id).is_empty());
        app.run("timeline.suppress", json!({"feature":id})).unwrap();
        assert!(feature_geometry(&app, id).is_empty());
    }

    #[test]
    fn timeline_modifier_highlight_follows_component_instances() {
        let mut app = SolveApp::new(solvecraft_engine::Session::default(), crate::Services::default());
        app.run("component.create", json!({"name":"Part"})).unwrap();
        app.run("solid.box", json!({"length":20,"width":20,"height":20})).unwrap();
        let id = app.run("solid.fillet", json!({"edges":[[0,0,10]],"radius":2})).unwrap()["feature"].as_u64().unwrap();
        app.run("occurrence.move", json!({"occurrence":"Part:1","translate":[30,0,0]})).unwrap();
        app.run("occurrence.copy", json!({"component":"Part","translate":[0,40,0]})).unwrap();
        let geometry = feature_geometry(&app, id);
        assert_eq!(geometry.len(), 2, "both occurrences retain the fillet's face: {geometry:?}");
        app.session.selection = vec![Sel::Feature { id }];
        let bounds = selection_bounds(&app).unwrap();
        assert!(bounds.min.x >= 29.99 && bounds.max.x <= 32.01 && bounds.max.y >= 41.99, "{bounds:?}");
    }

    fn hover_frame(app: &mut SolveApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let selected = app.highlighted();
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() },
            |ui| {
                show(app, ui);
            },
        );
        // This headless frame checks interactions without applying textures to a renderer.
        output.textures_delta.clear();
        // Click selection is applied after rendering; hover itself must already be current.
        if app.viewport.gpu.is_some() && selected == app.highlighted() {
            assert_eq!(app.viewport.hl_key, highlight_key(app), "GPU highlight must use this frame's hover");
        }
    }

    fn hover_app(gpu: bool) -> SolveApp {
        let mut s = solvecraft_engine::Session::default();
        s.execute("sketch.create", &json!({"plane": "XY"})).unwrap();
        s.execute("sketch.point", &json!({"point": [0, 0]})).unwrap();
        let mut app = SolveApp::new(s, crate::Services::default());
        app.viewport.no_pixels = true;
        if gpu {
            // The callback is queued by egui; no device is needed to inspect scene updates.
            app.viewport.gpu = Some(GpuTarget { format: egui_wgpu::wgpu::TextureFormat::Rgba8Unorm, depth: None, samples: 1 });
        }
        app
    }

    #[test]
    fn hover_stays_off_between_navigation_events_and_returns_on_release() {
        for gpu in [false, true] {
            for button in [egui::PointerButton::Middle, egui::PointerButton::Secondary, egui::PointerButton::Primary] {
                let ctx = egui::Context::default();
                let mut app = hover_app(gpu);
                if button == egui::PointerButton::Primary {
                    app.viewport.nav = Some(NavMode::Orbit);
                }
                hover_frame(&mut app, &ctx, vec![]);
                let p = projection(&app, app.viewport.rect.unwrap()).to_screen(Vec3::ZERO).unwrap();
                hover_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
                assert!(app.viewport.hover.is_some());
                hover_frame(&mut app, &ctx, vec![egui::Event::PointerButton { pos: p, button, pressed: true, modifiers: Default::default() }]);
                assert!(app.viewport.hover.is_none(), "hover on navigation press");
                for offset in [1.0, 2.0, 3.0] {
                    hover_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(p + vec2(offset, 0.0))]);
                    assert!(app.viewport.hover.is_none(), "hover while moving");
                    for _ in 0..3 {
                        hover_frame(&mut app, &ctx, vec![]);
                        assert!(app.viewport.hover.is_none(), "hover between movement events");
                    }
                }
                hover_frame(&mut app, &ctx, vec![egui::Event::PointerButton { pos: p, button, pressed: false, modifiers: Default::default() }]);
                assert!(app.viewport.hover.is_some(), "hover must return on release");
            }
        }
    }

    #[test]
    fn hover_stays_off_during_spin_and_animated_views() {
        let ctx = egui::Context::default();
        let mut app = hover_app(true);
        hover_frame(&mut app, &ctx, vec![]);
        let p = projection(&app, app.viewport.rect.unwrap()).to_screen(Vec3::ZERO).unwrap();
        hover_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
        assert!(app.viewport.hover.is_some());
        app.viewport.spin = true;
        for _ in 0..3 {
            hover_frame(&mut app, &ctx, vec![]);
            assert!(app.viewport.hover.is_none());
        }
        app.viewport.spin = false;
        hover_frame(&mut app, &ctx, vec![]);
        assert!(app.viewport.hover.is_some());
        app.cam_anim = Some(solvecraft_engine::render::CameraAnim::new(app.cam, app.cam, 0.0));
        for _ in 0..3 {
            hover_frame(&mut app, &ctx, vec![]);
            assert!(app.viewport.hover.is_none());
        }
        app.cam_anim = None;
        hover_frame(&mut app, &ctx, vec![]);
        assert!(app.viewport.hover.is_some());
    }

    fn cylinder_app() -> SolveApp {
        let mut s = solvecraft_engine::Session::default();
        s.execute("solid.cylinder", &json!({"radius": 10, "height": 20})).unwrap();
        let mut app = SolveApp::new(s, crate::Services::default());
        app.ui.show_origin = false;
        app
    }

    #[test]
    fn smooth_wall_display_preserves_wireframe_and_edge_selection() {
        let mut s = solvecraft_engine::Session::default();
        let sketch = s.execute("sketch.create", &json!({"plane":"XY"})).unwrap()["sketch"].as_u64().unwrap();
        s.execute("sketch.ellipse", &json!({"center":[0,0],"major":[25,0],"minor_radius":5})).unwrap();
        let inner =
            s.execute("sketch.offset", &json!({"curves":["e1"],"distance":-1,"chain":false})).unwrap()["curves"][0].as_str().unwrap().to_string();
        let outer =
            s.execute("sketch.offset", &json!({"curves":[inner],"distance":-0.5,"chain":false})).unwrap()["curves"][0].as_str().unwrap().to_string();
        s.execute(
            "solid.extrude",
            &json!({"sketch":sketch,"distance":3,"profiles":{"type":"whole_boundaries","profiles":[{"outer":[outer],"holes":[[inner]]}]}}),
        )
        .unwrap();
        let mut app = SolveApp::new(s, crate::Services::default());
        app.ui.ground_shadow = false;
        let st = app.session.world_state();
        let b = st.bodies.first().unwrap();
        let mesh = b.mesh();
        assert_eq!(mesh.edges.len(), 1536);
        assert_eq!(mesh.smooth_edges.iter().filter(|f| **f).count(), 512);
        let smooth = mesh.smooth_edges.iter().position(|f| *f).unwrap();
        let expected = mesh.edges.iter().enumerate().filter(|(i, _)| mesh.shaded_edge_visible(*i)).map(|(_, e)| e.len() - 1).sum::<usize>();
        let shaded = build_scene(&app);
        assert_eq!(shaded.lines.len() / crate::gpu::LINE_SIZE, expected);
        assert!(!shaded.tris.is_empty());
        app.ui.visual_style = 2;
        let wire = build_scene(&app);
        let expected_wire =
            mesh.edges.iter().enumerate().filter(|(i, _)| !mesh.seams.get(*i).copied().unwrap_or(false)).map(|(_, e)| e.len() - 1).sum::<usize>();
        assert_eq!(wire.lines.len() / crate::gpu::LINE_SIZE, expected_wire);
        assert!(expected_wire > expected);
        assert!(wire.tris.is_empty());
        app.ui.visual_style = 0;
        app.session.selection = vec![Sel::Edge { body: b.name.clone(), index: smooth, point: polyline_mid(&mesh.edges[smooth]) }];
        assert!(!build_highlight(&app).lines.is_empty());
        let mut without_display_mask = (*mesh).clone();
        without_display_mask.smooth_edges.clear();
        assert_eq!(mesh.surface_patch(mesh.edge_faces[smooth][0] as usize), without_display_mask.surface_patch(mesh.edge_faces[smooth][0] as usize));
        let cam = solvecraft_engine::view::home_camera(&app.session);
        let cpu = solvecraft_engine::view::scene(&app.session, &cam);
        let actual_edges = cpu.lines.iter().filter(|line| line.color == solvecraft_engine::view::colors::EDGE).count();
        assert_eq!(actual_edges, mesh.edges.iter().enumerate().filter(|(i, _)| mesh.shaded_edge_visible(*i)).count());
        if let Ok(directory) = std::env::var("SOLVECRAFT_SMOOTH_SCREENSHOTS") {
            std::fs::create_dir_all(&directory).unwrap();
            for (name, old_edges, wireframe) in [("after", false, false), ("before", true, false), ("wireframe", true, true)] {
                let mut scene = solvecraft_engine::view::scene(&app.session, &cam);
                scene.lines.retain(|line| line.color == solvecraft_engine::view::colors::EDGE);
                if old_edges {
                    scene.lines.clear();
                    for (i, edge) in mesh.edges.iter().enumerate() {
                        if mesh.edge_pickable(i) {
                            scene.lines.push(solvecraft_engine::render::SceneLine {
                                points: edge.clone(),
                                color: solvecraft_engine::view::colors::EDGE,
                                width: 1.2,
                                on_top: false,
                            });
                        }
                    }
                }
                if wireframe {
                    scene.meshes.clear();
                }
                let png = solvecraft_engine::render::render_png(&scene, &cam, 1200, 800).unwrap();
                std::fs::write(std::path::Path::new(&directory).join(format!("smooth-ring-{name}.png")), png).unwrap();
            }
        }
    }

    #[test]
    fn certified_small_crease_overrides_old_seam_fit_for_picking_and_patches() {
        let mut s = solvecraft_engine::Session::default();
        let sketch = s.execute("sketch.create", &json!({"plane":"XY"})).unwrap()["sketch"].as_u64().unwrap();
        let step = std::f64::consts::TAU / 8.;
        let k = 4. / 3. * (step / 4.).tan() * 10.;
        for i in 0..8 {
            let a = i as f64 * step;
            let b = (i + 1) as f64 * step;
            let p0 = [10. * a.cos(), 10. * a.sin()];
            let p3 = if i == 7 { [10., 0.] } else { [10. * b.cos(), 10. * b.sin()] };
            let mut p1 = [p0[0] - k * a.sin(), p0[1] + k * a.cos()];
            let mut p2 = [p3[0] + k * b.sin(), p3[1] - k * b.cos()];
            if i == 0 {
                p1[0] += k * 1e-6;
                p2[0] += k * 1e-6 * b.cos();
                p2[1] += k * 1e-6 * b.sin();
            }
            s.execute("sketch.spline.control_point", &json!({"points":[p0,p1,p2,p3]})).unwrap();
        }
        s.execute("solid.extrude", &json!({"sketch":sketch,"distance":3})).unwrap();
        let mut app = SolveApp::new(s, crate::Services::default());
        let st = app.session.world_state();
        let body = st.bodies.first().unwrap();
        let mesh = body.mesh();
        let crease = mesh
            .edges
            .iter()
            .position(|line| {
                line.first().zip(line.last()).is_some_and(|(a, b)| {
                    (a.x - 10.).abs() < 1e-8 && a.y.abs() < 1e-8 && (b.x - 10.).abs() < 1e-8 && b.y.abs() < 1e-8 && (a.z - b.z).abs() > 2.9
                })
            })
            .unwrap();
        assert!(mesh.seams[crease], "the fixture must exercise the coarse seam false positive");
        assert!(mesh.sharp_edges[crease]);
        assert!(mesh.shaded_edge_visible(crease));
        assert!(mesh.edge_pickable(crease));
        assert!(mesh.edge_faces[crease].iter().any(|f| mesh.surface_patch(*f as usize).len() == 1));
        let mid = polyline_mid(&mesh.edges[crease]);
        app.cam = app.cam.looking_from(Vec3::new(30., 0., 1.5));
        app.cam.target = Vec3::new(0., 0., 1.5);
        for perspective in [false, true] {
            app.ui.perspective = perspective;
            let proj = projection(&app, Rect::from_min_size(pos2(0., 0.), vec2(800., 600.)));
            let hits = pick(&app, &proj, proj.to_screen(mid).unwrap());
            assert!(hits.iter().any(|hit| matches!(hit,Hit::Edge{index,..} if *index==crease)), "{hits:?}");
        }
    }

    #[test]
    fn circular_items_measure_as_whole_rims_and_surfaces() {
        let mut app = cylinder_app();
        let st = app.session.world_state();
        let b = &st.bodies[0];
        let m = b.mesh();
        let edges: Vec<Sel> = m
            .edges
            .iter()
            .enumerate()
            .filter(|(_, p)| p.len() > 3 && p.iter().all(|q| (q.z - 20.0).abs() < 1e-6))
            .map(|(index, p)| Sel::Edge { body: b.name.clone(), index, point: polyline_mid(p) })
            .collect();
        let result = app.session.measure_items(&edges);
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert!((result["items"][0]["length_mm"].as_f64().unwrap() - 20.0 * std::f64::consts::PI).abs() < 0.2, "{result}");
        app.session.selection = edges;
        let d = crate::dialogs::Dialog::for_command(&app, "inspect.measure").unwrap();
        assert_eq!(d.items().len(), 1);

        let faces = m.edge_faces.iter().zip(&m.seams).find(|(_, seam)| **seam).unwrap().0;
        let wall = Sel::Face { body: b.name.clone(), index: faces[0] as usize, point: Vec3::new(10.0, 0.0, 10.0) };
        let wall_pieces = grouped_selection(&app.session, &wall);
        let result = app.session.measure_items(&wall_pieces);
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert!((result["items"][0]["area_mm2"].as_f64().unwrap() - 400.0 * std::f64::consts::PI).abs() < 10.0, "{result}");
        let mut d = crate::dialogs::Dialog::for_command(&app, "inspect.measure").unwrap();
        d.pick(&app.session, wall.clone());
        assert_eq!(d.items().len(), 2, "a rim and a wall are two measurement targets");
        d.pick(&app.session, Sel::Face { body: b.name.clone(), index: faces[1] as usize, point: Vec3::new(-10.0, 0.0, 10.0) });
        assert_eq!(d.items().len(), 1, "the other half toggles the same wall");
    }

    #[test]
    fn circular_rim_hover_click_and_dialog_pick_include_both_halves() {
        let mut app = cylinder_app();
        let st = app.session.world_state();
        let b = &st.bodies[0];
        let m = b.mesh();
        let top = m.edges.iter().position(|e| e.len() > 3 && e.iter().all(|p| (p.z - 20.0).abs() < 1e-6)).unwrap();
        let rim = m.circular_rim(top);
        assert_eq!(rim.len(), 2);
        let line_count: usize = rim.iter().map(|&i| m.edges[i].len() - 1).sum();
        for &i in &rim {
            let mid = polyline_mid(&m.edges[i]);
            app.viewport.hover = Some(Hit::Edge { body: b.name.clone(), index: i, mid });
            assert_eq!(build_highlight(&app).lines.len(), 2 * line_count * crate::gpu::LINE_SIZE);
            let item = hit_sel(app.viewport.hover.as_ref().unwrap()).unwrap();
            select(&mut app, Some(item.clone()), false);
            assert_eq!(app.session.selection.len(), 2);
            // Selecting both pieces must not draw each highlight twice.
            assert_eq!(build_highlight(&app).lines.len(), 2 * line_count * crate::gpu::LINE_SIZE);
            // Ctrl/Shift clicking either half deselects the entire rim.
            select(&mut app, Some(item.clone()), true);
            assert!(app.session.selection.is_empty());
            let mut d = crate::dialogs::Dialog::for_command(&app, "solid.chamfer").unwrap();
            if let crate::dialogs::Kind::Fillet { chain, .. } = &mut d.kind {
                *chain = false;
            }
            d.pick(&app.session, item.clone());
            assert_eq!(d.items().len(), 2);
            d.pick(&app.session, item);
            assert!(d.items().is_empty());
        }
        // Screen-space picking works from either side of the rim as the model turns.
        for &i in &rim {
            let mid = polyline_mid(&m.edges[i]);
            app.cam = app.cam.looking_from(Vec3::new(mid.x, mid.y, 20.0));
            app.cam.target = Vec3::new(0.0, 0.0, 10.0);
            for perspective in [false, true] {
                app.ui.perspective = perspective;
                let proj = projection(&app, Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)));
                let hits = pick(&app, &proj, proj.to_screen(mid).unwrap());
                let (_, sel) = candidate(&app, &hits).unwrap();
                assert!(matches!(sel, Sel::Edge { .. }), "{hits:?}");
                assert_eq!(grouped_selection(&app.session, &sel).len(), 2);
            }
        }
    }

    #[test]
    fn curved_face_hover_selection_and_dialogs_cross_the_seam() {
        let mut app = cylinder_app();
        let st = app.session.world_state();
        let b = &st.bodies[0];
        let m = b.mesh();
        let faces = m.edge_faces.iter().zip(&m.seams).find(|(_, seam)| **seam).unwrap().0;
        assert_eq!(faces.len(), 2);
        let triangle_count = m.tri_face.iter().filter(|f| faces.contains(f)).count();
        for &f in faces {
            let index = f as usize;
            let t = m.triangles.iter().zip(&m.tri_face).find(|(_, tf)| **tf == f).unwrap().0;
            let [a, c, d] = m.tri(t).unwrap();
            let point = (a + c + d) / 3.0;
            app.viewport.hover = Some(Hit::Face { body: b.name.clone(), index, point });
            assert_eq!(build_highlight(&app).tris.len(), triangle_count * 3 * crate::gpu::TRI_SIZE);
            let item = hit_sel(app.viewport.hover.as_ref().unwrap()).unwrap();
            select(&mut app, Some(item.clone()), false);
            assert_eq!(app.session.selection.len(), 2);
            assert_eq!(build_highlight(&app).tris.len(), triangle_count * 3 * crate::gpu::TRI_SIZE);
            assert_eq!(app.session.selection.first(), Some(&item), "keep the original tool placement point first");
            let mut dialog = crate::dialogs::Dialog::for_command(&app, "solid.chamfer").unwrap();
            assert_eq!(dialog.items().len(), 2);
            dialog.pick(&app.session, item.clone());
            assert!(dialog.items().is_empty());
            dialog.pick(&app.session, item);
            assert_eq!(dialog.items().len(), 2);
            // A point elsewhere on the opposite half still toggles the whole surface off.
            let other = faces.iter().find(|&&x| x != f).unwrap();
            select(&mut app, Some(Sel::Face { body: b.name.clone(), index: *other as usize, point: m.positions[0] }), true);
            assert!(app.session.selection.is_empty());
        }
        // Planar caps and box corners stay separate.
        let cap = (0..b.body.face_count()).find(|&i| m.surface_patch(i).len() == 1).unwrap();
        assert_eq!(m.surface_patch(cap), vec![cap]);
        app.session.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10})).unwrap();
        let st = app.session.world_state();
        let box_body = st.bodies.last().unwrap();
        for i in 0..box_body.body.face_count() {
            assert_eq!(box_body.mesh().surface_patch(i), vec![i]);
        }
    }

    #[test]
    fn immediate_wheel_zoom_applies_normalized_input_once() {
        let mut h = crate::scenario::Harness::new();
        h.app.preferences.smooth_wheel_zoom = false;
        h.app.preferences.zoom_to_cursor = false;
        h.call("ui.move", json!({"x":800,"y":500}));
        let before = h.app.cam;
        let line = h.ctx.options(|o| o.input_options.line_scroll_speed);
        for (unit, amount, points) in
            [(egui::MouseWheelUnit::Point, 120.0, 120.0), (egui::MouseWheelUnit::Line, 1.0, line), (egui::MouseWheelUnit::Page, 0.1, 100.0)]
        {
            let height = h.app.cam.half_height();
            h.app.synthetic.push(egui::Event::MouseWheel {
                unit,
                delta: vec2(0.0, amount),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            });
            h.frame();
            let expected = height * (-f64::from(points) * 0.0015).exp();
            assert!((h.app.cam.half_height() - expected).abs() < 1e-6);
            let after = h.app.cam;
            h.frames(30);
            assert_eq!(h.app.cam, after, "idle frames must not replay wheel input");
        }
        assert_eq!(h.app.cam.target, before.target);
        h.app.preferences.zoom_reverse = true;
        let height = h.app.cam.half_height();
        h.call("ui.scroll", json!({"x":800,"y":500,"delta":120}));
        assert!((h.app.cam.half_height() - height * 0.18_f64.exp()).abs() < 1e-6);
    }

    #[test]
    fn wheel_zoom_setting_changes_do_not_replay_smoothing() {
        let mut h = crate::scenario::Harness::new();
        h.app.preferences.zoom_to_cursor = false;
        h.call("ui.move", json!({"x":800,"y":500}));
        h.app.synthetic.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 120.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        h.frame();
        assert!(h.ctx.input(|i| i.is_scrolling()));
        h.app.preferences.smooth_wheel_zoom = false;
        let stopped = h.app.cam;
        h.frame();
        assert_eq!(h.app.cam, stopped);
        h.app.preferences.smooth_wheel_zoom = true;
        h.frames(90);
        assert_eq!(h.app.cam, stopped, "re-enabling must not replay the old smoothing tail");
        assert!(!h.app.viewport.wheel_zoom_raw_until_idle);
        h.app.preferences.smooth_wheel_zoom = false;
        for _ in 0..3 {
            h.app.synthetic.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 120.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            });
            h.frame();
        }
        assert!((h.app.cam.half_height() - stopped.half_height() * (-0.54_f64).exp()).abs() < 1e-6);
        let after = h.app.cam;
        h.frames(90);
        assert_eq!(h.app.cam, after);
    }

    #[test]
    fn shortcuts_window_wheel_does_not_zoom_the_viewport() {
        for smooth in [true, false] {
            let ctx = egui::Context::default();
            let mut app = SolveApp::new(solvecraft_engine::Session::default(), crate::Services::default());
            app.preferences.smooth_wheel_zoom = smooth;
            app.viewport.no_pixels = true;
            crate::help::run(&mut app, &ctx, "shortcuts");
            let mut time = 0.0;
            let mut frame = |app: &mut SolveApp, pointer: Option<Pos2>, wheel: f32| {
                time += 1.0 / 60.0;
                let mut events = Vec::new();
                if let Some(p) = pointer {
                    events.push(egui::Event::PointerMoved(p));
                }
                if wheel != 0.0 {
                    events.push(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: vec2(0.0, wheel),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                    });
                }
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 800.0))),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| show(app, ui));
                        crate::prefs::show(app, ui.ctx());
                    },
                );
                output.textures_delta.clear();
            };
            // Let egui establish the floating window's size and hit-test widgets.
            for _ in 0..4 {
                frame(&mut app, None, 0.0);
            }
            let window = ctx.memory(|m| m.area_rect(egui::Id::new("sc_prefs"))).unwrap();
            let list = window.center() + vec2(140.0, 0.0);
            let height = app.cam.half_height();
            let target = app.cam.target;
            frame(&mut app, Some(list), 0.0);
            for n in 0..12 {
                frame(&mut app, Some(list), if n == 0 { -120.0 } else { 0.0 });
                assert_eq!(app.cam.half_height(), height, "wheel input over shortcuts must stay in the popup");
                assert_eq!(app.cam.target, target);
            }
            // The window's non-scrolling sidebar must also shield the model.
            frame(&mut app, Some(window.left_center() + vec2(30.0, 0.0)), -120.0);
            assert_eq!(app.cam.half_height(), height);
            // An exposed part of the viewport still accepts wheel navigation with the popup open.
            let exposed = pos2(40.0, 700.0);
            assert!(!window.contains(exposed));
            frame(&mut app, Some(exposed), 0.0);
            frame(&mut app, Some(exposed), -120.0);
            assert_ne!(app.cam.half_height(), height);
            // Closing the popup returns its former area to the viewport.
            app.prefs_window.open = false;
            for _ in 0..12 {
                frame(&mut app, Some(list), 0.0);
            }
            let height = app.cam.half_height();
            frame(&mut app, Some(list), -120.0);
            assert_ne!(app.cam.half_height(), height);
        }
    }

    #[test]
    fn floating_dialog_blocks_viewport_hover_and_clicks() {
        let mut h = crate::scenario::Harness::new();
        h.call("engine.execute", json!({"command": "solid.box", "params": {"length": 40, "width": 30, "height": 20}}));
        h.call("ui.view", json!({"view": "fit"}));
        h.app.ui.pick_bodies = true;
        let p = h.app.viewport.rect.unwrap().center();
        h.call("ui.move", json!({"x": p.x, "y": p.y}));
        assert!(h.app.viewport.hover.is_some(), "the test point must hover over the model");
        h.call("ui.click", json!({"x": p.x, "y": p.y}));
        assert!(!h.app.session.selection.is_empty(), "the uncovered model must be selectable");
        h.app.session.selection.clear();

        h.call("ui.prefs", json!({"open": "Navigation"}));
        let window = h.ctx.memory(|m| m.area_rect(egui::Id::new("sc_prefs"))).unwrap();
        assert!(window.contains(p), "the dialog must cover the model at the test point");
        h.call("ui.move", json!({"x": p.x, "y": p.y}));
        assert!(h.app.viewport.hover.is_none(), "the dialog must block model highlights");
        assert!(h.app.viewport.mouse.is_none());
        h.call("ui.click", json!({"x": p.x, "y": p.y}));
        assert!(h.app.session.selection.is_empty(), "clicks in the dialog must not select the model");

        h.call("ui.prefs", json!({"button": "cancel"}));
        h.call("ui.move", json!({"x": p.x, "y": p.y}));
        assert!(h.app.viewport.hover.is_some(), "closing the dialog must restore model highlights");
        h.call("ui.click", json!({"x": p.x, "y": p.y}));
        assert!(!h.app.session.selection.is_empty());
    }

    #[test]
    fn floating_dialog_blocks_axis_view_highlights_and_clicks() {
        let mut h = crate::scenario::Harness::new();
        h.call("ui.resize", json!({"width": 800, "height": 700}));
        let p = crate::scenario::handle_at("axis_view:z").unwrap();
        h.call("ui.move", json!({"x": p.x, "y": p.y}));
        assert_eq!(crate::scenario::count_of("axis_view_highlights"), Some(1.0));
        h.call("ui.prefs", json!({"open": "Navigation"}));
        let window = h.ctx.memory(|m| m.area_rect(egui::Id::new("sc_prefs"))).unwrap();
        assert!(window.contains(p), "the floating dialog must cover the axis target");
        let camera = serde_json::to_value(h.app.cam).unwrap();
        h.call("ui.move", json!({"x": p.x, "y": p.y}));
        assert_eq!(crate::scenario::count_of("axis_view_highlights"), Some(0.0));
        h.call("ui.click", json!({"x": p.x, "y": p.y}));
        assert_eq!(serde_json::to_value(h.app.cam).unwrap(), camera, "dialog clicks must not turn the axis view");
        h.call("ui.drag", json!({"x0": p.x, "y0": p.y, "x1": p.x + 20.0, "y1": p.y, "steps": 8}));
        assert_eq!(serde_json::to_value(h.app.cam).unwrap(), camera, "dialog drags must not orbit the axis gizmo");
        assert!(h.app.viewport.axis_drag.is_none());
        h.call("ui.prefs", json!({"button": "cancel"}));
        h.call("ui.move", json!({"x": p.x, "y": p.y}));
        assert_eq!(crate::scenario::count_of("axis_view_highlights"), Some(1.0));
        h.call("ui.click", json!({"x": p.x, "y": p.y}));
        assert_ne!(serde_json::to_value(h.app.cam).unwrap(), camera);
    }

    #[test]
    fn view_axis_drag_owns_navigation_and_keeps_the_view_centre() {
        for nav in [None, Some(NavMode::Orbit), Some(NavMode::Pan)] {
            let mut h = crate::scenario::Harness::new();
            h.call("ui.view", json!({"view": "front"}));
            h.app.viewport.nav = nav;
            let before = h.app.cam;
            let p = crate::scenario::handle_at("axis_view:-y").unwrap();
            h.call("ui.drag", json!({"x0": p.x, "y0": p.y, "x1": p.x + 48.0, "y1": p.y, "steps": 8}));
            assert!((h.app.cam.yaw - 48.0 * 0.008).abs() < 1e-6, "{nav:?}: axis drag must orbit exactly once");
            assert_eq!(h.app.cam.target, before.target, "{nav:?}: axis drag must not pan");
            assert_eq!(h.app.cam.distance, before.distance);
            assert_eq!(h.app.cam.pitch, before.pitch);
            assert!(h.app.viewport.axis_drag.is_none(), "release clears the axis gesture");
            assert!(h.app.viewport.boxsel.is_none());
        }
    }

    #[test]
    fn section_outlines_follow_the_cut() {
        let mut s = solvecraft_engine::Session::default();
        s.execute("solid.box", &json!({"length": 40, "width": 30, "height": 20})).unwrap();
        let st = s.model.state();
        let m = st.bodies[0].mesh();
        let (o, n) = (Vec3::new(0.0, 15.0, 0.0), Vec3::new(0.0, -1.0, 0.0));
        let segs = solvecraft_engine::view::section_segments(&m, o, n);
        assert!(segs.len() >= 4, "{segs:?}");
        // On the plane (a hair to the kept side) and around the 40 x 20 cut.
        let mut len = 0.0;
        for (a, b) in &segs {
            assert!((a.y - 15.0).abs() < 0.01 && (b.y - 15.0).abs() < 0.01);
            len += a.dist(*b);
        }
        assert!((len - 120.0).abs() < 0.5, "outline length {len}");
        // A plane that misses the body cuts nothing.
        assert!(solvecraft_engine::view::section_segments(&m, Vec3::new(0.0, 99.0, 0.0), n).is_empty());
    }

    #[test]
    fn selection_shows_where_the_model_hides_it() {
        let mut s = solvecraft_engine::Session::default();
        s.execute("solid.box", &json!({"length": 40, "width": 30, "height": 20})).unwrap();
        let mut app = SolveApp::new(s, crate::Services::default());
        assert!(build_highlight(&app).hidden.is_empty());
        // The back face: its outline is drawn (dimmed) where the box hides it.
        app.session.selection = vec![Sel::Face { body: "Body1".into(), index: 3, point: Vec3::new(20.0, 30.0, 10.0) }];
        assert!(!build_highlight(&app).hidden.is_empty());
    }

    #[test]
    fn axis_view_top_bottom_choose_nearest_square_heading() {
        for (from, to) in [
            (0.0, 0.0),
            (30.0, 0.0),
            (-80.0, -90.0),
            (100.0, 90.0),
            (170.0, 180.0),
            (400.0, 360.0),
            (-400.0, -360.0),
            (-45.0, 0.0),
            (45.0, 0.0),
            (135.0, 90.0),
            (-135.0, -90.0),
            (44.99, 0.0),
            (45.01, 90.0),
            (-44.99, 0.0),
            (-45.01, -90.0),
        ] {
            let got = nearest_quarter_turn(f64::to_radians(from)).to_degrees();
            assert!((got - to).abs() < 1e-9, "{from}° → {got}°, expected {to}°");
        }
        assert_eq!(nearest_quarter_turn(f64::NAN), 0.0);
        assert_eq!(nearest_quarter_turn(f64::INFINITY), 0.0);
    }

    #[test]
    fn shift_orbit_accumulates_and_returns_to_free_rotation() {
        let mut h = crate::scenario::Harness::new();
        h.call("ui.view", json!({"view":"front"}));
        let rect = h.app.viewport.rect.unwrap();
        let before = h.app.cam;
        for _ in 0..100 {
            orbit_drag(&mut h.app, rect, 2.0, 0.0, None, true, false);
            step_orbit_snap(&mut h.app, rect, 1.0 / 60.0);
        }
        for _ in 0..120 {
            step_orbit_snap(&mut h.app, rect, 1.0 / 60.0);
        }
        assert!((h.app.cam.yaw - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(h.app.cam.target, before.target);
        assert_eq!(h.app.cam.distance, before.distance);
        orbit_drag(&mut h.app, rect, 0.0, 0.0, None, false, false);
        h.settle();
        assert!((h.app.cam.yaw - 1.6).abs() < 1e-6);
        assert!(h.app.viewport.orbit_free.is_none());
    }

    #[test]
    fn axis_snap_selects_all_directions_in_both_up_frames() {
        assert_eq!(snap_directions().count(), 26);
        for up in [solvecraft_engine::render::UpAxis::Z, solvecraft_engine::render::UpAxis::Y] {
            for d in snap_directions() {
                let mut cam = Camera { up, ..Camera::default() };
                cam = cam.looking_from(cam.from_frame(d + Vec3::new(0.05, 0.03, 0.02)));
                let to = snapped_axis_camera(cam);
                assert!(to.back().dist(cam.from_frame(d)) < 1e-6);
                assert_eq!(to.target, cam.target);
                assert_eq!(to.distance, cam.distance);
                assert_eq!(to.fov, cam.fov);
            }
        }
    }

    #[test]
    fn camera_resets_cancel_pending_axis_motion() {
        for reset in 0..3 {
            let mut h = crate::scenario::Harness::new();
            h.app.run("solid.box", json!({"length":10,"width":10,"height":10})).unwrap();
            let rect = h.app.viewport.rect.unwrap();
            orbit_drag(&mut h.app, rect, 120.0, 40.0, None, true, false);
            assert!(h.app.viewport.orbit_snap.is_some());
            match reset {
                0 => h.app.fit_view(),
                1 => h.app.set_up_axis(solvecraft_engine::render::UpAxis::Y),
                _ => crate::documents::new_design(&mut h.app),
            }
            let before = h.app.cam;
            step_orbit_snap(&mut h.app, rect, 1.0 / 60.0);
            assert_eq!(h.app.cam.yaw, before.yaw);
            assert_eq!(h.app.cam.pitch, before.pitch);
            assert_eq!(h.app.cam.target, before.target);
            assert!(h.app.viewport.orbit_snap.is_none());
            assert!(h.app.viewport.orbit_free.is_none());
        }
    }

    #[test]
    fn axis_snap_spring_eases_in_and_settles_at_each_frame_rate() {
        for fps in [30, 60, 120] {
            let mut h = crate::scenario::Harness::new();
            h.call("ui.view", json!({"view":"front"}));
            let rect = h.app.viewport.rect.unwrap();
            orbit_drag(&mut h.app, rect, 120.0, 0.0, None, true, false);
            assert_eq!(h.app.cam.yaw, 0.0, "choosing a diagonal must not jump the camera");
            step_orbit_snap(&mut h.app, rect, 1.0 / f64::from(fps));
            assert!(h.app.cam.yaw > 0.0 && h.app.cam.yaw < std::f64::consts::FRAC_PI_4 * 0.25);
            for _ in 0..fps * 2 {
                step_orbit_snap(&mut h.app, rect, 1.0 / f64::from(fps));
                assert!(h.app.cam.yaw <= std::f64::consts::FRAC_PI_4 + 0.03, "elastic finish stays small");
            }
            assert!((h.app.cam.yaw - std::f64::consts::FRAC_PI_4).abs() < 1e-6);
            assert!(h.app.viewport.orbit_snap.is_none());
        }
    }

    #[test]
    fn shift_orbit_keeps_cursor_pivot_and_default_middle_pan() {
        let mut h = crate::scenario::Harness::new();
        h.app.preferences.orbit_cursor = true;
        h.app.run("solid.box", json!({"length":40,"width":40,"height":40})).unwrap();
        h.app.fit_view();
        let rect = h.app.viewport.rect.unwrap();
        let at = rect.center() + vec2(50.0, -20.0);
        orbit_drag(&mut h.app, rect, 0.0, 0.0, Some(at), false, true);
        let pivot = h.app.viewport.orbit_pivot.unwrap();
        let before = projection(&h.app, rect).to_screen(pivot).unwrap();
        for _ in 0..20 {
            orbit_drag(&mut h.app, rect, 4.0, 2.0, Some(at), true, true);
            step_orbit_snap(&mut h.app, rect, 1.0 / 60.0);
            let after = projection(&h.app, rect).to_screen(pivot).unwrap();
            assert!(after.distance(before) < 2.0);
        }
        h.app.viewport.orbit_free = None;
        h.app.viewport.orbit_pivot = None;
        h.call("ui.view", json!({"view":"front"}));
        let before = h.app.cam;
        h.call("ui.drag", json!({"x0":800,"y0":500,"x1":1020,"y1":500,"button":"middle","shift":true,"steps":40}));
        assert!((h.app.cam.yaw - 1.76).abs() < 1e-6, "before={before:?}, after={:?}", h.app.cam);
        assert!(h.app.viewport.orbit_snap.is_none());
        h.call("ui.view", json!({"view":"front"}));
        h.call("ui.drag", json!({"x0":800,"y0":500,"x1":850,"y1":520,"button":"middle"}));
        assert_eq!(h.app.cam.yaw, before.yaw);
        assert_eq!(h.app.cam.pitch, before.pitch);
        assert_ne!(h.app.cam.target, before.target);
    }

    #[test]
    fn preset_shift_middle_orbit_is_free_but_shift_right_orbit_snaps() {
        for preset in ["fusion", "inventor"] {
            let mut h = crate::scenario::Harness::new();
            h.app.preferences.nav = preset.into();
            h.call("ui.view", json!({"view":"front"}));
            let before = h.app.cam;
            h.call("ui.drag", json!({"x0":800,"y0":500,"x1":850,"y1":520,"button":"middle","shift":true,"steps":20}));
            let mut expected = before;
            expected.orbit(50.0, 20.0);
            assert!((h.app.cam.yaw - expected.yaw).abs() < 1e-6, "{preset}: {:?}", h.app.cam);
            assert!((h.app.cam.pitch - expected.pitch).abs() < 1e-6, "{preset}: {:?}", h.app.cam);
            assert_eq!(h.app.cam.target, before.target);
            assert_eq!(h.app.cam.distance, before.distance);
            assert!(h.app.viewport.orbit_snap.is_none());
            h.call("ui.view", json!({"view":"front"}));
            h.call("ui.drag", json!({"x0":800,"y0":500,"x1":1020,"y1":500,"button":"right","shift":true,"steps":40}));
            assert!((h.app.cam.yaw - std::f64::consts::FRAC_PI_2).abs() < 1e-6, "{preset}: {:?}", h.app.cam);
        }
    }

    #[test]
    fn axis_view_click_again_flips_and_drag_snaps() {
        let mut h = crate::scenario::Harness::new();
        h.run(&json!([
            {"call":"ui.view","params":{"view":"home"}},
            {"click":{"handle":"axis_view:z"}},
            {"expect":{"camera":{"pitch_deg":90,"yaw_deg":0}}},
            {"click":{"handle":"axis_view:z"}},
            {"expect":{"camera":{"pitch_deg":-90,"yaw_deg":0}}},
            {"call":"ui.view","params":{"view":"front"}}
        ]))
        .unwrap();
        let c = axis_center(&h.app, h.app.viewport.rect.unwrap());
        h.call("ui.drag", json!({"x0":c.x,"y0":c.y,"x1":c.x+220.0,"y1":c.y,"shift":true,"steps":40}));
        assert!((h.app.cam.yaw - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
        assert!(h.app.viewport.axis_drag.is_none());
        assert!(h.app.viewport.orbit_free.is_none());
        assert!(h.app.viewport.boxsel.is_none());
    }

    #[test]
    fn construction_axes_span_the_model_and_points_show() {
        let mut s = solvecraft_engine::Session::default();
        s.execute("solid.box", &json!({"length": 40, "width": 30, "height": 20})).unwrap();
        s.execute("construct.axis.two_points", &json!({"a": {"vertex": [0, 0, 20]}, "b": {"vertex": [40, 30, 20]}})).unwrap();
        s.execute("construct.point.vertex", &json!({"point": {"vertex": [40, 0, 20]}})).unwrap();
        let mut app = SolveApp::new(s, crate::Services::default());
        let axes = construction_axis_segments(&app);
        assert_eq!(axes.len(), 1);
        let (name, a, b) = axes[0].clone();
        // The segment runs through both points and past them.
        let d = (b - a).normalized().unwrap();
        for p in [Vec3::new(0.0, 0.0, 20.0), Vec3::new(40.0, 30.0, 20.0)] {
            let t = (p - a).dot(d);
            assert!(t > 0.0 && t < a.dist(b) && (a + d * t).dist(p) < 1e-6, "{p:?} not on {a:?}..{b:?}");
        }
        assert_eq!(construction_points(&app).len(), 1);
        // It is an axis for dialogs (pattern directions, revolve axes), and hiding it hides it.
        let (o, dir) = crate::dialogs::axis_of(&app, &Sel::Axis { name: name.clone() }).unwrap();
        assert!(o.dist(Vec3::new(0.0, 0.0, 20.0)) < 1e-6 && dir.dist(d) < 1e-6);
        app.session.visibility.hidden_origin.push(name);
        assert!(construction_axis_segments(&app).is_empty());
    }
}
