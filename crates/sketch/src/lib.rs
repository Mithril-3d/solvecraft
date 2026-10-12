//! Sketches: points and curves on a plane, geometric constraints and driving dimensions, a
//! Levenberg–Marquardt constraint solver with degree-of-freedom analysis, and closed-profile
//! (region) detection for features.
//!
//! The solver is our own clean-room implementation (residual functions per constraint, local
//! finite-difference Jacobians, damped Gauss–Newton per connected component, rank analysis by
//! Gaussian elimination). See `plan/architecture.md` §sketch.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod curves;
mod dims;
mod freeform_offset;
mod linalg;
mod link;
mod model;
mod offset;
mod offset_topology;
mod periodic;
mod profiles;
mod shape;
mod solver;
mod text;
mod wire;

pub use curves::{
    MAX_SPLINE_POINTS, bspline_beziers, conic_point, ellipse_point, end_curvature, fit_beziers, fit_beziers_mode, spline_end_tangent,
    spline_end_tangent_mode, spline_point, spline_point_mode, spline_polyline, spline_polyline_mode,
};
pub use dims::{DimFrame, DimLayout, chain_centres, default_text, dim_frame, dim_layout, encode_text};
pub use freeform_offset::{FreeformOffset, OFFSET_TOLERANCE};
pub use link::{Link, LinkGeom, LinkKind, LinkSource, MAX_LINK_ENTITIES};
pub use model::{Constraint, ConstraintKind, Curve, CurveKind, MAX_CONSTRAINTS, MAX_CURVES, MAX_POINTS, SPoint, Sketch, SketchError, SketchView};
pub use offset::OffsetChain;
pub use offset_topology::{OffsetCreated, OffsetRole, OffsetRoleFlags, OffsetRoleKind, OffsetTopology};
pub use periodic::periodic_beziers;
pub use profiles::{CurvePick, Profile, distinct_curves, find_drawn_profiles, find_profiles, merge_regions, profile_at_point, profile_by_curves};
pub use shape::{Shape, intersections};
pub use solver::{Hold, SolveReport, SolveStatus, measure_dimension, solve, solve_holding};
pub use text::{MAX_TEXT_CHARS, text_geometry};
pub use wire::{MAX_FIT_POINTS, MAX_WIRE_POINTS, Wire, WireSpace, fit_polyline};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod virtual_corner_tests;
