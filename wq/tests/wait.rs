use std::{sync::Arc, time::Duration};

use anyhow::Result;
use uuid::Uuid;

mod common;
use common::{start_valkey, TestWork, TestWorkResult, DEFAULT_DEADLINE};
use wq::{JobId, JobStatus, WorkQueue};

#[tokio::test]
async fn test_wait_for_resolve() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = Arc::new(
        WorkQueue::<TestWork, TestWorkResult>::builder("test_resolve")
            .build(&valkey.url())
            .await?,
    );

    let work_param = TestWork(Uuid::new_v4().to_string());

    let expected_job_id = queue
        .enqueue_job(&work_param, wq::Priority::Low, DEFAULT_DEADLINE)
        .await?;

    let job = queue
        .claim_job("test")
        .await?
        .expect("Should've gotten a job");
    assert_eq!(expected_job_id, job.job_id);

    let hqueue = queue.clone();
    let worker_handler = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        hqueue
            .resolve_job(
                "test",
                job.job_id,
                JobStatus::Success,
                TestWorkResult(job.params.0.clone()),
            )
            .await
            .expect("Failed to resolve job");
    });
    let status = queue.wait_for_job(&job.job_id, None).await?;
    assert_eq!(status, Some(JobStatus::Success));

    worker_handler.abort();

    Ok(())
}

#[tokio::test]
async fn test_wait_for_resolve_timeout() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_wait_for_resolve_timeout")
        .build(&valkey.url())
        .await?;

    let work_param = TestWork(Uuid::new_v4().to_string());

    let expected_job_id = queue
        .enqueue_job(&work_param, wq::Priority::Low, DEFAULT_DEADLINE)
        .await?;

    let job = queue
        .claim_job("test")
        .await?
        .expect("Should've gotten a job");

    assert_eq!(expected_job_id, job.job_id);

    let status = queue
        .wait_for_job(&job.job_id, Some(Duration::from_millis(10)))
        .await;
    assert!(status.is_ok());
    assert!(status.unwrap().is_none());

    Ok(())
}

#[tokio::test]
async fn test_wait_for_already_resolved() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_wait_for_already_resolved")
        .build(&valkey.url())
        .await?;

    let work_param = TestWork(Uuid::new_v4().to_string());

    let expected_job_id = queue
        .enqueue_job(&work_param, wq::Priority::Low, DEFAULT_DEADLINE)
        .await?;

    let job = queue
        .claim_job("test")
        .await?
        .expect("Should've gotten a job");

    assert_eq!(expected_job_id, job.job_id);

    queue
        .resolve_job(
            "test",
            job.job_id,
            JobStatus::Success,
            TestWorkResult(job.params.0),
        )
        .await
        .expect("Failed to resolve job");
    let status = queue
        .wait_for_job(&job.job_id, Some(Duration::from_millis(10)))
        .await?;
    assert_eq!(status, Some(JobStatus::Success));

    Ok(())
}

#[tokio::test]
async fn test_wait_for_non_existent_job() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_wait_for_non_existent_job")
        .build(&valkey.url())
        .await?;

    let status = tokio::time::timeout(
        Duration::from_millis(10),
        queue.wait_for_job(&JobId::new(), None),
    )
    .await?;

    assert!(status.is_err());

    Ok(())
}
