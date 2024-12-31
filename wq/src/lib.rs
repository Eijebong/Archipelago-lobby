use std::{
    collections::HashMap,
    fmt::Display,
    marker::PhantomData,
    str::from_utf8,
    time::{Duration, Instant},
};

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use redis::{
    aio::MultiplexedConnection, AsyncCommands, ErrorKind, FromRedisValue, PushInfo, PushKind,
    RedisError,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::{sync::broadcast, task::JoinHandle};
use uuid::Uuid;

mod builder;
mod claim;
mod result;
mod stats;

pub use builder::WorkQueueBuilder;
pub use claim::Claim;
pub use result::JobResult;
pub use stats::QueueStats;

#[repr(i8)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Priority {
    High = -10,
    Normal = -5,
    Low = -1,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum JobStatus {
    Pending = 0,
    Running = 1,
    Success = 10,
    Failure = 11,
    InternalError = 12,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Job<P> {
    pub job_id: JobId,
    pub params: P,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobDesc<P> {
    params: P,
    deadline: DateTime<Utc>,
}

impl JobStatus {
    pub fn is_resolved(&self) -> bool {
        matches!(self, Self::Success | Self::Failure | Self::InternalError)
    }

    pub fn as_stat_name(&self) -> &str {
        assert!(self.is_resolved());
        match self {
            JobStatus::Success => "succeeded",
            JobStatus::Failure => "failed",
            JobStatus::InternalError => "errored",
            _ => unreachable!(),
        }
    }
}

impl TryFrom<u8> for JobStatus {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Pending,
            1 => Self::Running,
            10 => Self::Success,
            11 => Self::Failure,
            12 => Self::InternalError,
            status => bail!("Invalid job status: {}", status),
        })
    }
}

impl FromRedisValue for Priority {
    fn from_redis_value(v: &redis::Value) -> redis::RedisResult<Self> {
        let value = i8::from_redis_value(v)?;
        let priority = match value {
            -10 => Priority::High,
            -5 => Priority::Normal,
            -1 => Priority::Low,
            v => {
                return Err(RedisError::from((
                    ErrorKind::ParseError,
                    "Invalid priority received",
                    format!("Received priority number: {}", v),
                )));
            }
        };

        Ok(priority)
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
#[serde(transparent)]
pub struct JobId(Uuid);

impl From<Uuid> for JobId {
    fn from(value: Uuid) -> Self {
        JobId(value)
    }
}

impl JobId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.0))
    }
}

impl FromRedisValue for JobId {
    fn from_redis_value(v: &redis::Value) -> redis::RedisResult<Self> {
        let id_str = String::from_redis_value(v)?;
        let Ok(id) = Uuid::parse_str(&id_str) else {
            return Err(RedisError::from((
                ErrorKind::TypeError,
                "Response was of incompatible type",
                format!("UUID (response was {:?})", id_str),
            )));
        };

        Ok(Self(id))
    }
}

pub struct WorkQueue<
    P: Serialize + DeserializeOwned + Send + Sync,
    R: Serialize + DeserializeOwned + Send + Sync,
> {
    queue_key: String,
    claims_key: String,
    results_key: String,
    stats_key: String,
    client: MultiplexedConnection,
    reclaim_timeout: Duration,
    claim_timeout: Duration,
    pubsub_rx: broadcast::Receiver<PushInfo>,
    _phantom: PhantomData<(P, R)>,
}

impl<
        P: Serialize + DeserializeOwned + Send + Sync + 'static,
        R: Serialize + DeserializeOwned + Send + Sync + 'static,
    > WorkQueue<P, R>
{
    async fn reclaim_checker_inner(
        mut conn: MultiplexedConnection,
        reclaim_timeout: Duration,
        claims_key: String,
        queue_key: String,
    ) {
        loop {
            tokio::time::sleep(reclaim_timeout / 2).await;

            let queue_claims = match conn.hgetall::<_, HashMap<String, Claim>>(&claims_key).await {
                Ok(claims) => claims,
                Err(e) => {
                    tracing::error!("Error while listing claims for queue {}: {}", queue_key, e);
                    continue;
                }
            };

            if queue_claims.is_empty() {
                continue;
            }

            let now = Utc::now();
            for claim in queue_claims.values() {
                if now
                    .signed_duration_since(claim.time)
                    .abs()
                    .to_std()
                    .unwrap()
                    > reclaim_timeout
                {
                    tracing::warn!(
                        "Claim for job {} by worker {} expired. Reinserting in queue at {}",
                        claim.job_id,
                        claim.worker_id,
                        queue_key
                    );

                    let job_claim_key = format!("{}:{}", claims_key, &claim.job_id);
                    let res = redis::pipe()
                        .zadd(&queue_key, claim.job_id.to_string(), claim.priority as i8)
                        .del(job_claim_key)
                        .publish(&queue_key, 0)
                        .exec_async(&mut conn)
                        .await;

                    if let Err(e) = res {
                        tracing::error!(
                            "Error while reinserting job {} in queue after its claim expired: {}",
                            claim.job_id,
                            e
                        );
                    }
                }
            }
        }
    }

    pub fn start_reclaim_checker(&self) -> JoinHandle<()> {
        let conn = self.client.clone();

        tokio::spawn(Self::reclaim_checker_inner(
            conn,
            self.reclaim_timeout,
            self.claims_key.clone(),
            self.queue_key.clone(),
        ))
    }

    pub async fn reclaim_job(&self, job_id: &JobId, worker_id: &str) -> Result<()> {
        let mut conn = self.client.clone();

        let Some(mut current_claim) = conn
            .hget::<_, _, Option<Claim>>(&self.claims_key, job_id.to_string())
            .await?
        else {
            bail!("Job has been cancelled");
        };
        current_claim.refresh(worker_id)?;
        conn.hset::<_, _, _, ()>(&self.claims_key, job_id.to_string(), current_claim)
            .await?;

        Ok(())
    }

    /// Resolve the job `job_id` by setting its status and result. This will effectively remove the
    /// job from the queue and claims, set the result and notify everyone waiting on the job ID.
    pub async fn resolve_job(
        &self,
        worker_id: &str,
        job_id: JobId,
        status: JobStatus,
        result: R,
    ) -> Result<()> {
        let mut conn = self.client.clone();

        if !status.is_resolved() {
            bail!("Trying to report a status that doesn't resolve the job");
        }

        let Some(current_claim) = conn
            .hget::<_, _, Option<Claim>>(&self.claims_key, job_id.to_string())
            .await?
        else {
            bail!("Trying to resolve a job that isn't claimed");
        };

        if current_claim.worker_id != worker_id {
            bail!("Trying to resolve a job that isn't yours");
        }

        let job_result = JobResult { status, result };

        redis::pipe()
            .del(self.get_job_key(&job_id))
            .hdel(&self.claims_key, job_id.to_string())
            .set::<_, JobResult<R>>(self.get_result_key(&job_id), job_result)
            .incr(self.get_stats_key(status.as_stat_name()), 1)
            .publish::<_, u8>(self.get_result_key(&job_id), status as u8)
            .exec_async(&mut conn)
            .await?;

        Ok(())
    }

    /// Return the job's status. Returns `Ok(None)` if the job doesn't exist in the queue nor in
    /// the results.
    pub async fn get_job_status(&self, job_id: &JobId) -> Result<Option<JobStatus>> {
        let mut conn = self.client.clone();
        let result_key = self.get_result_key(job_id);

        let (is_resolved, is_claimed, is_queued): (bool, bool, bool) = redis::pipe()
            .exists(self.get_result_key(job_id))
            .hexists(&self.claims_key, job_id.to_string())
            .exists(self.get_job_key(job_id))
            .query_async(&mut conn)
            .await?;

        if is_resolved {
            let job_result = conn.get::<_, JobResult<R>>(result_key).await?;
            return Ok(Some(job_result.status));
        }

        if is_claimed {
            return Ok(Some(JobStatus::Running));
        }

        if is_queued {
            return Ok(Some(JobStatus::Pending));
        }

        Ok(None)
    }

    /// Wait for a job to complete. If timeout is None, a default timeout of one day will be used.
    /// Returns `Ok(None)` on timeout.
    pub async fn wait_for_job(
        &self,
        job_id: &JobId,
        timeout: Option<Duration>,
    ) -> Result<Option<JobStatus>> {
        let job_status = self.get_job_status(job_id).await?;
        if job_status.is_none() {
            bail!(
                "Trying to wait for a job that doesn't exist in the queue or which result has already been processed"
            );
        }

        let mut conn = self.client.clone();

        let mut remaining_time = timeout.unwrap_or(Duration::from_secs(3600 * 24));
        let start = Instant::now();

        let channel_name = self.get_result_key(job_id);
        conn.subscribe(&channel_name).await?;

        let job_result = conn
            .get::<_, Option<JobResult<R>>>(self.get_result_key(job_id))
            .await?;
        if let Some(result) = job_result {
            conn.unsubscribe(&channel_name).await?;
            return Ok(Some(result.status));
        }

        let status = loop {
            let result =
                tokio::time::timeout(remaining_time, self.pubsub_rx.resubscribe().recv()).await;

            let Ok(result) = result else { return Ok(None) };

            let result = result?;
            let Some(new_remaining_time) = remaining_time.checked_sub(Instant::now() - start)
            else {
                return Ok(None);
            };
            remaining_time = new_remaining_time;

            if result.kind != PushKind::Message {
                continue;
            }

            let mut data = result.data.into_iter();
            let Some(redis::Value::BulkString(raw_channel)) = data.next() else {
                continue;
            };
            let Ok(recv_channel) = from_utf8(&raw_channel) else {
                tracing::warn!("Invalid channel name on pubsub: {:?}", raw_channel);
                continue;
            };

            if recv_channel != channel_name {
                continue;
            }

            let Some(redis::Value::BulkString(raw_status)) = data.next() else {
                continue;
            };
            let Ok(status) = from_utf8(&raw_status) else {
                tracing::warn!("Invalid status on pubsub: {:?}", raw_status);
                conn.unsubscribe(&channel_name).await?;
                return Ok(Some(JobStatus::InternalError));
            };

            let Ok(status) = status.parse::<u8>() else {
                tracing::warn!("Invalid status on pubsub: {:?}", status);
                conn.unsubscribe(&channel_name).await?;
                return Ok(Some(JobStatus::InternalError));
            };

            break status;
        };

        conn.unsubscribe(&channel_name).await?;

        Ok(Some(JobStatus::try_from(status)?))
    }

    /// Returns the result for the given job id
    pub async fn get_job_result(&self, job_id: JobId) -> Result<R> {
        let mut conn = self.client.clone();
        let result_key = self.get_result_key(&job_id);
        let result_str = conn.get::<_, String>(result_key).await?;
        dbg!(&result_str);

        Ok(serde_json::from_str::<JobResult<R>>(&result_str)?.result)
    }

    /// Tries to getu a job for 30s and returns `Ok(None)` if nothing shows up.
    /// When a job is claimed, register the worker's claim on the job.
    pub async fn claim_job(&self, worker_id: &str) -> Result<Option<Job<P>>> {
        tracing::trace!("Worker {} is trying to claim a job", worker_id);

        let mut conn = self.client.clone();
        let mut remaining_time = self.claim_timeout;
        let start = Instant::now();

        let (job_id, priority, params) = loop {
            let result = conn
                .zpopmin::<_, Vec<(JobId, Priority)>>(&self.queue_key, 1)
                .await?;

            if let Some(result) = result.first() {
                let (job_id, priority) = result;
                let params_str = conn.get::<_, String>(self.get_job_key(job_id)).await?;
                let desc: JobDesc<P> = serde_json::from_str(&params_str)?;
                if Utc::now() > desc.deadline {
                    self.cancel_job(*job_id).await?;
                    continue;
                }

                break (*job_id, *priority, desc.params);
            };

            let result =
                tokio::time::timeout(remaining_time, self.pubsub_rx.resubscribe().recv()).await;

            if result.is_err() {
                // Timeout
                return Ok(None);
            };

            let Some(new_remaining_time) = remaining_time.checked_sub(Instant::now() - start)
            else {
                return Ok(None);
            };
            remaining_time = new_remaining_time;
        };

        let claim = Claim::new(worker_id, job_id, priority);
        conn.hset::<_, _, _, ()>(&self.claims_key, job_id.to_string(), claim)
            .await?;
        tracing::info!("Gave job {} to worker {}", job_id, worker_id);

        let job = Job { job_id, params };

        Ok(Some(job))
    }

    /// Add a job ID to the ordered set `wq:{name}:queue`, with priority as the score.
    /// This also creates a new key named `wq:{name}:{job_id}` containing the job's parameters
    pub async fn enqueue_job(
        &self,
        params: &P,
        priority: Priority,
        deadline_in: Duration,
    ) -> Result<JobId> {
        let mut conn = self.client.clone();

        let job_id = JobId::new();
        tracing::info!(
            "Enqueuing job with id {} and priority {:?}",
            job_id,
            priority
        );

        let job_key = self.get_job_key(&job_id);
        let job_desc = JobDesc {
            params,
            deadline: Utc::now() + deadline_in,
        };
        let job_desc_str = serde_json::to_string(&job_desc)?;

        tracing::trace!(
            "Adding job {} to queue at {} with priority {}",
            job_id,
            self.queue_key,
            priority as i8
        );

        redis::pipe()
            .set(&job_key, job_desc_str)
            .zadd(&self.queue_key, job_id.to_string(), priority as i8)
            .publish(&self.queue_key, 0)
            .exec_async(&mut conn)
            .await?;

        Ok(job_id)
    }

    /// Cancel a job. If the job is claimed, the worker will receive an error on reclaim and should
    /// actually cancel the job at that moment.
    pub async fn cancel_job(&self, job_id: JobId) -> Result<()> {
        let mut conn = self.client.clone();

        tracing::info!("Cancelling job {}", job_id);

        redis::pipe()
            .zrem(&self.queue_key, job_id.to_string())
            .del(self.get_job_key(&job_id))
            .hdel(&self.claims_key, job_id.to_string())
            .exec_async(&mut conn)
            .await?;

        Ok(())
    }

    pub async fn get_stats(&self) -> Result<QueueStats> {
        let mut conn = self.client.clone();

        let jobs_failed = conn
            .get::<_, Option<u64>>(self.get_stats_key("failed"))
            .await?
            .unwrap_or(0);
        let jobs_succeeded = conn
            .get::<_, Option<u64>>(self.get_stats_key("succeeded"))
            .await?
            .unwrap_or(0);
        let jobs_errored = conn
            .get::<_, Option<u64>>(self.get_stats_key("errored"))
            .await?
            .unwrap_or(0);
        let jobs_scheduled = conn
            .zcount(&self.queue_key, Priority::High as i8, Priority::Low as i8)
            .await?;
        let jobs_claimed = conn.hlen(&self.claims_key).await?;

        Ok(QueueStats {
            jobs_failed,
            jobs_succeeded,
            jobs_errored,
            jobs_scheduled,
            jobs_claimed,
        })
    }

    pub fn get_job_key(&self, job_id: &JobId) -> String {
        format!("{}:{}", self.queue_key, &job_id)
    }

    pub fn get_result_key(&self, job_id: &JobId) -> String {
        format!("{}:{}", self.results_key, &job_id)
    }

    pub fn get_stats_key(&self, stat_name: &str) -> String {
        format!("{}:{}", self.stats_key, stat_name)
    }

    pub fn builder(name: &str) -> WorkQueueBuilder<P, R> {
        WorkQueueBuilder::new(name)
    }
}
