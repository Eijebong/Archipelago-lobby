use std::time::Duration;

use anyhow::Result;
use common::{start_valkey, TestWork, TestWorkResult};
use uuid::Uuid;
use wq::{Job, JobStatus, QueueStats, WorkQueue};

mod common;

async fn claim_resolve_jobs(
    queue: &WorkQueue<TestWork, TestWorkResult>,
    status: JobStatus,
    n: u32,
) -> Result<()> {
    let claims = claim_jobs(queue, n, n).await?;
    for claim in &claims {
        queue
            .resolve_job(
                "test",
                claim.job_id,
                status,
                TestWorkResult(claim.params.0.clone()),
            )
            .await?;
    }
    Ok(())
}

async fn enqueue_jobs(queue: &WorkQueue<TestWork, TestWorkResult>, n: u32) -> Result<()> {
    let param = TestWork(Uuid::new_v4().to_string());
    for _ in 0..n {
        queue
            .enqueue_job(&param, wq::Priority::Low, Duration::from_secs(10))
            .await?;
    }

    Ok(())
}

async fn claim_jobs(
    queue: &WorkQueue<TestWork, TestWorkResult>,
    n_scheduled: u32,
    n_claimed: u32,
) -> Result<Vec<Job<TestWork>>> {
    let mut claims = vec![];
    enqueue_jobs(queue, n_scheduled).await?;
    for _ in 0..n_claimed {
        let claim = queue.claim_job("test").await?.expect("Failed to claim job");
        claims.push(claim);
    }

    Ok(claims)
}

#[tokio::test]
async fn test_get_empty_stats() -> Result<()> {
    let valkey = start_valkey()?;
    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_get_empty_stats")
        .build(&valkey.url())
        .await?;

    assert_eq!(
        queue.get_stats().await?,
        QueueStats {
            jobs_failed: 0,
            jobs_succeeded: 0,
            jobs_errored: 0,
            jobs_scheduled: 0,
            jobs_claimed: 0
        }
    );

    Ok(())
}

#[tokio::test]
async fn test_stats_resolved() -> Result<()> {
    let valkey = start_valkey()?;
    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_stats_resolved")
        .build(&valkey.url())
        .await?;

    claim_resolve_jobs(&queue, JobStatus::Success, 3).await?;
    claim_resolve_jobs(&queue, JobStatus::InternalError, 2).await?;
    claim_resolve_jobs(&queue, JobStatus::Failure, 4).await?;
    assert_eq!(
        queue.get_stats().await?,
        QueueStats {
            jobs_failed: 4,
            jobs_succeeded: 3,
            jobs_errored: 2,
            jobs_scheduled: 0,
            jobs_claimed: 0
        }
    );

    Ok(())
}

#[tokio::test]
async fn test_stats_scheduled() -> Result<()> {
    let valkey = start_valkey()?;
    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_stats_scheduled")
        .build(&valkey.url())
        .await?;

    enqueue_jobs(&queue, 10).await?;
    assert_eq!(
        queue.get_stats().await?,
        QueueStats {
            jobs_failed: 0,
            jobs_succeeded: 0,
            jobs_errored: 0,
            jobs_scheduled: 10,
            jobs_claimed: 0
        }
    );

    Ok(())
}

#[tokio::test]
async fn test_stats_claimed() -> Result<()> {
    let valkey = start_valkey()?;
    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_stats_claimed")
        .build(&valkey.url())
        .await?;

    claim_jobs(&queue, 10, 5).await?;
    assert_eq!(
        queue.get_stats().await?,
        QueueStats {
            jobs_failed: 0,
            jobs_succeeded: 0,
            jobs_errored: 0,
            jobs_scheduled: 5,
            jobs_claimed: 5
        }
    );

    Ok(())
}
