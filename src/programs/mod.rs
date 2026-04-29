pub mod catalog;
pub mod engine;
pub mod enrollment;
pub mod migration;

use anyhow::Result;
use tracing::warn;

use crate::state::State;

pub use catalog::Catalog;
pub use engine::{ItemStatus, ResolvedItem};

/// Holds the live State. The viewer drives this via `current` / `advance` /
/// `rotate_next` / `undo`. State is persisted internally on every mutation.
pub struct Controller {
    state: State,
    current_program_id: Option<String>,
}

impl Controller {
    pub fn new(state: State) -> Result<Self> {
        // Validate the embedded catalog parses; we don't otherwise need it here.
        let _ = Catalog::embedded()?;
        Ok(Self {
            state,
            current_program_id: None,
        })
    }

    /// Pick the program for this invocation by rotation, resolve it, and
    /// remember which one is active so future calls (`advance`, `undo`)
    /// know what to mutate.
    pub fn current(&mut self) -> Option<ResolvedItem> {
        let count = self.state.invocation_count;
        let enrollment = engine::pick_for_invocation(&self.state, count)?.clone();
        self.current_program_id = Some(enrollment.program_id.clone());
        match engine::resolve(&enrollment) {
            Ok(r) => Some(r),
            Err(e) => {
                warn!(error = %e, "resolve failed");
                None
            }
        }
    }

    pub fn advance(&mut self) -> Option<ResolvedItem> {
        let id = self.current_program_id.clone()?;
        match engine::advance(&mut self.state, &id) {
            Ok(r) => {
                if r.status == ItemStatus::JustCompleted {
                    // Program archived inside advance(); current_program_id is now stale.
                    // Leave it pointing at the now-archived id; rotate_next will move
                    // forward. The viewer shows the siyum banner on this resolved item.
                }
                Some(r)
            }
            Err(e) => {
                warn!(error = %e, "advance failed");
                None
            }
        }
    }

    pub fn rotate_next(&mut self) -> Option<ResolvedItem> {
        if self.state.enrollments.is_empty() {
            return None;
        }
        let next_idx = match self.current_program_id.as_deref() {
            Some(id) => engine::next_program_index(&self.state, id).unwrap_or(0),
            None => 0,
        };
        let enrollment = self.state.enrollments.get(next_idx)?.clone();
        self.current_program_id = Some(enrollment.program_id.clone());
        match engine::resolve(&enrollment) {
            Ok(r) => Some(r),
            Err(e) => {
                warn!(error = %e, "resolve failed during rotate");
                None
            }
        }
    }

    pub fn undo(&mut self) -> Option<ResolvedItem> {
        let id = self.current_program_id.clone()?;
        match engine::undo(&mut self.state, &id) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "undo failed");
                None
            }
        }
    }

    pub fn bump_invocation(&mut self) {
        self.state.bump();
        let _ = self.state.save();
    }
}
