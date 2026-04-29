//! Glue between main.rs and the programs Controller.

use anyhow::Result;

use crate::programs::{Controller, ResolvedItem};
use crate::state::State;

pub fn build_controller(state: State) -> Result<Controller> {
    Controller::new(state)
}

/// First-render resolution. If the user has no enrollments yet returns
/// `Ok(None)` — the caller should surface an empty-state hint.
pub fn resolve_initial(controller: &mut Controller) -> Result<Option<ResolvedItem>> {
    Ok(controller.current())
}
