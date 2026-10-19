//! Perspective-correct unwrapped UV and analytic gradients from caller-owned barycentrics.
//! No projection, clipping, raster coverage, instance transform or texture sampling.

pub const UV_INTERPOLATION_POLICY: &str = "solidcraft.perspective-uv-differential/1";
/// Matches the existing raster's near-edge admission; does not replace its top-left rule.
pub const BARYCENTRIC_EDGE_TOLERANCE: f64 = 1e-9;
const MAX_UV: f64 = 1_000_000.0;
const MAX_DERIVATIVE: f64 = 1_000_000.0;
const SUM_TOLERANCE: f64 = 1e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterpolationError {
    VertexUv,
    ReciprocalW,
    Barycentrics,
    Derivatives,
    Output,
}
impl std::fmt::Display for InterpolationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::VertexUv => "vertex UV must be finite and within one million repeats",
            Self::ReciprocalW => "reciprocal clip-w must be finite and in [1e-12, 1e12]",
            Self::Barycentrics => "barycentric weights exceed near-edge bounds or do not sum to one",
            Self::Derivatives => "barycentric derivatives must be bounded, finite and sum to zero",
            Self::Output => "interpolated UV or gradients exceed the numerical reference bounds",
        })
    }
}
impl std::error::Error for InterpolationError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvDifferential {
    pub uv: [f64; 2],
    /// Unwrapped UV per actual shading-grid pixel step, not necessarily final-image pixels.
    pub duv_dx: [f64; 2],
    pub duv_dy: [f64; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct PerspectiveUv {
    anchor: [f64; 2],
    deltas: [[f64; 2]; 3],
    reciprocal_w: [f64; 3],
}
impl PerspectiveUv {
    /// Vertices must already have valid positive clip-w after caller-owned clipping.
    /// Only a common reciprocal-w scale is removed; UVs are never wrapped or flipped.
    pub fn new(uv: [[f64; 2]; 3], reciprocal_w: [f64; 3]) -> Result<Self, InterpolationError> {
        if uv.iter().flatten().any(|v| !v.is_finite() || v.abs() > MAX_UV) {
            return Err(InterpolationError::VertexUv);
        }
        if reciprocal_w.iter().any(|v| !v.is_finite() || !(1e-12..=1e12).contains(v)) {
            return Err(InterpolationError::ReciprocalW);
        }
        let [anchor, _, _] = uv;
        let [anchor_u, anchor_v] = anchor;
        let scale = reciprocal_w.into_iter().fold(0.0, f64::max);
        let deltas = uv.map(|[u, v]| [u - anchor_u, v - anchor_v]);
        Ok(Self { anchor, deltas, reciprocal_w: reciprocal_w.map(|v| v / scale) })
    }

    /// Barycentrics and derivatives come from the same projected triangle/shading grid.
    /// No coverage rule is applied here: weights beyond the explicit near-edge bound, malformed partitions and
    /// unsupported gradients refuse instead of falling back to affine interpolation.
    pub fn evaluate(&self, weights: [f64; 3], dweights_dx: [f64; 3], dweights_dy: [f64; 3]) -> Result<UvDifferential, InterpolationError> {
        let admitted_weights = -BARYCENTRIC_EDGE_TOLERANCE..=1.0 + 2.0 * BARYCENTRIC_EDGE_TOLERANCE;
        if weights.iter().any(|v| !v.is_finite() || !admitted_weights.contains(v)) || (weights.into_iter().sum::<f64>() - 1.0).abs() > SUM_TOLERANCE {
            return Err(InterpolationError::Barycentrics);
        }
        for derivatives in [dweights_dx, dweights_dy] {
            let scale = derivatives.into_iter().map(f64::abs).fold(1.0, f64::max);
            if derivatives.iter().any(|v| !v.is_finite() || v.abs() > MAX_DERIVATIVE)
                || derivatives.into_iter().sum::<f64>().abs() > SUM_TOLERANCE * scale
            {
                return Err(InterpolationError::Derivatives);
            }
        }
        let denominator = weighted_sum(weights, self.reciprocal_w);
        if !(denominator > 0.0) || !denominator.is_finite() {
            return Err(InterpolationError::Output);
        }
        let dx_denominator = weighted_sum(dweights_dx, self.reciprocal_w);
        let dy_denominator = weighted_sum(dweights_dy, self.reciprocal_w);
        let evaluate_channel = |anchor, [a, b, c]: [f64; 3]| {
            let [qa, qb, qc] = self.reciprocal_w;
            let corrected = [a * qa, b * qb, c * qc];
            let delta = weighted_sum(weights, corrected) / denominator;
            // Avoid denominator squared and subtract the anchor before differentiating.
            [
                anchor + delta,
                (weighted_sum(dweights_dx, corrected) - delta * dx_denominator) / denominator,
                (weighted_sum(dweights_dy, corrected) - delta * dy_denominator) / denominator,
            ]
        };
        let [[u0, v0], [u1, v1], [u2, v2]] = self.deltas;
        let [anchor_u, anchor_v] = self.anchor;
        let [u, du_dx, du_dy] = evaluate_channel(anchor_u, [u0, u1, u2]);
        let [v, dv_dx, dv_dy] = evaluate_channel(anchor_v, [v0, v1, v2]);
        let result = UvDifferential { uv: [u, v], duv_dx: [du_dx, dv_dx], duv_dy: [du_dy, dv_dy] };
        if result.uv.iter().chain(&result.duv_dx).chain(&result.duv_dy).any(|v| !v.is_finite() || v.abs() > MAX_UV) {
            return Err(InterpolationError::Output);
        }
        Ok(result)
    }
}

fn weighted_sum(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

#[cfg(test)]
#[path = "texture_interpolation_tests.rs"]
mod tests;
