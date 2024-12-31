use std::{marker::PhantomData, time::Duration};

use anyhow::Result;
use redis::AsyncConnectionConfig;
use serde::{de::DeserializeOwned, Serialize};

use crate::WorkQueue;

pub struct WorkQueueBuilder<T, R> {
    queue_name: String,
    reclaim_timeout: Duration,
    claim_timeout: Duration,
    _phantom: PhantomData<(T, R)>,
}

impl<
        T: Serialize + DeserializeOwned + Send + Sync,
        R: Serialize + DeserializeOwned + Send + Sync,
    > WorkQueueBuilder<T, R>
{
    pub fn new(queue_name: &str) -> Self {
        Self {
            queue_name: queue_name.to_string(),
            reclaim_timeout: Duration::from_secs(30),
            claim_timeout: Duration::from_secs(30),
            _phantom: PhantomData,
        }
    }

    pub async fn build(self, valkey_conn: &str) -> Result<WorkQueue<T, R>> {
        let client = redis::Client::open(valkey_conn)?;
        let (tx, rx) = tokio::sync::broadcast::channel(1024);
        let config = AsyncConnectionConfig::new().set_push_sender(tx);
        let mut client = client
            .get_multiplexed_async_connection_with_config(&config)
            .await?;
        let queue_key = format!("wq:{}:queue", self.queue_name);
        client.subscribe(&queue_key).await?;

        Ok(WorkQueue {
            claims_key: format!("wq:{}:claims", self.queue_name),
            results_key: format!("wq:{}:results", self.queue_name),
            stats_key: format!("wq:{}:stats", self.queue_name),
            queue_key,
            reclaim_timeout: self.reclaim_timeout,
            claim_timeout: self.claim_timeout,
            client,
            pubsub_rx: rx,
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
}
