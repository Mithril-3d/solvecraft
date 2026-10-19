//! How headless pictures of the model are drawn: a renderer the host installs. The engine
//! describes what a picture shows ([`crate::view::scene`]); drawing it is a GPU job the engine
//! cannot do itself (the GPU crate sits above it), so the application, the CLI and the MCP
//! server install one ([`install`]) that draws with the viewport's own pipelines (or, as a last
//! resort the host chooses, the CPU rasterizer). Every picture names the renderer that drew it.
//! Without a renderer, a picture is an error and a save simply has no thumbnail.

use std::sync::{Arc, RwLock};

use solidcraft_render::{Camera, Scene};

/// A picture: straight RGBA, rows top to bottom, and the renderer that drew it (`gpu`,
/// `gpu-software` or `cpu`).
pub struct Picture {
    pub rgba: Vec<u8>,
    pub renderer: &'static str,
}

/// A picture as PNG bytes, and the renderer that drew it.
pub struct Png {
    pub bytes: Vec<u8>,
    pub renderer: &'static str,
}

/// What a reply says when the CPU rasterizer drew the picture.
pub const CPU_WARNING: &str = "drawn by the CPU rasterizer (no GPU adapter qualified, or SOLIDCRAFT_GPU=cpu): surface analysis colours, highlights and legends are not drawn; `solidcraft-cli gpu` lists the adapters";

/// The warning that goes with a picture drawn by `renderer`, if any.
pub fn warning(renderer: &str) -> Option<&'static str> {
    (renderer == "cpu").then_some(CPU_WARNING)
}

/// Draws `scene` seen by `cam` at `width` × `height` pixels (`width * height * 4` bytes of RGBA),
/// or says why it could not.
pub type Renderer = dyn Fn(&Scene, &Camera, u32, u32) -> Result<Picture, String> + Send + Sync;

static RENDERER: RwLock<Option<Arc<Renderer>>> = RwLock::new(None);

/// Make `renderer` draw this process's headless pictures (replacing any installed before).
pub fn install(renderer: Arc<Renderer>) {
    if let Ok(mut r) = RENDERER.write() {
        *r = Some(renderer);
    }
}

/// The installed renderer, if any.
pub fn installed() -> Option<Arc<Renderer>> {
    RENDERER.read().ok().and_then(|r| r.clone())
}

/// `scene` drawn by the installed renderer.
pub fn rgba(scene: &Scene, cam: &Camera, width: u32, height: u32) -> Result<Picture, String> {
    rgba_with(installed().as_deref(), scene, cam, width, height)
}

/// `scene` drawn by the installed renderer, as PNG.
pub fn png(scene: &Scene, cam: &Camera, width: u32, height: u32) -> Result<Png, String> {
    png_with(installed().as_deref(), scene, cam, width, height)
}

/// [`rgba`] with `renderer` instead of the installed one.
pub fn rgba_with(renderer: Option<&Renderer>, scene: &Scene, cam: &Camera, width: u32, height: u32) -> Result<Picture, String> {
    let renderer = renderer.ok_or("no renderer is installed to draw pictures in this process")?;
    let picture = renderer(scene, cam, width, height)?;
    if picture.rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return Err(format!("the renderer returned {} bytes for {width} × {height} pixels", picture.rgba.len()));
    }
    if let Some(w) = warning(picture.renderer) {
        log::warn!("{w}");
    }
    Ok(picture)
}

/// [`png`] with `renderer` instead of the installed one.
pub fn png_with(renderer: Option<&Renderer>, scene: &Scene, cam: &Camera, width: u32, height: u32) -> Result<Png, String> {
    let Picture { rgba, renderer } = rgba_with(renderer, scene, cam, width, height)?;
    let bytes = solidcraft_render::png(width as usize, height as usize, rgba).ok_or_else(|| "PNG encoding failed".to_string())?;
    Ok(Png { bytes, renderer })
}

/// For tests: a stand-in for the renderer the hosts install, a flat picture whose first byte
/// counts the scene's meshes.
#[cfg(test)]
pub(crate) fn install_test_renderer() {
    install(Arc::new(|scene: &Scene, _: &Camera, w: u32, h: u32| {
        let mut rgba = vec![200; (w * h * 4) as usize];
        if let Some(first) = rgba.first_mut() {
            *first = scene.meshes.len() as u8;
        }
        Ok(Picture { rgba, renderer: "gpu" })
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_renderer_a_picture_is_an_error_not_a_crash() {
        let e = png_with(None, &Scene::default(), &Camera::default(), 16, 16).err().unwrap();
        assert!(e.contains("no renderer"), "{e}");
    }

    #[test]
    fn a_renderer_that_returns_the_wrong_size_is_an_error() {
        let short: &Renderer = &|_, _, _, _| Ok(Picture { rgba: vec![0; 3], renderer: "gpu" });
        assert!(rgba_with(Some(short), &Scene::default(), &Camera::default(), 2, 2).is_err());
        let plain: &Renderer = &|_, _, w, h| Ok(Picture { rgba: vec![255; (w * h * 4) as usize], renderer: "cpu" });
        let png = png_with(Some(plain), &Scene::default(), &Camera::default(), 2, 2).unwrap();
        assert!(png.bytes.starts_with(b"\x89PNG"));
        assert_eq!(png.renderer, "cpu");
        assert_eq!((warning("cpu"), warning("gpu"), warning("gpu-software")), (Some(CPU_WARNING), None, None));
    }
}
