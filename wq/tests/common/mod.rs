use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    process::{Child, Command, Stdio},
    sync::atomic::AtomicU16,
    time::Duration,
};
use uuid::Uuid;
use wq::{JobId, JobStatus, WorkQueue};
pub struct ValkeyInstance {
    port: u16,
    process: Child,
}

impl Drop for ValkeyInstance {
    fn drop(&mut self) {
        let _ = self.process.kill();
    }
}

impl ValkeyInstance {
    pub fn url(&self) -> String {
        format!("redis://127.0.0.1:{}?protocol=resp3", self.port)
    }
}

#[allow(dead_code)]
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(30);
static PORT: AtomicU16 = AtomicU16::new(47000);

pub fn start_valkey() -> Result<ValkeyInstance> {
    let port = PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let process = Command::new("valkey-server")
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .spawn()?;

    let instance = ValkeyInstance { port, process };

    let mut tries_left = 10;
    while tries_left > 0 {
        let client = redis::Client::open(instance.url()).unwrap();
        let conn = client.get_connection();

        if conn.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
        tries_left -= 1;
    }

    if tries_left == 0 {
        panic!("Failed to start valkey on port {}", instance.port);
    }

    Ok(instance)
}

#[allow(unused)]
pub async fn queue_resolve_job(queue: &WorkQueue<TestWork, TestWorkResult>) -> Result<JobId> {
    let work_param = TestWork(Uuid::new_v4().to_string());

    let expected_job_id = queue
        .enqueue_job(&work_param, wq::Priority::Low, DEFAULT_DEADLINE)
        .await?;

    let job = queue
        .claim_job("test")
        .await?
        .expect("Should've gotten a job");

    assert_eq!(expected_job_id, job.job_id);

    queue
        .resolve_job(
            "test",
            job.job_id,
            JobStatus::Success,
            TestWorkResult(job.params.0.clone()),
        )
        .await?;

    Ok(expected_job_id)
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct TestWork(pub String);

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct TestWorkResult(pub String);
