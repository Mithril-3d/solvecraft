//! The pick buffer: which model edges are visible at each pixel of the viewport, for picking on a
//! large model without testing every edge. Surfaces are drawn into the depth buffer only; then
//! every edge segment is drawn as a line carrying its id, depth tested against them (lifted
//! toward the eye by the picker's slack, the depth by which an edge may lie behind a face and
//! still count as visible) and written into an `R32Uint` id texture (0: no edge).
//!
//! The pass is drawn once per view (camera, viewport size, model), and only small tiles around
//! the cursor are read back, so a pointer moving over a still view reads a few kilobytes and
//! draws nothing. Read-backs arrive a frame or two later ([`PickResults`]); until a tile is in,
//! the picker answers on the CPU. The ids are hints: the picker still runs its exact tests on
//! the edges a tile names.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use wgpu::util::DeviceExt;

/// The side of a read-back tile, in pixels (a multiple of 64, so rows need no padding).
pub const TILE: u32 = 128;
/// Most tiles kept from one view.
const KEEP: usize = 6;
/// Most read-backs in flight.
const IN_FLIGHT: usize = 3;
/// Bytes per surface vertex (position) and per edge vertex (position, id).
const FACE_VERTEX: u64 = 12;
const EDGE_VERTEX: u64 = 16;
const UNIFORM_SIZE: u64 = 96;

const SHADER: &str = r#"
struct U {
    vp: mat4x4<f32>,
    eye: vec4<f32>,  // eye position, w = 1 for perspective
    back: vec4<f32>, // toward the eye (orthographic), w = how far edges are lifted toward it
};
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs_face(@location(0) p: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.vp * vec4<f32>(p, 1.0);
}

@fragment
fn fs_face() -> @location(0) u32 {
    return 0u;
}

struct EOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) id: u32,
};

@vertex
fn vs_edge(@location(0) p: vec3<f32>, @location(1) id: u32) -> EOut {
    var toward = u.back.xyz;
    if (u.eye.w > 0.5) {
        let v = u.eye.xyz - p;
        let l = length(v);
        if (l > 0.0) {
            toward = v / l;
        }
    }
    var o: EOut;
    o.pos = u.vp * vec4<f32>(p + toward * u.back.w, 1.0);
    o.id = id;
    return o;
}

@fragment
fn fs_edge(i: EOut) -> @location(0) u32 {
    return i.id;
}
"#;

/// What the pass draws: surfaces (for depth) and edge segments with their ids (from 1).
#[derive(Default, Clone)]
pub struct PickScene {
    faces: Vec<u8>,
    edges: Vec<u8>,
}

impl PickScene {
    pub fn triangle(&mut self, t: [[f32; 3]; 3]) {
        for p in t {
            for c in p {
                self.faces.extend_from_slice(&c.to_le_bytes());
            }
        }
    }

    /// Segment `a`–`b` of the edge `id` (0 is no edge: not drawn).
    pub fn segment(&mut self, a: [f32; 3], b: [f32; 3], id: u32) {
        if id == 0 {
            return;
        }
        for p in [a, b] {
            for c in p {
                self.edges.extend_from_slice(&c.to_le_bytes());
            }
            self.edges.extend_from_slice(&id.to_le_bytes());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

/// The view the pass is drawn for.
#[derive(Clone, Copy, Debug)]
pub struct PickView {
    /// Changes whenever anything below or the scene changes.
    pub key: u64,
    /// Column-major view-projection (as [`crate::view::Frame::view_proj`]).
    pub view_proj: [[f32; 4]; 4],
    /// Eye position, and 1 for a perspective camera (0: orthographic).
    pub eye: [f32; 4],
    /// Direction toward the eye (orthographic).
    pub back: [f32; 3],
    /// How far edges are lifted toward the eye (model units).
    pub lift: f32,
    /// The viewport in physical pixels.
    pub size: [u32; 2],
}

/// Edge ids of a block of pixels of one view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickTile {
    pub key: u64,
    pub origin: [u32; 2],
    pub size: [u32; 2],
    /// Row-major, `size[0] * size[1]`.
    pub ids: Vec<u32>,
}

impl PickTile {
    /// Whether the tile is of view `key` and holds every pixel of `[x0, y0, x1, y1]` (inclusive)
    /// that lies inside a viewport of `view` pixels.
    pub fn covers(&self, key: u64, view: [u32; 2], [x0, y0, x1, y1]: [i64; 4]) -> bool {
        let clamp = |v: i64, n: u32| v.clamp(0, i64::from(n.max(1)) - 1);
        let (x0, x1, y0, y1) = (clamp(x0, view[0]), clamp(x1, view[0]), clamp(y0, view[1]), clamp(y1, view[1]));
        let [ox, oy] = self.origin.map(i64::from);
        let [w, h] = self.size.map(i64::from);
        self.key == key && x0 >= ox && y0 >= oy && x1 < ox + w && y1 < oy + h
    }

    /// The ids within `[x0, y0, x1, y1]` (inclusive; outside the tile: none), each once.
    pub fn ids_in(&self, [x0, y0, x1, y1]: [i64; 4]) -> Vec<u32> {
        let [ox, oy] = self.origin.map(i64::from);
        let [w, h] = self.size.map(i64::from);
        let mut out = Vec::new();
        for y in y0.max(oy)..=y1.min(oy + h - 1) {
            for x in x0.max(ox)..=x1.min(ox + w - 1) {
                if let Some(&id) = self.ids.get(((y - oy) * w + (x - ox)) as usize)
                    && id != 0
                {
                    out.push(id);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The tiles read back so far (of the latest view only), newest last.
#[derive(Default, Debug)]
pub struct PickResults {
    pub tiles: Vec<PickTile>,
}

impl PickResults {
    /// The newest tile of view `key` covering `window`.
    pub fn tile(&self, key: u64, view: [u32; 2], window: [i64; 4]) -> Option<&PickTile> {
        self.tiles.iter().rev().find(|t| t.covers(key, view, window))
    }
}

pub type PickOut = Arc<Mutex<PickResults>>;

/// The tile (origin and size) around pixel `centre` of a viewport of `view` pixels.
pub fn tile_around(centre: [u32; 2], view: [u32; 2]) -> ([u32; 2], [u32; 2]) {
    let size = [TILE.min(view[0]), TILE.min(view[1])];
    let origin = [0, 1].map(|k| centre[k].saturating_sub(size[k] / 2).min(view[k].saturating_sub(size[k])));
    (origin, size)
}

const NEW: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

/// A tile being copied out of the id texture.
struct Pending {
    key: u64,
    origin: [u32; 2],
    size: [u32; 2],
    bytes_per_row: u32,
    buffer: wgpu::Buffer,
    /// The frame whose commands copy it (it is mapped once they are submitted).
    frame: u64,
    state: Arc<AtomicU8>,
}

struct Target {
    size: [u32; 2],
    ids: wgpu::Texture,
    ids_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

struct Uploaded {
    key: u64,
    faces: Option<(wgpu::Buffer, u32)>,
    edges: Option<(wgpu::Buffer, u32)>,
}

/// The pass's pipelines, its textures and the read-backs in flight, on one device.
pub struct PickBuffer {
    face: wgpu::RenderPipeline,
    edge: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    scene: Option<Uploaded>,
    target: Option<Target>,
    /// The view drawn into the textures.
    drawn: Option<u64>,
    pending: Vec<Pending>,
    frame: u64,
}

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const IDS: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

impl PickBuffer {
    pub fn new(device: &wgpu::Device) -> PickBuffer {
        let module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sc_pick"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sc_pick"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sc_pick"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sc_pick_u"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sc_pick"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        const FA: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
        const EA: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32];
        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        stride: u64,
                        attributes: &'static [wgpu::VertexAttribute],
                        topology: wgpu::PrimitiveTopology,
                        write: bool,
                        mask: wgpu::ColorWrites| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride, step_mode: wgpu::VertexStepMode::Vertex, attributes })],
                    compilation_options: Default::default(),
                },
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: IDS, blend: None, write_mask: mask })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let face =
            pipeline("sc_pick_face", "vs_face", "fs_face", FACE_VERTEX, &FA, wgpu::PrimitiveTopology::TriangleList, true, wgpu::ColorWrites::empty());
        let edge = pipeline("sc_pick_edge", "vs_edge", "fs_edge", EDGE_VERTEX, &EA, wgpu::PrimitiveTopology::LineList, false, wgpu::ColorWrites::ALL);
        PickBuffer { face, edge, uniform, bind, scene: None, target: None, drawn: None, pending: Vec::new(), frame: 0 }
    }

    /// Before the frame is submitted: collect the tiles read back, then draw `view` of `scene`
    /// (`scene_key` names it; it is uploaded when that changes) unless it is drawn already, and
    /// start reading back the tile around pixel `centre` unless it is in or on its way. The tiles
    /// land in `out` a frame or two later; keep frames coming until then.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene_key: u64,
        scene: &PickScene,
        view: &PickView,
        centre: Option<[u32; 2]>,
        out: &PickOut,
    ) {
        self.frame += 1;
        self.collect(device, out);
        let [w, h] = view.size;
        let max = device.limits().max_texture_dimension_2d;
        if w == 0 || h == 0 || w > max || h > max {
            return;
        }
        if self.scene.as_ref().is_none_or(|s| s.key != scene_key) {
            let limit = device.limits().max_buffer_size;
            let upload = |label: &str, bytes: &[u8], stride: u64| {
                let n = bytes.len() as u64 / stride;
                (n > 0 && (bytes.len() as u64) <= limit && n <= u64::from(u32::MAX)).then(|| {
                    (
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(label),
                            contents: bytes,
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                        n as u32,
                    )
                })
            };
            self.scene = Some(Uploaded {
                key: scene_key,
                faces: upload("sc_pick_faces", &scene.faces, FACE_VERTEX),
                edges: upload("sc_pick_edges", &scene.edges, EDGE_VERTEX),
            });
            self.drawn = None;
        }
        if self.target.as_ref().is_none_or(|t| t.size != view.size) {
            let texture = |label: &str, format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let ids = texture("sc_pick_ids", IDS, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
            let depth = texture("sc_pick_depth", DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT);
            let (ids_view, depth_view) = (ids.create_view(&Default::default()), depth.create_view(&Default::default()));
            self.target = Some(Target { size: view.size, ids, ids_view, depth_view });
            self.drawn = None;
        }
        let (Some(target), Some(scene)) = (&self.target, &self.scene) else { return };
        if self.drawn != Some(view.key) {
            queue.write_buffer(&self.uniform, 0, &uniform_bytes(view));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sc_pick"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.ids_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind, &[]);
            for (pipeline, part) in [(&self.face, &scene.faces), (&self.edge, &scene.edges)] {
                if let Some((buffer, n)) = part {
                    pass.set_pipeline(pipeline);
                    pass.set_vertex_buffer(0, buffer.slice(..));
                    pass.draw(0..*n, 0..1);
                }
            }
            drop(pass);
            self.drawn = Some(view.key);
            // Tiles of other views no longer answer.
            self.pending.retain(|p| p.key == view.key);
            if let Ok(mut o) = out.lock() {
                o.tiles.retain(|t| t.key == view.key);
            }
        }
        let Some(centre) = centre else { return };
        let (origin, size) = tile_around(centre, view.size);
        let known = out.lock().is_ok_and(|o| o.tiles.iter().any(|t| t.key == view.key && t.origin == origin && t.size == size))
            || self.pending.iter().any(|p| p.key == view.key && p.origin == origin && p.size == size);
        if known || self.pending.len() >= IN_FLIGHT {
            return;
        }
        let bytes_per_row = (size[0] * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sc_pick_tile"),
            size: u64::from(bytes_per_row) * u64::from(size[1]),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.ids,
                mip_level: 0,
                origin: wgpu::Origin3d { x: origin[0], y: origin[1], z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: Some(size[1]) },
            },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        self.pending.push(Pending { key: view.key, origin, size, bytes_per_row, buffer, frame: self.frame, state: Arc::new(AtomicU8::new(NEW)) });
    }

    /// Whether read-backs are still on their way (keep frames coming).
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Maps the copies earlier frames submitted and moves the mapped ones into `out`.
    fn collect(&mut self, device: &wgpu::Device, out: &PickOut) {
        let frame = self.frame;
        for p in &self.pending {
            if p.frame < frame && p.state.load(Ordering::Acquire) == NEW {
                p.state.store(MAPPING, Ordering::Release);
                let state = p.state.clone();
                p.buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| state.store(if r.is_ok() { READY } else { FAILED }, Ordering::Release));
            }
        }
        if self.pending.iter().any(|p| p.state.load(Ordering::Acquire) == MAPPING) {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        let mut done = Vec::new();
        self.pending.retain(|p| match p.state.load(Ordering::Acquire) {
            READY => {
                if let Ok(bytes) = p.buffer.slice(..).get_mapped_range() {
                    let [w, h] = p.size;
                    let mut ids = Vec::with_capacity(w as usize * h as usize);
                    for row in bytes.chunks(p.bytes_per_row as usize).take(h as usize) {
                        ids.extend(row.as_chunks::<4>().0.iter().take(w as usize).map(|c| u32::from_le_bytes(*c)));
                    }
                    done.push(PickTile { key: p.key, origin: p.origin, size: p.size, ids });
                }
                p.buffer.unmap();
                false
            }
            FAILED => false,
            _ => true,
        });
        if !done.is_empty()
            && let Ok(mut o) = out.lock()
        {
            for t in done {
                if self.drawn == Some(t.key) && t.ids.len() == t.size[0] as usize * t.size[1] as usize {
                    o.tiles.retain(|x| x.key == t.key);
                    o.tiles.push(t);
                }
            }
            let excess = o.tiles.len().saturating_sub(KEEP);
            o.tiles.drain(..excess);
        }
    }
}

fn uniform_bytes(v: &PickView) -> Vec<u8> {
    let mut b = Vec::with_capacity(UNIFORM_SIZE as usize);
    for col in v.view_proj {
        for x in col {
            b.extend_from_slice(&x.to_le_bytes());
        }
    }
    for x in v.eye.into_iter().chain(v.back).chain([v.lift]) {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

#[cfg(test)]
#[path = "pick_tests.rs"]
mod tests;
