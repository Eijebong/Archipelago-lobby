#[derive(Debug, PartialEq)]
pub struct QueueStats {
    pub jobs_failed: u64,
    pub jobs_succeeded: u64,
    pub jobs_errored: u64,
    pub jobs_scheduled: u64,
    pub jobs_claimed: u64,
}
