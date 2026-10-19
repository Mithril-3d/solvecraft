//! Bounded desktop preparation of independent imported bodies, collected in file order.
use super::{Body, Result, guard, mt};

pub(super) struct Job {
    pub source: Source,
    pub check_mesh: bool,
}

/// What a job places: a solid read from the file (oriented here), or a body an earlier
/// instance of the same item already oriented.
pub(super) enum Source {
    Solid(mt::Solid),
    Oriented(Body),
}

/// The placed body, its faces the mesher missed, and the body oriented where the file defines
/// it (for later instances of the same item).
type Prepared = Result<(Body, usize, Body)>;

pub(super) fn workers() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
    }
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
}

fn prepare(job: &Job, placement: &mt::Matrix4) -> Prepared {
    guard("STEP body preparation", || {
        // Orient once where the file defines the solid; each placement then takes its
        // orientation from the matrix instead of meshing the whole part again.
        let oriented = match &job.source {
            Source::Solid(solid) => Body::new(solid.clone())?,
            Source::Oriented(body) => body.clone(),
        };
        let body = oriented.placed(placement)?;
        // Counted where the orientation mesh already counted them (a placement meshes the
        // same faces).
        let missing = if job.check_mesh { oriented.unmeshed_faces() } else { 0 };
        Ok((body, missing, oriented))
    })
}

pub(super) fn bodies(solids: &[Job], placement: &mt::Matrix4, workers: usize) -> Vec<Prepared> {
    #[cfg(not(target_arch = "wasm32"))]
    if solids.len() >= 4 && workers > 1 {
        return std::thread::scope(|scope| {
            let size = solids.len().div_ceil(workers.min(8));
            let mut pending = Vec::new();
            for chunk in solids.chunks(size) {
                let handle = std::thread::Builder::new()
                    .name("step-body".into())
                    .spawn_scoped(scope, move || chunk.iter().map(|solid| prepare(solid, placement)).collect::<Vec<_>>())
                    .ok();
                pending.push((chunk, handle));
            }
            let mut output = Vec::with_capacity(solids.len());
            for (chunk, handle) in pending {
                match handle {
                    Some(handle) => match handle.join() {
                        Ok(results) => output.extend(results),
                        Err(_) => output.extend(chunk.iter().map(|_| Err(crate::KernelError::Internal("STEP body worker panicked".into())))),
                    },
                    // Resource exhaustion must not lose an import: use the serial path.
                    None => output.extend(chunk.iter().map(|solid| prepare(solid, placement))),
                }
            }
            output
        });
    }
    let _ = workers;
    solids.iter().map(|solid| prepare(solid, placement)).collect()
}
