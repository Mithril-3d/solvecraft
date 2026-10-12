//! The SolveCraft document: user and model parameters, and a parametric timeline of features
//! (sketches, extrudes, revolves, fillets, primitives, combines…). Evaluating the timeline with
//! the kernel produces the model (bodies and solved sketches). Evaluation is incremental: a
//! feature is recomputed only when it, a parameter it uses, or an earlier feature changed.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod appearance;
mod assembly;
mod body_transfer;
mod body_transfer_sheet;
pub use body_transfer::{valid_transfer_matrix, validate_body_transfer};
pub mod canvas;
mod clipboard;
pub use clipboard::{clone_expression, remap_topology};
pub mod config;
pub mod construct;
mod document;
mod eval;
pub mod expr;
pub mod face_anchor;
pub use face_anchor::{FaceAnchor, FaceAttachmentStatus};
pub mod format;
mod format_offsets;
pub use format_offsets::whole_profile_selection_from_value;
pub mod joints;
pub mod motion;
pub mod naming;
mod params;
mod profile_refs;
pub use profile_refs::{capture_profile_selection, profile_selection_indices};
pub mod plastic;
pub mod project;
mod project3d;
pub use project3d::iso_grid;
pub mod sheet;

pub use assembly::{IDENTITY, Mat, Occurrence, apply_point, apply_vector, is_identity, map_point_expr, mat_inverse, mat_mul, rigid};
pub use document::*;
pub use eval::{
    EdgeRef, FeatureResult, Model, ModelBody, ModelState, RevolveExtent, SolvedSketch, ThreadInfo, curve_pick_error, edge_names_for, edge_refs,
    object_height, parse_metric_thread, pattern_matrices, revolve_extent, revolve_extent_with_convention, select_profiles, state_in_frame,
    state_in_frame_at, through_all_distance, world_state,
};
pub use params::{ParamRow, ParamUser, cycles};
pub use solvecraft_kernel as kernel;
pub use solvecraft_sketch as sketch;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DocError {
    #[error("expression: {0}")]
    Expr(String),
    #[error("{0}")]
    Invalid(String),
    #[error("unknown {0}")]
    Unknown(String),
    #[error(transparent)]
    Sketch(#[from] solvecraft_sketch::SketchError),
    #[error(transparent)]
    Kernel(#[from] solvecraft_kernel::KernelError),
}

pub type Result<T> = std::result::Result<T, DocError>;

#[cfg(test)]
mod named_projection_tests;
#[cfg(test)]
mod tests;

mod anchor_frame;
pub use anchor_frame::state_in_face_anchor_frame;
