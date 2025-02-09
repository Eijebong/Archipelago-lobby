use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use redis::Commands;
use uuid::Uuid;

mod common;
use common::{queue_resolve_job, start_valkey, TestWork, TestWorkResult, DEFAULT_DEADLINE};
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

#[tokio::test]
async fn test_callback_on_resolve_processed() -> Result<()> {
    let valkey = start_valkey()?;
    let client = redis::Client::open(valkey.url())?;
    let mut conn = client.get_connection()?;

    static HAS_BEEN_CALLED: Mutex<bool> = Mutex::new(false);

    fn callback(
        _params: TestWork,
        result: TestWorkResult,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> {
        Box::pin(async move {
            *HAS_BEEN_CALLED.lock().unwrap() = !result.0.is_empty();
            Ok(true)
        })
    }

    let queue =
        WorkQueue::<TestWork, TestWorkResult>::builder("test_callback_on_resolve_processed")
            .with_callback(Arc::pin(callback))
            .build(&valkey.url())
            .await?;

    let job_id = queue_resolve_job(&queue).await?;

    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(*HAS_BEEN_CALLED.lock().unwrap(), true);

    // Check that the result and params are gone after the event has been processed
    let result = conn.get::<_, Option<String>>(queue.get_result_key(&job_id))?;
    let params = conn.get::<_, Option<String>>(queue.get_job_key(&job_id))?;
    assert!(params.is_none());
    assert!(result.is_none());

    Ok(())
}

#[tokio::test]
async fn test_callback_on_resolve_not_processed() -> Result<()> {
    let valkey = start_valkey()?;
    let client = redis::Client::open(valkey.url())?;
    let mut conn = client.get_connection()?;

    static HAS_BEEN_CALLED: Mutex<bool> = Mutex::new(false);

    fn callback(
        _params: TestWork,
        result: TestWorkResult,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> {
        Box::pin(async move {
            *HAS_BEEN_CALLED.lock().unwrap() = !result.0.is_empty();
            Ok(false)
        })
    }

    let queue =
        WorkQueue::<TestWork, TestWorkResult>::builder("test_callback_on_resolve_not_processed")
            .with_callback(Arc::pin(callback))
            .build(&valkey.url())
            .await?;

    let job_id = queue_resolve_job(&queue).await?;

    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(*HAS_BEEN_CALLED.lock().unwrap(), true);

    let result = conn.get::<_, Option<String>>(queue.get_result_key(&job_id))?;
    let params = conn.get::<_, Option<String>>(queue.get_job_key(&job_id))?;
    assert!(params.is_some());
    assert!(result.is_some());

    Ok(())
}

#[tokio::test]
async fn test_callback_on_resolve_error() -> Result<()> {
    let valkey = start_valkey()?;
    let client = redis::Client::open(valkey.url())?;
    let mut conn = client.get_connection()?;

    static HAS_BEEN_CALLED: Mutex<bool> = Mutex::new(false);

    fn callback(
        _params: TestWork,
        result: TestWorkResult,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> {
        Box::pin(async move {
            *HAS_BEEN_CALLED.lock().unwrap() = !result.0.is_empty();
            Err(anyhow::anyhow!("oof"))
        })
    }

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_callback_on_resolve_error")
        .with_callback(Arc::pin(callback))
        .build(&valkey.url())
        .await?;

    let job_id = queue_resolve_job(&queue).await?;

    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(*HAS_BEEN_CALLED.lock().unwrap(), true);

    let result = conn.get::<_, Option<String>>(queue.get_result_key(&job_id))?;
    let params = conn.get::<_, Option<String>>(queue.get_job_key(&job_id))?;
    assert!(params.is_some());
    assert!(result.is_some());

    Ok(())
}

#[tokio::test]
async fn test_callback_orphaned_jobs() -> Result<()> {
    let valkey = start_valkey()?;
    let client = redis::Client::open(valkey.url())?;
    let mut conn = client.get_connection()?;

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_callback_orphaned_jobs")
        .build(&valkey.url())
        .await?;

    let job_id = queue_resolve_job(&queue).await?;
    let result = conn.get::<_, Option<String>>(queue.get_result_key(&job_id))?;
    let params = conn.get::<_, Option<String>>(queue.get_job_key(&job_id))?;
    assert!(params.is_some());
    assert!(result.is_some());

    // Leak the job on purpose here
    static HAS_BEEN_CALLED: Mutex<bool> = Mutex::new(false);

    fn callback(
        _params: TestWork,
        result: TestWorkResult,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>> {
        Box::pin(async move {
            *HAS_BEEN_CALLED.lock().unwrap() = !result.0.is_empty();
            Ok(true)
        })
    }

    let queue = WorkQueue::<TestWork, TestWorkResult>::builder("test_callback_orphaned_jobs")
        .with_callback(Arc::pin(callback))
        .build(&valkey.url())
        .await?;

    queue.process_orphaned_job_results().await?;
    tokio::time::sleep(Duration::from_millis(5)).await;

    assert_eq!(*HAS_BEEN_CALLED.lock().unwrap(), true);

    let result = conn.get::<_, Option<String>>(queue.get_result_key(&job_id))?;
    let params = conn.get::<_, Option<String>>(queue.get_job_key(&job_id))?;
    assert!(params.is_none());
    assert!(result.is_none());

    Ok(())
}
