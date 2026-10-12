mod divide_face;
mod faces_classification;
mod integrate;
mod intersection_curve;
// SolveCraft: expose the constructor for workspace regression coverage of the vendored fix.
#[doc(hidden)]
pub use intersection_curve::IntersectionCurveWithParameters;
mod loops_store;
mod polyline_construction;
pub use integrate::{and, or, ShapeOpsCurve, ShapeOpsSurface};
