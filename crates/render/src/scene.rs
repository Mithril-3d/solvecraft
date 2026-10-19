//! What a picture of the model shows, independent of how it is drawn: meshes in colours, lines,
//! the background, a section cut and a surface analysis. The GPU draws it with the viewport's
//! pipelines (`solidcraft_gpu::capture`).

use std::sync::Arc;

use solidcraft_geom::{Mesh, Vec3};

use crate::SceneAnalysis;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// The colour as 0..1 per channel.
    pub fn f(self) -> [f64; 3] {
        [self.0 as f64 / 255.0, self.1 as f64 / 255.0, self.2 as f64 / 255.0]
    }
}

pub struct SceneMesh {
    pub mesh: Arc<Mesh>,
    pub color: Rgb,
    /// Faces in colours of their own: (B-rep face index, colour).
    pub face_colors: Vec<(u32, Rgb)>,
}

pub struct SceneLine {
    pub points: Vec<Vec3>,
    pub color: Rgb,
    /// Width in output pixels.
    pub width: f64,
    /// Draw over everything (sketch geometry being edited).
    pub on_top: bool,
}

#[derive(Default)]
pub struct Scene {
    pub meshes: Vec<SceneMesh>,
    pub lines: Vec<SceneLine>,
    /// The ground grid: drawn under everything (the model hides it; it never hides the model).
    pub grid: Vec<SceneLine>,
    /// Background gradient top → bottom.
    pub background: Option<(Rgb, Rgb)>,
    /// Scene radius for clipping planes.
    pub radius: f64,
    /// A section cut: (normal, d); what lies where normal·p > d is cut away, and the inside
    /// shows as a hatched cap.
    pub clip: Option<(Vec3, f64)>,
    /// The colour the section cap is tinted toward (default: [`SECTION_CAP`]).
    pub cap: Option<Rgb>,
    /// A surface analysis shading the meshes (zebra, draft, curvature map, ...).
    pub analysis: Option<SceneAnalysis>,
}

/// The section cap's default tint, as in the viewport.
pub const SECTION_CAP: Rgb = Rgb(214, 128, 72);

/// RGBA pixels (`w × h`) to PNG bytes.
pub fn png(w: usize, h: usize, rgba: Vec<u8>) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(u32::try_from(w).ok()?, u32::try_from(h).ok()?, rgba)?;
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}
