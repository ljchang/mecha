//! Synthetic authored-provider integration, never a memory-benefit measurement.
#![cfg(feature = "memory-prototype")]
#[path = "../examples/support/memory_bridge.rs"]
#[allow(dead_code)]
mod support;
use anyhow::Result;

#[tokio::test]
async fn connected_capture_reopen_changed_goal_and_capture_only_retry() -> Result<()> {
    support::connected_capture_reopen_changed_goal_and_capture_only_retry().await
}

#[tokio::test]
async fn successful_dispatch_wrong_artifact_and_unknown_sensor_remain_distinct() -> Result<()> {
    support::successful_dispatch_wrong_artifact_and_unknown_sensor_remain_distinct().await
}

#[tokio::test]
async fn goalmemory_rechecks_missing_authority_withdrawal_and_forgetting() -> Result<()> {
    support::goalmemory_rechecks_missing_authority_withdrawal_and_forgetting().await
}

#[tokio::test]
async fn changing_confirmed_goal_after_freeze_refuses_capture_without_projection() -> Result<()> {
    support::changing_confirmed_goal_after_freeze_refuses_capture_without_projection().await
}

#[tokio::test]
#[ignore = "child entry point; parent supplies isolated synthetic root"]
async fn subprocess_goalmemory_changed_criterion() -> Result<()> {
    let root = std::path::PathBuf::from(
        std::env::var_os("MECHA_BRIDGE_QUALIFICATION_ROOT")
            .ok_or_else(|| anyhow::anyhow!("parent root missing"))?,
    );
    support::subprocess_goalmemory_changed_criterion(&root).await
}
