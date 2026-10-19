//! Surface analysis as a view draws it: the shading mode and the colours it uses.

use solidcraft_geom::Vec3;

/// Surface analysis colours, straight sRGB 0..1 as the shader takes them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnalysisColors {
    /// Draft Analysis: at least the draft angle, between, at most minus the angle.
    pub draft: [[f32; 3]; 3],
    /// Accessibility Analysis: reachable, blocked.
    pub access: [[f32; 3]; 2],
    /// Curvature Map: flat, the middle of the scale, at or below the analysis radius.
    pub curvature: [[f32; 3]; 3],
    /// Stripe the negative draft and blocked faces too, so colour is not the only cue.
    pub hatch: bool,
}

impl AnalysisColors {
    /// The light theme's colours (no colour vision mode): what headless pictures use.
    pub const DEFAULT: AnalysisColors = AnalysisColors {
        draft: [[0.20, 0.75, 0.30], [0.95, 0.80, 0.15], [0.85, 0.22, 0.20]],
        access: [[0.20, 0.75, 0.30], [0.85, 0.22, 0.20]],
        curvature: [[0.15, 0.35, 0.95], [0.10, 0.80, 0.25], [0.90, 0.15, 0.10]],
        hatch: false,
    };
}

impl Default for AnalysisColors {
    fn default() -> Self {
        AnalysisColors::DEFAULT
    }
}

/// A way of shading the model's faces to judge their shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnalysisMode {
    /// Reflected stripes: `stripes` across the view.
    Zebra { stripes: f64 },
    /// Faces by draft angle (degrees) to `pull`.
    Draft { pull: Vec3, angle: f64 },
    /// Colour by curvature, reaching the tight end at 1 / `radius`.
    Curvature { radius: f64 },
    /// Mirror finish reflecting a studio.
    Environment,
    /// Reachable along `dir` (facing it, nothing above) or blocked.
    Access { dir: Vec3 },
}

/// A surface analysis shading a picture: the mode and its colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneAnalysis {
    pub mode: AnalysisMode,
    pub colors: AnalysisColors,
}
