use anyhow::Result;
use redis::Commands;
use wq::{Claim, JobId, JobResult, JobStatus, Priority};

mod common;
use common::start_valkey;

#[test]
fn test_roundtrip_claim() -> Result<()> {
    let valkey = start_valkey()?;

    let mut redis = redis::Client::open(valkey.url())?;
    let original_claim = Claim::new("test-claim", JobId::new(), Priority::Low);

    redis.set::<_, _, ()>("roundtrip-claim", &original_claim)?;
    let gotten_claim = redis.get::<_, Claim>("roundtrip-claim")?;
    assert_eq!(original_claim, gotten_claim);

    Ok(())
}

#[test]
fn test_job_result_with_value() -> Result<()> {
    let valkey = start_valkey()?;

    let mut redis = redis::Client::open(valkey.url())?;
    let original_result: JobResult<String> = JobResult {
        status: JobStatus::Success,
        result: Some("test result".to_string()),
    };

    redis.set::<_, _, ()>("roundtrip-result", &original_result)?;
    let gotten_result = redis.get::<_, JobResult<String>>("roundtrip-result")?;
    assert_eq!(original_result, gotten_result);

    Ok(())
}

#[test]
fn test_job_result_with_none() -> Result<()> {
    let valkey = start_valkey()?;

    let mut redis = redis::Client::open(valkey.url())?;
    let original_result: JobResult<String> = JobResult {
        status: JobStatus::InternalError,
        result: None,
    };

    redis.set::<_, _, ()>("roundtrip-result-none", &original_result)?;
    let gotten_result = redis.get::<_, JobResult<String>>("roundtrip-result-none")?;
    assert_eq!(original_result, gotten_result);

    Ok(())
}
