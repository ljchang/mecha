//! The Slack front-end: everything that knows about both Slack and the agent.
//!
//! The split is deliberate and is `docs/SLACK-DESIGN.md` §1. `mecha-slack` is
//! the transport and cannot depend on `mecha-core`, so it cannot learn what a
//! run, a tool or an approval is. This module knows both sides, which is why it
//! lives here beside `tui/` rather than in that crate.

pub mod approve;
pub mod connector;
pub mod doctor;
pub mod pump;
pub mod threads;
