use prometheus::{IntCounterVec, IntGaugeVec, Opts, Registry};
use wq::QueueStats;

#[derive(Clone)]
pub struct QueueCounters {
    pub jobs_counter: IntCounterVec,
    pub jobs_scheduled: IntGaugeVec,
    pub jobs_claimed: IntGaugeVec,
}

impl QueueCounters {
    pub fn new(registry: &Registry) -> ap_lobby::error::Result<Self> {
        let ret = Self {
            jobs_counter: IntCounterVec::new(
                Opts::new("queue_jobs_count", "The number of jobs per queue"),
                &["queue", "status"],
            )?,
            jobs_scheduled: IntGaugeVec::new(
                Opts::new(
                    "queue_jobs_scheduled",
                    "The current number of jobs in the queue",
                ),
                &["queue"],
            )?,
            jobs_claimed: IntGaugeVec::new(
                Opts::new("queue_jobs_claimed", "The current number of jobs claimed"),
                &["queue"],
            )?,
        };

        registry.register(Box::new(ret.jobs_counter.clone()))?;
        registry.register(Box::new(ret.jobs_scheduled.clone()))?;
        registry.register(Box::new(ret.jobs_claimed.clone()))?;

        Ok(ret)
    }

    pub fn update_queue(&self, queue_name: &str, stats: QueueStats) {
        let jobs_succeeded = self
            .jobs_counter
            .with_label_values(&[queue_name, "succeeded"]);
        jobs_succeeded.inc_by(stats.jobs_succeeded - jobs_succeeded.get());
        let jobs_failed = self.jobs_counter.with_label_values(&[queue_name, "failed"]);
        jobs_failed.inc_by(stats.jobs_failed - jobs_failed.get());
        let jobs_errored = self
            .jobs_counter
            .with_label_values(&[queue_name, "errored"]);
        jobs_errored.inc_by(stats.jobs_errored - jobs_errored.get());

        self.jobs_scheduled
            .with_label_values(&[queue_name])
            .set(stats.jobs_scheduled as i64);
        self.jobs_claimed
            .with_label_values(&[queue_name])
            .set(stats.jobs_claimed as i64);
    }
}
