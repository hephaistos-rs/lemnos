//! VMs and containers: listing them, powering them on and off, and managing
//! their snapshots and copies.
//!
//! Proxmox calls both "guests" and numbers them from the same ID space, so
//! they share types here. [`list`] fetches all of them in a single request.
//! Everything that changes a guest only kicks off the change: it returns a
//! [`Task`](crate::tasks::Task) immediately, and you need
//! [`crate::tasks::wait`] to find out whether it worked.

use proxmox_api::ReqwestError;
use proxmox_api::cluster::ClusterClient;
use proxmox_api::cluster::resources::{GetOutputItems, GetParams, Type, Type2};
use proxmox_api::nodes::NodesClient;
use proxmox_api::nodes::node::lxc::vmid::status::current::Status as LxcStatus;
use proxmox_api::nodes::node::qemu::vmid::status::current::Status as QemuStatus;
use proxmox_api::types::bounded_integer::BoundedInteger;
use proxmox_api::types::bounded_number::BoundedNumber;

use crate::client::Client;

mod lifecycle;
mod power;
pub mod snapshots;

pub use lifecycle::*;
pub use power::*;

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
    /// Frozen with its memory kept, by [`suspend`]. [`resume`] continues it.
    Paused,
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
        status: status_from(resource.status.as_deref()),
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

fn status_from(status: Option<&str>) -> GuestStatus {
    match status {
        Some("running") => GuestStatus::Running,
        Some("stopped") => GuestStatus::Stopped,
        Some("paused") => GuestStatus::Paused,
        other => GuestStatus::Other(other.unwrap_or_default().to_string()),
    }
}

/// Live figures for a single guest, asked from the node it runs on.
#[derive(Debug, Clone)]
pub struct Current {
    pub status: GuestStatus,
    /// Set while an operation such as a backup or migration holds the guest.
    /// Most other changes are refused until it is released.
    pub lock: Option<String>,
    /// CPU usage as a fraction of `max_cpu`.
    pub cpu: Option<f64>,
    /// Number of cores assigned.
    pub max_cpu: Option<f64>,
    /// Memory in use, in bytes.
    pub mem: Option<u64>,
    /// Memory assigned, in bytes.
    pub max_mem: Option<u64>,
    /// Uptime in seconds.
    pub uptime: Option<u64>,
}

/// Fetches the current state of one guest. Unlike [`list`], this can tell a
/// paused VM from a running one.
pub async fn current(client: &Client, guest: &GuestId) -> Result<Current, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let unsigned = |value: Option<i64>| value.and_then(|value| u64::try_from(value).ok());

    Ok(match guest.kind {
        GuestKind::Qemu => {
            let status = node.qemu().vmid(guest.vmid).status();
            let current = status.current().get().await?;
            // `status` only knows running and stopped; `qmpstatus` is what
            // QEMU itself reports, and is where "paused" shows up.
            let status = match (current.qmpstatus.as_deref(), current.status) {
                (Some("paused"), _) => GuestStatus::Paused,
                (_, QemuStatus::Running) => GuestStatus::Running,
                (_, QemuStatus::Stopped) => GuestStatus::Stopped,
            };
            Current {
                status,
                lock: current.lock,
                cpu: current.cpu,
                max_cpu: current.cpus,
                mem: unsigned(current.mem),
                max_mem: unsigned(current.maxmem),
                uptime: unsigned(current.uptime),
            }
        }
        GuestKind::Lxc => {
            let status = node.lxc().vmid(guest.vmid).status();
            let current = status.current().get().await?;
            Current {
                status: match current.status {
                    LxcStatus::Running => GuestStatus::Running,
                    LxcStatus::Stopped => GuestStatus::Stopped,
                },
                lock: current.lock,
                cpu: current.cpu,
                max_cpu: current.cpus,
                mem: unsigned(current.mem),
                max_mem: unsigned(current.maxmem),
                uptime: unsigned(current.uptime),
            }
        }
    })
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
