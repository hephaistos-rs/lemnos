//! VMs and containers: listing them and powering them on and off.
//!
//! Proxmox calls both "guests" and numbers them from the same ID space, so
//! they share types here. [`list`] fetches all of them in a single request.
//! The power functions only kick off the change: they return a [`Task`]
//! immediately, and you need [`crate::tasks::wait`] to find out whether it
//! worked.

use proxmox_api::ReqwestError;
use proxmox_api::cluster::ClusterClient;
use proxmox_api::cluster::resources::{GetOutputItems, GetParams, Type, Type2};
use proxmox_api::nodes::NodesClient;
use proxmox_api::types::bounded_integer::BoundedInteger;
use proxmox_api::types::bounded_number::BoundedNumber;

use crate::client::Client;
use crate::tasks::Task;

/// A guest's numeric ID, 100 or higher. Create one with `VmId::try_from(100)`.
pub use proxmox_api::types::VmId;

/// Whether a guest is a VM or a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestKind {
    /// A full virtual machine (QEMU/KVM).
    Qemu,
    /// A Linux container (LXC).
    Lxc,
}

/// Everything you need to address a single guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestId {
    /// The node the guest is currently on. This changes when it migrates.
    pub node: String,
    pub vmid: VmId,
    pub kind: GuestKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestStatus {
    Running,
    Stopped,
    /// Any other status Proxmox reports, passed through as-is.
    Other(String),
}

/// A guest as it appears in the cluster resource list.
#[derive(Debug, Clone)]
pub struct Guest {
    pub id: GuestId,
    pub name: Option<String>,
    pub status: GuestStatus,
    /// Templates can be cloned but not started.
    pub template: bool,
    /// CPU usage as a fraction of `max_cpu`, so `0.5` means half of all
    /// cores are busy.
    pub cpu: Option<f64>,
    /// Number of cores assigned.
    pub max_cpu: Option<f64>,
    /// Memory in use, in bytes.
    pub mem: Option<u64>,
    /// Memory assigned, in bytes.
    pub max_mem: Option<u64>,
    /// Uptime in seconds.
    pub uptime: Option<u64>,
    pub tags: Vec<String>,
}

/// Lists every VM and container in the cluster.
pub async fn list(client: &Client) -> Result<Vec<Guest>, ReqwestError> {
    let params = GetParams {
        ty: Some(Type::Vm),
        ..Default::default()
    };
    let resources = ClusterClient::new(client).resources().get(params).await?;

    Ok(resources.into_iter().filter_map(guest_from).collect())
}

/// Returns `None` if the resource isn't a guest, or is missing something
/// [`GuestId`] needs.
fn guest_from(resource: GetOutputItems) -> Option<Guest> {
    let kind = match resource.ty {
        Type2::Qemu => GuestKind::Qemu,
        Type2::Lxc => GuestKind::Lxc,
        _ => return None,
    };
    let vmid = VmId::try_from(resource.vmid?.get()).ok()?;

    Some(Guest {
        id: GuestId {
            node: resource.node?,
            vmid,
            kind,
        },
        name: resource.name,
        status: match resource.status.as_deref() {
            Some("running") => GuestStatus::Running,
            Some("stopped") => GuestStatus::Stopped,
            other => GuestStatus::Other(other.unwrap_or_default().to_string()),
        },
        template: resource.template.unwrap_or(false),
        cpu: resource.cpu.map(|cpu| cpu.get()),
        max_cpu: resource.maxcpu.map(|cpu| cpu.get()),
        mem: resource.mem,
        max_mem: resource.maxmem.and_then(|mem| u64::try_from(mem).ok()),
        uptime: resource.uptime.and_then(|secs| u64::try_from(secs).ok()),
        tags: resource
            .tags
            .iter()
            .flat_map(|tags| tags.split([';', ',']))
            .filter(|tag| !tag.is_empty())
            .map(str::to_string)
            .collect(),
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn guests(json: &str) -> Vec<Guest> {
        let resources: Vec<GetOutputItems> = serde_json::from_str(json).unwrap();
        resources.into_iter().filter_map(guest_from).collect()
    }

    #[test]
    fn reads_a_vm_and_a_container() {
        let guests = guests(
            r#"[
                {"id":"qemu/100","type":"qemu","vmid":100,"node":"pve1","name":"web",
                 "status":"running","template":0,"cpu":0.25,"maxcpu":4,
                 "mem":1073741824,"maxmem":4294967296,"disk":0,"maxdisk":34359738368,
                 "uptime":3600,"netin":1,"netout":2,"diskread":0,"diskwrite":0,
                 "tags":"prod;web"},
                {"id":"lxc/101","type":"lxc","vmid":101,"node":"pve2","name":"dns",
                 "status":"stopped","template":0,"cpu":0,"maxcpu":1,
                 "mem":0,"maxmem":536870912,"disk":0,"maxdisk":8589934592,"uptime":0}
            ]"#,
        );

        assert_eq!(guests.len(), 2);

        let web = &guests[0];
        assert_eq!(web.id.node, "pve1");
        assert_eq!(web.id.vmid, VmId::try_from(100).unwrap());
        assert_eq!(web.id.kind, GuestKind::Qemu);
        assert_eq!(web.name.as_deref(), Some("web"));
        assert_eq!(web.status, GuestStatus::Running);
        assert_eq!(web.cpu, Some(0.25));
        assert_eq!(web.max_mem, Some(4294967296));
        assert_eq!(web.tags, ["prod", "web"]);

        let dns = &guests[1];
        assert_eq!(dns.id.kind, GuestKind::Lxc);
        assert_eq!(dns.status, GuestStatus::Stopped);
        assert!(dns.tags.is_empty());
    }

    #[test]
    fn skips_resources_that_are_not_guests() {
        let guests = guests(
            r#"[{"id":"storage/pve1/local","type":"storage","node":"pve1",
                 "storage":"local","status":"available"}]"#,
        );

        assert!(guests.is_empty());
    }
}
