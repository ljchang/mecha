//! One shell/run/posture reading for owner command gates.
use mecha_core::closure::{self, Actor};

pub(crate) fn acting_actor() -> Actor {
    acting_actor_explained().0
}

/// [`acting_actor`], with why it is not `owner` when it is not — so a
/// reject whose reason will not be read as the owner's says so rather than
/// demoting it silently (review of #343).
pub(crate) fn acting_actor_explained() -> (Actor, Option<String>) {
    let ancestor = match crate::commands::tasks::live_run_pids() {
        Ok(pids) => closure::run_ancestor(&pids),
        Err(e) => {
            return (
                Actor::Unknown,
                Some(format!(
                    "the run markers could not be read ({e:#}), so whether a run did this \
                     cannot be told"
                )),
            )
        }
    };
    closure::attribute_explained(
        &closure::posture_from_env(),
        &closure::ShellReading::from_registry(),
        ancestor,
    )
}
