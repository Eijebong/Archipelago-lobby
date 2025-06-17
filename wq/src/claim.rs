use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use redis::{ErrorKind, FromRedisValue, RedisError, ToRedisArgs};
use serde::{Deserialize, Serialize};

use crate::{JobId, Priority};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Claim {
    pub job_id: JobId,
    pub priority: Priority,
    pub worker_id: String,
    pub time: DateTime<Utc>,
}

impl Claim {
    pub fn new(worker_id: &str, job_id: JobId, priority: Priority) -> Self {
        Self {
            job_id,
            priority,
            worker_id: worker_id.to_string(),
            time: Utc::now(),
        }
    }

    pub fn refresh(&mut self, worker_id: &str) -> Result<()> {
        if worker_id != self.worker_id {
            bail!(
                "Worker {} tried to refresh a claim that isn't theirs (owner is {})",
                worker_id,
                self.worker_id
            );
        }

        self.time = Utc::now();

        Ok(())
    }
}

impl ToRedisArgs for Claim {
    fn write_redis_args<W>(&self, out: &mut W)
    where
        W: ?Sized + redis::RedisWrite,
    {
        let serialized = serde_json::to_string(self).expect("Failed to serialize claim");
        String::write_redis_args(&serialized, out)
    }
}

impl FromRedisValue for Claim {
    fn from_redis_value(v: &redis::Value) -> redis::RedisResult<Self> {
        let s = String::from_redis_value(v)?;
        let Ok(v) = serde_json::from_str(&s) else {
            return Err(RedisError::from((
                ErrorKind::TypeError,
                "Response was of incompatible type",
                format!("Claim (response was {:?})", s),
            )));
        };

        Ok(v)
    }
}
