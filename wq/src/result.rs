use redis::{ErrorKind, FromRedisValue, RedisError, ToRedisArgs};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::JobStatus;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct JobResult<R> {
    pub status: JobStatus,
    pub result: R,
}

impl<R: DeserializeOwned + Serialize> ToRedisArgs for JobResult<R> {
    fn write_redis_args<W>(&self, out: &mut W)
    where
        W: ?Sized + redis::RedisWrite,
    {
        let serialized = serde_json::to_string(self).expect("Failed to serialize job result");
        String::write_redis_args(&serialized, out)
    }
}

impl<R: DeserializeOwned + Serialize> FromRedisValue for JobResult<R> {
    fn from_redis_value(v: &redis::Value) -> redis::RedisResult<Self> {
        let s = String::from_redis_value(v)?;
        let Ok(v) = serde_json::from_str(&s) else {
            return Err(RedisError::from((
                ErrorKind::TypeError,
                "Response was of incompatible type",
                format!("JobResult (response was {:?})", s),
            )));
        };

        Ok(v)
    }
}
