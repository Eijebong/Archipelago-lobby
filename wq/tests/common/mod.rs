use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    process::{Child, Command, Stdio},
    sync::atomic::AtomicU16,
    time::Duration,
};
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

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct TestWork(pub String);

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct TestWorkResult(pub String);
