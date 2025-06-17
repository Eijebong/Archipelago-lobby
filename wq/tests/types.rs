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
fn test_job_result() -> Result<()> {
    let valkey = start_valkey()?;

    let mut redis = redis::Client::open(valkey.url())?;
    let original_result = JobResult {
        status: JobStatus::Success,
        result: (),
    };

    redis.set::<_, _, ()>("roundtrip-result", &original_result)?;
    let gotten_result = redis.get::<_, JobResult<_>>("roundtrip-result")?;
    assert_eq!(original_result, gotten_result);

    Ok(())
}
