use std::time::Duration;

use anyhow::Result;
use redis::Commands;
use uuid::Uuid;
use wq::{Claim, JobStatus, Priority};

mod common;
use common::{start_valkey, TestWork, TestWorkResult, DEFAULT_DEADLINE};

#[tokio::test]
async fn simple_get_pop() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = wq::WorkQueue::<TestWork, ()>::builder("simple_get_pop")
        .build(&valkey.url())
        .await?;
    let expected_work_content = Uuid::new_v4().to_string();

    let expected_job_id = queue
        .enqueue_job(
            &TestWork(expected_work_content.clone()),
            wq::Priority::Low,
            DEFAULT_DEADLINE,
        )
        .await?;
    let job = queue
        .claim_job("test")
        .await?
        .expect("Should've gotten a job");

    assert_eq!(job.params.0, expected_work_content);
    assert_eq!(job.job_id, expected_job_id);

    Ok(())
}

#[tokio::test]
async fn test_priority() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = wq::WorkQueue::<TestWork, ()>::builder("test_priority")
        .build(&valkey.url())
        .await?;
    let expected_low_content = format!("low-{}", Uuid::new_v4());
    let expected_normal_content = format!("normal-{}", Uuid::new_v4());
    let expected_high_content = format!("high-{}", Uuid::new_v4());

    let mut low_queue = vec![];
    let mut normal_queue = vec![];
    let mut high_queue = vec![];

    for i in 0..10 {
        let (priority, work, expected_queue) = match i % 3 {
            0 => (Priority::Low, &expected_low_content, &mut low_queue),
            1 => (
                Priority::Normal,
                &expected_normal_content,
                &mut normal_queue,
            ),
            2 => (Priority::High, &expected_high_content, &mut high_queue),
            _ => unreachable!(),
        };

        expected_queue.push(
            queue
                .enqueue_job(&TestWork(work.clone()), priority, DEFAULT_DEADLINE)
                .await?,
        );
    }

    for _ in 0..3 {
        let job = queue
            .claim_job("test")
            .await?
            .expect("Should've gotten a job");

        assert_eq!(job.params.0, expected_high_content);
        assert_eq!(job.job_id, high_queue.remove(0))
    }
    for _ in 0..3 {
        let job = queue
            .claim_job("test")
            .await?
            .expect("Should've gotten a job");

        assert_eq!(job.params.0, expected_normal_content);
        assert_eq!(job.job_id, normal_queue.remove(0))
    }
    for _ in 0..4 {
        let job = queue
            .claim_job("test")
            .await?
            .expect("Should've gotten a job");

        assert_eq!(job.params.0, expected_low_content);
        assert_eq!(job.job_id, low_queue.remove(0))
    }

    Ok(())
}

#[tokio::test]
async fn test_reclaim() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = wq::WorkQueue::<TestWork, ()>::builder("test_reclaim")
        .build(&valkey.url())
        .await?;
    let mut redis = redis::Client::open(valkey.url())?;

    let expected_job_id = queue
        .enqueue_job(&TestWork("".to_string()), Priority::Low, DEFAULT_DEADLINE)
        .await?;

    let Some(job) = queue.claim_job("test").await? else {
        panic!("Failed to get a job");
    };

    assert_eq!(job.job_id, expected_job_id);

    let claim = redis.hget::<_, _, Claim>("wq:test_reclaim:claims", job.job_id.to_string())?;
    let first_claim_time = claim.time.clone();
    assert_eq!(claim.worker_id, "test");

    queue.reclaim_job(&job.job_id, "test").await?;

    let claim = redis.hget::<_, _, Claim>("wq:test_reclaim:claims", job.job_id.to_string())?;
    let second_claim_time = claim.time.clone();
    assert_eq!(claim.worker_id, "test");
    assert!(second_claim_time > first_claim_time);

    Ok(())
}

#[tokio::test]
async fn test_reclaim_different_id() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = wq::WorkQueue::<TestWork, ()>::builder("test_reclaim_different_id")
        .build(&valkey.url())
        .await?;
    let expected_job_id = queue
        .enqueue_job(&TestWork("".to_string()), Priority::Low, DEFAULT_DEADLINE)
        .await?;
    let Some(job) = queue.claim_job("test").await? else {
        panic!("Failed to get a job");
    };

    assert_eq!(job.job_id, expected_job_id);
    assert!(queue.reclaim_job(&job.job_id, "test").await.is_ok());
    assert!(queue
        .reclaim_job(&job.job_id, "test-different")
        .await
        .is_err());
    assert!(queue.reclaim_job(&job.job_id, "test").await.is_ok());

    Ok(())
}

#[tokio::test]
async fn test_no_job() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = wq::WorkQueue::<TestWork, ()>::builder("test_no_job")
        .with_claim_timeout(Duration::from_millis(1))
        .build(&valkey.url())
        .await?;
    let job = queue.claim_job("test").await?;
    assert_eq!(None, job);

    Ok(())
}

#[tokio::test]
async fn test_delete_result() -> Result<()> {
    let valkey = start_valkey()?;
    let client = redis::Client::open(valkey.url())?;
    let mut conn = client.get_connection()?;

    let queue = wq::WorkQueue::<TestWork, TestWorkResult>::builder("test_delete_result")
        .with_claim_timeout(Duration::from_millis(1))
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
            TestWorkResult(job.job_id.to_string()),
        )
        .await?;

    let result = conn.get::<_, Option<String>>(queue.get_result_key(&job.job_id))?;
    assert!(result.is_some());

    queue.delete_job_result(job.job_id).await?;
    let result = conn.get::<_, Option<String>>(queue.get_result_key(&job.job_id))?;
    assert!(result.is_none());

    Ok(())
}
