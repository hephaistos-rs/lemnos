//! Copying, moving and removing guests.

use proxmox_api::ReqwestError;
use proxmox_api::nodes::NodesClient;
use proxmox_api::nodes::node::{lxc, qemu};
use proxmox_api::types::bounded_integer::BoundedInteger;

use super::{GuestId, GuestKind, VmId};
use crate::client::Client;
use crate::tasks::Task;

/// Choices for [`clone`]. Everything left as `None` is decided by Proxmox.
#[derive(Debug, Clone, Default)]
pub struct CloneOptions {
    /// Name of the copy (the hostname, for a container).
    pub name: Option<String>,
    /// `Some(true)` copies every disk in full. `Some(false)` makes a linked
    /// clone that shares the source's disks, which only templates allow.
    pub full: Option<bool>,
    /// Node to create the copy on, if not the source's own.
    pub target_node: Option<String>,
    /// Storage for the copy's disks. Only used by a full clone.
    pub storage: Option<String>,
}

/// Creates a copy of `source` with the ID `new_id`.
///
/// The task runs on the source's node, even when the copy ends up elsewhere.
pub async fn clone(
    client: &Client,
    source: &GuestId,
    new_id: VmId,
    options: CloneOptions,
) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&source.node);
    // Each endpoint has its own ID type, all with the same range as `VmId`.
    let same_range = "a VmId is always a valid new ID";

    let upid = match source.kind {
        GuestKind::Qemu => {
            let new_id = new_id.get().try_into().expect(same_range);
            let mut params = qemu::vmid::clone::PostParams::new(new_id);
            params.name = options.name;
            params.full = options.full;
            params.target = options.target_node;
            params.storage = options.storage;
            node.qemu().vmid(source.vmid).clone().post(params).await?
        }
        GuestKind::Lxc => {
            let new_id = new_id.get().try_into().expect(same_range);
            let mut params = lxc::vmid::clone::PostParams::new(new_id);
            params.hostname = options.name;
            params.full = options.full;
            params.target = options.target_node;
            params.storage = options.storage;
            node.lxc().vmid(source.vmid).clone().post(params).await?
        }
    };
    Ok(Task::new(&source.node, upid))
}

/// Moves the guest to `target_node`.
///
/// With `keep_running`, a running VM is moved live, without downtime. A
/// running container can't be moved live: it is shut down, moved and started
/// again. Without `keep_running`, a running guest is refused.
///
/// Once the task has finished, the guest's [`GuestId::node`] is `target_node`.
pub async fn migrate(
    client: &Client,
    guest: &GuestId,
    target_node: &str,
    keep_running: bool,
) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let mut params = qemu::vmid::migrate::PostParams::new(target_node.to_string());
            params.online = Some(keep_running);
            node.qemu().vmid(guest.vmid).migrate().post(params).await?
        }
        GuestKind::Lxc => {
            let mut params = lxc::vmid::migrate::PostParams::new(target_node.to_string());
            params.restart = Some(keep_running);
            node.lxc().vmid(guest.vmid).migrate().post(params).await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}

/// Deletes the guest and all of its disks. This cannot be undone. Proxmox
/// refuses while the guest is running.
///
/// With `purge`, it is also removed from backup jobs, replication and HA.
pub async fn destroy(client: &Client, guest: &GuestId, purge: bool) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let params = qemu::vmid::DeleteParams {
                purge: Some(purge),
                ..Default::default()
            };
            node.qemu().vmid(guest.vmid).delete(params).await?
        }
        GuestKind::Lxc => {
            let params = lxc::vmid::DeleteParams {
                purge: Some(purge),
                ..Default::default()
            };
            node.lxc().vmid(guest.vmid).delete(params).await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}
