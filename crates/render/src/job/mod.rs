//! CPU product-image work and its device-independent, session-scoped protocol.
//! Hosts own execution; documents and history never contain these values.

use std::fmt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::{BackendId, Camera, ExecutorId};

mod host;
mod image;
pub use host::{CpuHost, HostKind, JobHandle, JobStatus, RenderFrame};
pub use image::{Admission, ImageTask, TaskPoll, admit};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobId {
    pub session: u64,
    pub serial: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Queued,
    Preparing,
    Running,
    Encoding,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
}

impl Phase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Completed | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    LimitExceeded,
    AllocationFailed,
    HostUnavailable,
    QueueFull,
    TimedOut,
    Cancelled,
    InvalidScene,
    EncodingFailed,
    WrongOwner,
    UnknownJob,
    NotCompleted,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct JobError {
    pub code: ErrorCode,
    pub message: String,
}

impl JobError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}
impl std::error::Error for JobError {}

pub type Result<T> = std::result::Result<T, JobError>;

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(JobError::new(ErrorCode::Cancelled, "render cancelled")) } else { Ok(()) }
    }
}

/// Fully resolved input; no mutable session, topology graph or GPU resources.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRequest {
    pub camera: Camera,
    pub width: u32,
    pub height: u32,
    pub background: [u8; 3],
    pub transparent_background: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ColourEncoding {
    Srgb,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaConvention {
    Straight,
}

#[derive(Clone, Debug, Serialize)]
pub struct FrameInfo {
    pub job: JobId,
    pub scene_generation: u64,
    pub backend: BackendId,
    pub executor: ExecutorId,
    pub width: u32,
    pub height: u32,
    pub encoding: ColourEncoding,
    pub alpha: AlphaConvention,
}
