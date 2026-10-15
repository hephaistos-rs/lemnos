//! Snapshots: saved states of a guest that it can be rolled back to.

use proxmox_api::ReqwestError;
use proxmox_api::nodes::NodesClient;
use proxmox_api::nodes::node::{lxc, qemu};

use super::{GuestId, GuestKind};
use crate::client::Client;
use crate::tasks::Task;

/// The entry Proxmox adds to every snapshot list to mark where the guest is
/// now. It isn't a snapshot, so [`list`] leaves it out.
const CURRENT: &str = "current";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub name: String,
    pub description: String,
    /// The snapshot this one was taken on top of, if any.
    pub parent: Option<String>,
    /// When it was taken, in seconds since the Unix epoch.
    pub taken_at: Option<i64>,
    /// Whether the VM's memory was saved too, so a rollback resumes it
    /// running. Always `false` for containers.
    pub includes_memory: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the request to Proxmox failed: {0}")]
    Api(#[from] ReqwestError),
    #[error("`{0}` is not a valid snapshot name: it can be at most 40 characters")]
    InvalidName(String),
}

/// Lists the guest's snapshots.
pub async fn list(client: &Client, guest: &GuestId) -> Result<Vec<Snapshot>, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);

    let snapshots: Vec<Snapshot> = match guest.kind {
        GuestKind::Qemu => {
            let found = node.qemu().vmid(guest.vmid).snapshot().get().await?;
            let found = found.into_iter().map(|snapshot| Snapshot {
                name: snapshot.name,
                description: snapshot.description,
                parent: snapshot.parent,
                taken_at: snapshot.snaptime,
                includes_memory: snapshot.vmstate.unwrap_or(false),
            });
            found.collect()
        }
        GuestKind::Lxc => {
            let found = node.lxc().vmid(guest.vmid).snapshot().get().await?;
            let found = found.into_iter().map(|snapshot| Snapshot {
                name: snapshot.name,
                description: snapshot.description,
                parent: snapshot.parent,
                taken_at: snapshot.snaptime,
                includes_memory: false,
            });
            found.collect()
        }
    };

    Ok(without_current(snapshots))
}

fn without_current(snapshots: Vec<Snapshot>) -> Vec<Snapshot> {
    let mut snapshots = snapshots;
    snapshots.retain(|snapshot| snapshot.name != CURRENT);
    snapshots
}

/// Takes a snapshot called `name`. With `include_memory`, a running VM's
/// memory is saved as well; containers ignore it.
pub async fn create(
    client: &Client,
    guest: &GuestId,
    name: &str,
    description: Option<&str>,
    include_memory: bool,
) -> Result<Task, Error> {
    let node = NodesClient::new(client).node(&guest.node);
    let invalid = |_| Error::InvalidName(name.to_string());
    let description = description.map(str::to_string);

    let upid = match guest.kind {
        GuestKind::Qemu => {
            let name = name.to_string().try_into().map_err(invalid)?;
            let mut params = qemu::vmid::snapshot::PostParams::new(name);
            params.description = description;
            params.vmstate = Some(include_memory);
            node.qemu().vmid(guest.vmid).snapshot().post(params).await?
        }
        GuestKind::Lxc => {
            let name = name.to_string().try_into().map_err(invalid)?;
            let mut params = lxc::vmid::snapshot::PostParams::new(name);
            params.description = description;
            node.lxc().vmid(guest.vmid).snapshot().post(params).await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}

/// Puts the guest back to how it was when the snapshot was taken. Everything
/// that changed since is lost.
pub async fn rollback(client: &Client, guest: &GuestId, name: &str) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let snapshot = node.qemu().vmid(guest.vmid).snapshot().snapname(name);
            snapshot.rollback().post(Default::default()).await?
        }
        GuestKind::Lxc => {
            let snapshot = node.lxc().vmid(guest.vmid).snapshot().snapname(name);
            snapshot.rollback().post(Default::default()).await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}

/// Deletes the snapshot. The guest itself is left as it is.
pub async fn delete(client: &Client, guest: &GuestId, name: &str) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let snapshot = node.qemu().vmid(guest.vmid).snapshot().snapname(name);
            snapshot.delete(Default::default()).await?
        }
        GuestKind::Lxc => {
            let snapshot = node.lxc().vmid(guest.vmid).snapshot().snapname(name);
            snapshot.delete(Default::default()).await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_marker_is_not_a_snapshot() {
        let snapshot = |name: &str| Snapshot {
            name: name.to_string(),
            description: String::new(),
            parent: None,
            taken_at: None,
            includes_memory: false,
        };

        let kept = without_current(vec![snapshot("before-upgrade"), snapshot("current")]);

        assert_eq!(kept, [snapshot("before-upgrade")]);
    }
}
