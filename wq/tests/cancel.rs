use std::time::Duration;

use anyhow::Result;
use common::{TestWork, TestWorkResult, DEFAULT_DEADLINE};
use uuid::Uuid;
use wq::WorkQueue;

mod common;

#[tokio::test]
async fn test_cancel() -> Result<()> {
    let valkey = common::start_valkey()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_cancel")
        .with_claim_timeout(Duration::from_millis(1))
        .build(&valkey.url())
        .await?;

    let work_param = TestWork(Uuid::new_v4().to_string());
    let expected_job_id = queue
        .enqueue_job(&work_param, wq::Priority::Low, DEFAULT_DEADLINE)
        .await?;
    queue.cancel_job(expected_job_id).await?;
    let job = queue.claim_job("test").await?;
    assert!(job.is_none());

    Ok(())
}

#[tokio::test]
async fn test_cancel_after_claim() -> Result<()> {
    let valkey = common::start_valkey()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_cancel_after_claim")
        .with_claim_timeout(Duration::from_millis(1))
        .build(&valkey.url())
        .await?;

    let work_param = TestWork(Uuid::new_v4().to_string());
    let expected_job_id = queue
        .enqueue_job(&work_param, wq::Priority::Low, DEFAULT_DEADLINE)
        .await?;
    let job = queue.claim_job("test").await?.expect("Failed to get a job");
    queue.reclaim_job(&job.job_id, "test").await?;

    queue.cancel_job(expected_job_id).await?;
    let reclaim_result = queue.reclaim_job(&job.job_id, "test").await;
    assert!(reclaim_result.is_err());

    Ok(())
}
