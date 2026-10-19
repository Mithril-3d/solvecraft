//! Independent isotropic CPU LOD reference from unwrapped UV gradients per shading-grid pixel.
//! No sampling, derivative inference, normal filtering or GPU capability is implied.
use crate::texture_input::MAX_TEXTURE_EDGE;

/// Versioned convention: largest singular value, log2, magnification at zero,
/// saturated at the last floor-halved level. Not a GPU parity guarantee.
pub const LOD_POLICY_ID: &str = "solidcraft.isotropic-singular-value-lod/1";
pub const MAX_UV_GRADIENT: f64 = 1_048_576.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LodError {
    Dimensions,
    Gradients,
}
impl std::fmt::Display for LodError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Dimensions => "texture dimensions must be positive and within admission limits",
            Self::Gradients => "UV gradients must be finite and within the CPU reference bound",
        })
    }
}
impl std::error::Error for LodError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodSelection {
    /// Continuous LOD, explicitly saturated to [0, max_level].
    pub lod: f64,
    /// Largest singular value of the UV-to-texel differential, before saturation.
    pub footprint_texels: f64,
    pub max_level: u32,
}

/// Select LOD using caller-supplied derivatives of UNWRAPPED UV per shading-grid pixel.
/// `duv_dx` and `duv_dy` must already account for perspective/placement/mapping.
/// For Sx by Sy supersampling, divide final-image-pixel derivatives by Sx and Sy,
/// respectively. Derivatives already measured on the shading grid need no division.
/// The policy uses a circular differential footprint, not exact pixel-square coverage.
/// It can blur the shorter axis of an elongated footprint; it is not anisotropic.
/// Dimensions describe the base image; its available chain must end at 1x1 through
/// floor-halving. Odd-size levels use the documented log2 approximation.
pub fn select_isotropic_lod(dimensions: [u32; 2], duv_dx: [f64; 2], duv_dy: [f64; 2]) -> Result<LodSelection, LodError> {
    let [width, height] = dimensions;
    if width == 0 || height == 0 || width > MAX_TEXTURE_EDGE || height > MAX_TEXTURE_EDGE {
        return Err(LodError::Dimensions);
    }
    if duv_dx.iter().chain(&duv_dy).any(|v| !v.is_finite() || v.abs() > MAX_UV_GRADIENT) {
        return Err(LodError::Gradients);
    }
    let [dudx, dvdx] = duv_dx;
    let [dudy, dvdy] = duv_dy;
    let columns = [dudx * f64::from(width), dvdx * f64::from(height), dudy * f64::from(width), dvdy * f64::from(height)];
    let scale = columns.into_iter().map(f64::abs).fold(0.0, f64::max);
    let footprint_texels = if scale == 0.0 {
        0.0
    } else {
        let [a, b, c, d] = columns.map(|v| v / scale);
        // Largest eigenvalue of J^T J; scaling keeps the quadratic bounded.
        let first = a * a + b * b;
        let second = c * c + d * d;
        let cross = a * c + b * d;
        scale * (0.5 * (first + second + (first - second).hypot(2.0 * cross))).sqrt()
    };
    let mut w = width;
    let mut h = height;
    let mut max_level = 0;
    while w > 1 || h > 1 {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        max_level += 1;
    }
    let lod = if footprint_texels <= 1.0 { 0.0 } else { footprint_texels.log2().min(f64::from(max_level)) };
    Ok(LodSelection { lod, footprint_texels, max_level })
}

#[cfg(test)]
#[path = "texture_lod_tests.rs"]
mod tests;
