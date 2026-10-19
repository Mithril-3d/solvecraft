//! The GPU pick buffer in the viewport ([`solidcraft_gpu::pick`]): which model edges are seen at
//! each pixel, so a pick over a large model tests only the edges near the cursor that are not
//! hidden, instead of every edge the cursor's ray passes near.
//!
//! The pass is drawn for a view once it holds still (the same view two frames running), and a
//! tile around the cursor is read back; the tile arrives a frame or two later, and until it
//! does (and while the view moves) the picker answers from the CPU indexes
//! ([`crate::pick_index`]). The tile only narrows the candidates: the picker's exact tests
//! still decide. Not used for Select Other (every target along the ray), in wireframe (nothing
//! hides an edge there), without a GPU, or for a model past [`MAX_TRIANGLES`].

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use egui::Pos2;
use solidcraft_engine::doc::ModelState;
use solidcraft_gpu::pick::{PickBuffer, PickOut, PickScene, PickView};

use crate::SolveApp;
use crate::viewport::Proj;

/// Most surface triangles the pass draws (beyond them the CPU indexes answer alone).
pub const MAX_TRIANGLES: usize = 6_000_000;
/// How near the cursor an edge's pixels are looked for (points): vertices pick within 7, edges 6.
const WINDOW: f32 = 8.0;
/// Frames kept coming after a tile is asked for, so its read-back lands.
const FRAMES: u8 = 4;

/// The pass's scene for one model state: the ids it draws stand for (body, edge).
struct Scene {
    state: Arc<ModelState>,
    key: u64,
    scene: Arc<PickScene>,
    /// Per id − 1: (body index in the state, edge index).
    edges: Vec<(u32, u32)>,
    /// One-point edges (no line to draw): always candidates.
    points: Vec<(u32, u32)>,
}

#[derive(Default)]
pub(crate) struct State {
    scene: Mutex<Option<Arc<Scene>>>,
    out: PickOut,
    /// A pick found no tile for its cursor: the view it was in and the pixel.
    want: Mutex<Option<(u64, [u32; 2])>>,
    /// The view of the last frame (the pass is drawn for a view that held still).
    last_view: u64,
    /// Frames still to request so read-backs land.
    frames: u8,
}

/// The edges a tile names near the cursor, per body index of the world state.
pub(crate) struct NearEdges {
    by_body: HashMap<usize, Vec<usize>>,
}

impl NearEdges {
    /// Every segment of the named edges of body `index` of `mesh`, as (edge, first point), in
    /// edge order.
    pub(crate) fn segments(&self, index: usize, mesh: &solidcraft_engine::geom::Mesh) -> Vec<(usize, usize)> {
        let mut v = Vec::new();
        for &ei in self.by_body.get(&index).map_or(&[][..], |e| &e[..]) {
            let n = mesh.edges.get(ei).map_or(0, |e| e.len().max(2) - 1);
            v.extend((0..n).map(|si| (ei, si)));
        }
        v
    }
}

/// What the pass's scene depends on: the world state, and which bodies show.
fn model_key(app: &SolveApp, st: &Arc<ModelState>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (Arc::as_ptr(st) as usize).hash(&mut h);
    app.session.revision.hash(&mut h);
    app.session.visibility.hidden_bodies.hash(&mut h);
    app.session.visibility.hidden_occurrences.hash(&mut h);
    h.finish()
}

/// The view's pixels: the viewport's size in physical pixels.
fn view_size(app: &SolveApp, proj: &Proj) -> [u32; 2] {
    let ppp = app.viewport.pixels_per_point();
    [(proj.rect.width() * ppp).round().max(0.0) as u32, (proj.rect.height() * ppp).round().max(0.0) as u32]
}

/// How far edges are lifted toward the eye: the picker's slack for an edge behind a face.
fn lift(app: &SolveApp) -> f64 {
    crate::viewport::scene_radius(app) * 0.02
}

/// Everything the drawn ids depend on.
fn view_key(app: &SolveApp, proj: &Proj, model: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut h);
    for col in proj.vp.to_f32() {
        for x in col {
            x.to_bits().hash(&mut h);
        }
    }
    view_size(app, proj).hash(&mut h);
    lift(app).to_bits().hash(&mut h);
    h.finish()
}

fn usable(app: &SolveApp) -> bool {
    app.viewport.gpu.is_some() && app.ui.visual_style != 2
}

/// The edges seen within reach of `pos`, when a tile of this view is in; otherwise `None`, and
/// a tile around `pos` is asked for.
pub(crate) fn edges_near(app: &SolveApp, proj: &Proj, pos: Pos2, st: &Arc<ModelState>) -> Option<NearEdges> {
    if !usable(app) {
        return None;
    }
    let model = model_key(app, st);
    let scene = app.viewport.pick_buffer.scene.lock().ok()?.clone().filter(|s| s.key == model && Arc::ptr_eq(&s.state, st));
    let key = view_key(app, proj, model);
    let size = view_size(app, proj);
    let ppp = app.viewport.pixels_per_point();
    let (x, y) = ((pos.x - proj.rect.left()) * ppp, (pos.y - proj.rect.top()) * ppp);
    if !(x >= 0.0 && y >= 0.0 && x < size[0] as f32 && y < size[1] as f32) {
        return None;
    }
    let (x, y) = (x as i64, y as i64);
    let r = (WINDOW * ppp).ceil() as i64;
    let window = [x - r, y - r, x + r, y + r];
    let ids = scene.as_ref().and_then(|_| app.viewport.pick_buffer.out.lock().ok()?.tile(key, size, window).map(|t| t.ids_in(window)));
    let (Some(scene), Some(ids)) = (scene, ids) else {
        if let Ok(mut w) = app.viewport.pick_buffer.want.lock() {
            *w = Some((key, [x as u32, y as u32]));
        }
        return None;
    };
    let mut by_body: HashMap<usize, Vec<usize>> = HashMap::new();
    for (b, e) in ids.iter().filter_map(|id| scene.edges.get((*id as usize).checked_sub(1)?)).chain(&scene.points) {
        by_body.entry(*b as usize).or_default().push(*e as usize);
    }
    for v in by_body.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    Some(NearEdges { by_body })
}

/// The pass's scene for the world state: visible bodies' triangles, and their pickable edges.
/// `None` past [`MAX_TRIANGLES`] or without edges.
fn scene(app: &SolveApp, st: &Arc<ModelState>, key: u64) -> Option<Arc<Scene>> {
    let f = |p: solidcraft_engine::geom::Vec3| [p.x as f32, p.y as f32, p.z as f32];
    let mut scene = PickScene::default();
    let (mut edges, mut points, mut triangles) = (Vec::new(), Vec::new(), 0usize);
    for (bi, b) in st.bodies.iter().enumerate() {
        if !app.session.body_visible(&b.name) {
            continue;
        }
        let m = b.mesh();
        triangles += m.triangles.len();
        if triangles > MAX_TRIANGLES {
            return None;
        }
        for t in &m.triangles {
            if let Some([a, b, c]) = m.tri(t) {
                scene.triangle([f(a), f(b), f(c)]);
            }
        }
        for (ei, e) in m.edges.iter().enumerate() {
            if !m.edge_pickable(ei) || edges.len() >= u32::MAX as usize - 1 {
                continue;
            }
            if e.len() == 1 {
                points.push((bi as u32, ei as u32));
                continue;
            }
            edges.push((bi as u32, ei as u32));
            let id = edges.len() as u32;
            for w in e.windows(2) {
                scene.segment(f(w[0]), f(w[1]), id);
            }
        }
    }
    (!scene.is_empty()).then(|| Arc::new(Scene { state: st.clone(), key, scene: Arc::new(scene), edges, points }))
}

/// In the frame's GPU paint: draw the pass for a view that held still when a pick asked for a
/// tile, and keep frames coming until the read-back lands.
pub(crate) fn paint(app: &mut SolveApp, ctx: &egui::Context, painter: &egui::Painter, proj: &Proj) {
    if !usable(app) {
        return;
    }
    let st = app.session.world_state();
    let model = model_key(app, &st);
    let key = view_key(app, proj, model);
    let state = &mut app.viewport.pick_buffer;
    let still = std::mem::replace(&mut state.last_view, key) == key;
    let want = state.want.lock().ok().and_then(|mut w| w.take()).filter(|(k, _)| *k == key && still);
    if want.is_none() && state.frames == 0 {
        return;
    }
    state.frames = if want.is_some() { FRAMES } else { state.frames.saturating_sub(1) };
    let cached = state.scene.lock().ok().and_then(|s| s.clone()).filter(|s| s.key == model && Arc::ptr_eq(&s.state, &st));
    let scene = match cached {
        Some(s) => s,
        None => {
            let Some(s) = scene(app, &st, model) else { return };
            if let Ok(mut slot) = app.viewport.pick_buffer.scene.lock() {
                *slot = Some(s.clone());
            }
            s
        }
    };
    let e = proj.cam.eye().to_f32();
    let view = PickView {
        key,
        view_proj: proj.vp.to_f32(),
        eye: [e[0], e[1], e[2], if proj.cam.fov > 0.0 { 1.0 } else { 0.0 }],
        back: proj.cam.back().to_f32(),
        lift: lift(app) as f32,
        size: view_size(app, proj),
    };
    let cb =
        PickCallback { scene_key: scene.key, scene: scene.scene.clone(), view, centre: want.map(|w| w.1), out: app.viewport.pick_buffer.out.clone() };
    painter.add(egui_wgpu::Callback::new_paint_callback(proj.rect, cb));
    ctx.request_repaint();
}

/// The pass as an egui paint callback: all its work is in `prepare` (it draws into its own
/// textures, not the frame).
struct PickCallback {
    scene_key: u64,
    scene: Arc<PickScene>,
    view: PickView,
    centre: Option<[u32; 2]>,
    out: PickOut,
}

impl egui_wgpu::CallbackTrait for PickCallback {
    fn prepare(
        &self,
        device: &egui_wgpu::wgpu::Device,
        queue: &egui_wgpu::wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut egui_wgpu::wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<egui_wgpu::wgpu::CommandBuffer> {
        if resources.get::<PickBuffer>().is_none() {
            resources.insert(PickBuffer::new(device));
        }
        if let Some(pb) = resources.get_mut::<PickBuffer>() {
            pb.prepare(device, queue, encoder, self.scene_key, &self.scene, &self.view, self.centre, &self.out);
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, _pass: &mut egui_wgpu::wgpu::RenderPass<'static>, _resources: &egui_wgpu::CallbackResources) {}
}

#[cfg(test)]
#[path = "pick_buffer_tests.rs"]
mod tests;
