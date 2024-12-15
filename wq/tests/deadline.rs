use std::time::Duration;

use anyhow::Result;
use common::{start_valkey, TestWork, TestWorkResult};
use uuid::Uuid;
use wq::WorkQueue;

mod common;

#[tokio::test]
async fn test_deadline_passed() -> Result<()> {
    let valkey = start_valkey()?;
    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_deadline_passed")
        .with_claim_timeout(Duration::from_millis(1))
        .build(&valkey.url())
        .await?;

    let work_param = TestWork(Uuid::new_v4().to_string());
    queue
        .enqueue_job(&work_param, wq::Priority::Low, Duration::from_millis(1))
        .await?;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let claim = queue.claim_job("test").await?;

    assert!(claim.is_none());

    Ok(())
}
