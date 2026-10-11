//! Powering guests on and off.

use proxmox_api::ReqwestError;
use proxmox_api::nodes::NodesClient;

use super::{GuestId, GuestKind};
use crate::client::Client;
use crate::tasks::Task;

/// Starts the guest.
pub async fn start(client: &Client, guest: &GuestId) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let status = node.qemu().vmid(guest.vmid).status();
            status.start().post(Default::default()).await?
        }
        GuestKind::Lxc => {
            let status = node.lxc().vmid(guest.vmid).status();
            status.start().post(Default::default()).await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}

/// Asks the guest OS to shut down gracefully. This can take a while, and
/// fails if the guest ignores the request. Use [`stop`] to force it.
pub async fn shutdown(client: &Client, guest: &GuestId) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let status = node.qemu().vmid(guest.vmid).status();
            status.shutdown().post(Default::default()).await?
        }
        GuestKind::Lxc => {
            let status = node.lxc().vmid(guest.vmid).status();
            status.shutdown().post(Default::default()).await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}

/// Hard stop, like pulling the plug. It takes effect immediately, but the
/// guest gets no chance to shut down cleanly.
pub async fn stop(client: &Client, guest: &GuestId) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let status = node.qemu().vmid(guest.vmid).status();
            status.stop().post(Default::default()).await?
        }
        GuestKind::Lxc => {
            let status = node.lxc().vmid(guest.vmid).status();
            status.stop().post(Default::default()).await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}

/// Gracefully shuts the guest down and starts it again.
pub async fn reboot(client: &Client, guest: &GuestId) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let status = node.qemu().vmid(guest.vmid).status();
            status.reboot().post(Default::default()).await?
        }
        GuestKind::Lxc => {
            let status = node.lxc().vmid(guest.vmid).status();
            status.reboot().post(Default::default()).await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}

/// Freezes the guest where it is, keeping its memory. It uses no CPU while
/// paused, and [`resume`] continues it as if nothing happened.
pub async fn suspend(client: &Client, guest: &GuestId) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let status = node.qemu().vmid(guest.vmid).status();
            status.suspend().post(Default::default()).await?
        }
        GuestKind::Lxc => {
            let status = node.lxc().vmid(guest.vmid).status();
            status.suspend().post().await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}

/// Continues a guest that [`suspend`] paused.
pub async fn resume(client: &Client, guest: &GuestId) -> Result<Task, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let upid = match guest.kind {
        GuestKind::Qemu => {
            let status = node.qemu().vmid(guest.vmid).status();
            status.resume().post(Default::default()).await?
        }
        GuestKind::Lxc => {
            let status = node.lxc().vmid(guest.vmid).status();
            status.resume().post().await?
        }
    };
    Ok(Task::new(&guest.node, upid))
}
