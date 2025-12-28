use std::{marker::PhantomData, time::Duration};

use anyhow::Result;
use deadpool_redis::{Config, Runtime};
use serde::{de::DeserializeOwned, Serialize};

use crate::{ResolveCallback, WorkQueue};

pub struct WorkQueueBuilder<T, R: Clone> {
    queue_name: String,
    reclaim_timeout: Duration,
    claim_timeout: Duration,
    result_callback: Option<ResolveCallback<T, R>>,
    _phantom: PhantomData<(T, R)>,
}

impl<
        T: Serialize + DeserializeOwned + Send + Sync,
        R: Serialize + DeserializeOwned + Send + Sync + Clone,
    > WorkQueueBuilder<T, R>
{
    pub fn new(queue_name: &str) -> Self {
        Self {
            queue_name: queue_name.to_string(),
            reclaim_timeout: Duration::from_secs(30),
            claim_timeout: Duration::from_secs(30),
            result_callback: None,
            _phantom: PhantomData,
        }
    }

    pub async fn build(self, valkey_conn: &str) -> Result<WorkQueue<T, R>> {
        let pool_config = Config::from_url(valkey_conn);
        let pool = pool_config.create_pool(Some(Runtime::Tokio1))?;

        // Client for creating pubsub connections
        let redis_client = redis::Client::open(valkey_conn)?;

        Ok(WorkQueue {
            claims_key: format!("wq:{}:claims", self.queue_name),
            results_key: format!("wq:{}:results", self.queue_name),
            stats_key: format!("wq:{}:stats", self.queue_name),
            queue_key: format!("wq:{}:queue", self.queue_name),
            reclaim_timeout: self.reclaim_timeout,
            claim_timeout: self.claim_timeout,
            pool,
            redis_client,
            result_callback: self.result_callback,
            _phantom: PhantomData,
        })
    }

    pub fn with_reclaim_timeout(mut self, timeout: Duration) -> Self {
        self.reclaim_timeout = timeout;
        self
    }

    pub fn with_claim_timeout(mut self, timeout: Duration) -> Self {
        self.claim_timeout = timeout;
        self
    }

    pub fn with_callback(mut self, callback: ResolveCallback<T, R>) -> Self {
        self.result_callback = Some(callback);
        self
    }
}
