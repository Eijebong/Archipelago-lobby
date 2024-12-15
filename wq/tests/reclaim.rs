use anyhow::Result;
use std::time::Duration;
use uuid::Uuid;

mod common;
use common::{start_valkey, TestWork, DEFAULT_DEADLINE};

#[tokio::test]
async fn test_reclaim_expires() -> Result<()> {
    let valkey = start_valkey()?;

    let queue = std::sync::Arc::new(
        wq::WorkQueue::<TestWork, ()>::builder("test_reclaim_expires")
            .with_reclaim_timeout(Duration::from_micros(1))
            .build(&valkey.url())
            .await?,
    );
    let reclaim_handle = queue.start_reclaim_checker();

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
    assert_eq!(expected_job_id, job.job_id);

    // Let the reclaim loop realize that the job claim expired
    tokio::time::sleep(Duration::from_millis(5)).await;

    let job = queue
        .claim_job("test-new")
        .await?
        .expect("Should've gotten a job");

    assert_eq!(expected_work_content, job.params.0);
    assert_eq!(expected_job_id, job.job_id);

    reclaim_handle.abort();
    let _ = reclaim_handle.await;

    Ok(())
}
