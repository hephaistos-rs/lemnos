//! Tracking the background tasks Proxmox spawns for slow operations.
//!
//! Things like starting a guest or taking a snapshot return immediately with
//! a task ID (a "UPID") while the actual work continues in the background.
//! So having a [`Task`] only tells you Proxmox accepted the request. [`wait`]
//! polls until the task finishes and tells you whether it succeeded, and
//! [`log`] shows what it printed along the way.

use std::time::Duration;

use proxmox_api::ReqwestError;
use proxmox_api::nodes::NodesClient;
use proxmox_api::nodes::node::tasks::upid::log::GetParams;
use proxmox_api::nodes::node::tasks::upid::status::Status;
use tokio::time::Instant;

use crate::client::{Client, no_data};

/// How often [`wait`] polls the task's status.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// A task running on a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// The node running the task. Only this node can report its status.
    pub node: String,
    pub upid: String,
}

impl Task {
    pub fn new(node: impl Into<String>, upid: impl Into<String>) -> Self {
        Self {
            node: node.into(),
            upid: upid.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read the task's status: {0}")]
    Api(#[from] ReqwestError),
    /// The task finished with an error. The message comes straight from
    /// Proxmox.
    #[error("task failed: {0}")]
    Failed(String),
    /// The task is still running. It wasn't cancelled, so you can wait on it
    /// again.
    #[error("task was still running after {0:?}")]
    TimedOut(Duration),
}

/// Waits up to `timeout` for `task` to finish.
pub async fn wait(client: &Client, task: &Task, timeout: Duration) -> Result<(), Error> {
    let tasks = NodesClient::new(client).node(&task.node).tasks();
    let status = tasks.upid(&task.upid).status();
    let deadline = Instant::now() + timeout;

    loop {
        let current = status.get().await?;
        if current.status == Status::Stopped {
            return outcome(current.exitstatus);
        }
        if Instant::now() >= deadline {
            return Err(Error::TimedOut(timeout));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Asks Proxmox to abort a running task. What it had already done is not
/// undone, and [`wait`] afterwards reports the task as failed.
pub async fn cancel(client: &Client, task: &Task) -> Result<(), ReqwestError> {
    let tasks = NodesClient::new(client).node(&task.node).tasks();
    no_data(tasks.upid(&task.upid).delete()).await
}

/// Fetches the task's output, one entry per line and at most `max_lines` of
/// them from the start. This is where the reason for a failure is spelled out.
pub async fn log(
    client: &Client,
    task: &Task,
    max_lines: u32,
) -> Result<Vec<String>, ReqwestError> {
    let tasks = NodesClient::new(client).node(&task.node).tasks();
    let params = GetParams {
        limit: i128::from(max_lines).try_into().ok(),
        ..Default::default()
    };
    let lines = tasks.upid(&task.upid).log().get(params).await?;

    Ok(lines.into_iter().map(|line| line.t).collect())
}

/// Turns a finished task's exit status into a `Result`.
fn outcome(exitstatus: Option<String>) -> Result<(), Error> {
    match exitstatus.as_deref() {
        // "WARNINGS: 2" means the task finished but logged some warnings.
        Some(status) if status == "OK" || status.starts_with("WARNINGS") => Ok(()),
        Some(status) => Err(Error::Failed(status.to_string())),
        None => Err(Error::Failed("no exit status reported".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_and_warnings_count_as_success() {
        assert!(outcome(Some("OK".into())).is_ok());
        assert!(outcome(Some("WARNINGS: 2".into())).is_ok());
    }

    #[test]
    fn anything_else_is_a_failure() {
        let error = outcome(Some("VM 100 already running".into())).unwrap_err();
        assert!(matches!(error, Error::Failed(text) if text == "VM 100 already running"));

        assert!(matches!(outcome(None), Err(Error::Failed(_))));
    }
}
