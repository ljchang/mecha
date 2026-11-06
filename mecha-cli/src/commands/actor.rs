//! One shell/run/posture reading for owner command gates.
use mecha_core::closure::{self, Actor};

pub(crate) fn acting_actor() -> Actor {
    acting_actor_explained().0
}

/// [`acting_actor`], with why it is not `owner` when it is not — so a
/// reject whose reason will not be read as the owner's says so rather than
/// demoting it silently (review of #343).
pub(crate) fn acting_actor_explained() -> (Actor, Option<String>) {
    closure::acting_actor_explained()
}
