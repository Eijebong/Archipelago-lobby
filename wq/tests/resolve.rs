use anyhow::Result;
use redis::Commands;
use uuid::Uuid;

mod common;
use common::{start_valkey, TestWork, TestWorkResult, DEFAULT_DEADLINE};
use wq::{JobId, JobResult, JobStatus, WorkQueue};

#[tokio::test]
async fn test_resolve_success() -> Result<()> {
    let valkey = start_valkey()?;
    let client = redis::Client::open(valkey.url())?;
    let mut conn = client.get_connection()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_resolve")
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
            TestWorkResult(job.params.0.clone()),
        )
        .await?;

    let result = conn.get::<_, JobResult<TestWorkResult>>(queue.get_result_key(&job.job_id))?;
    assert_eq!(result.status, JobStatus::Success);
    assert_eq!(result.result.0, job.params.0);

    Ok(())
}

#[tokio::test]
async fn test_resolve_with_non_resolved_status() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_resolve")
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

    assert!(queue
        .resolve_job(
            "test",
            job.job_id,
            JobStatus::Pending,
            TestWorkResult(job.params.0.clone())
        )
        .await
        .is_err());
    assert!(queue
        .resolve_job(
            "test",
            job.job_id,
            JobStatus::Running,
            TestWorkResult(job.params.0)
        )
        .await
        .is_err());

    Ok(())
}

#[tokio::test]
async fn test_reresolve() -> Result<()> {
    let valkey = start_valkey()?;
    let client = redis::Client::open(valkey.url())?;
    let mut conn = client.get_connection()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_reresolve")
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
            TestWorkResult(job.params.0.clone()),
        )
        .await?;
    let second_resolve = queue
        .resolve_job(
            "test",
            job.job_id,
            JobStatus::Failure,
            TestWorkResult(job.params.0.clone()),
        )
        .await;

    assert!(second_resolve.is_err());

    let result = conn.get::<_, JobResult<TestWorkResult>>(queue.get_result_key(&job.job_id))?;
    assert_eq!(result.status, JobStatus::Success);
    assert_eq!(result.result.0, job.params.0);

    Ok(())
}

#[tokio::test]
async fn test_resolve_non_existant() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_resolve_non_existant")
        .build(&valkey.url())
        .await?;

    let job_id = JobId::new();
    let resolve = queue
        .resolve_job(
            "test",
            job_id,
            JobStatus::Failure,
            TestWorkResult("".to_string()),
        )
        .await;

    assert!(resolve.is_err());
    Ok(())
}

#[tokio::test]
async fn test_resolve_wrong_worker() -> Result<()> {
    let valkey = start_valkey()?;
    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_resolve_wrong_worker")
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

    let resolve = queue
        .resolve_job(
            "wrong",
            job.job_id,
            JobStatus::Success,
            TestWorkResult(job.params.0.clone()),
        )
        .await;

    assert!(resolve.is_err());

    Ok(())
}

#[tokio::test]
async fn test_get_result() -> Result<()> {
    let valkey = start_valkey()?;
    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_get_result")
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
            TestWorkResult(job.params.0.clone()),
        )
        .await?;

    let result = queue.get_job_result(job.job_id).await?;
    assert_eq!(result.0, job.params.0);

    Ok(())
}
