#![forbid(unsafe_code)]
#[path = "../app_icon.rs"]
mod app_icon;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../app_icon.rs");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS")? != "windows" {
        return Ok(());
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    let sizes = [16usize, 24, 32, 48, 64, 128, 256];
    let mut images = Vec::new();
    for size in sizes {
        let rgba = app_icon::rgba(size)?;
        // ICO DIB: bottom-up BGRA plus a padded one-bit transparency mask.
        let mask_stride = size.div_ceil(32) * 4;
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&(size as i32).to_le_bytes());
        dib.extend_from_slice(&((size * 2) as i32).to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&[0; 24]);
        for row in rgba.chunks_exact(size * 4).rev() {
            for p in row.as_chunks::<4>().0.iter() {
                dib.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        for row in rgba.chunks_exact(size * 4).rev() {
            let mut mask = vec![0u8; mask_stride];
            for (x, p) in row.as_chunks::<4>().0.iter().enumerate() {
                if p[3] == 0 {
                    mask[x / 8] |= 0x80 >> (x % 8);
                }
            }
            dib.extend_from_slice(&mask);
        }
        images.push((size, dib));
    }
    let mut ico = vec![0, 0, 1, 0];
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, data) in &images {
        let dim = if *size == 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for (_, data) in images {
        ico.extend_from_slice(&data);
    }
    let icon = out.join("solidcraft.ico");
    std::fs::write(&icon, ico)?;
    let rc = out.join("solidcraft.rc");
    std::fs::write(&rc, format!("1 ICON \"{}\"\n", icon.display().to_string().replace('\\', "/")))?;
    // Missing resource tools are a build error, rather than silently shipping a generic icon.
    embed_resource::compile_for(&rc, ["solvecraft"], embed_resource::NONE).manifest_required()?;
    Ok(())
}
