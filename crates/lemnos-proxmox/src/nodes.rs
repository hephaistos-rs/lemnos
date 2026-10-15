//! The physical hosts in a cluster and their resource usage.

use proxmox_api::ReqwestError;
use proxmox_api::cluster::ClusterClient;
use proxmox_api::cluster::resources::{GetOutputItems, GetParams, Type, Type2};
use proxmox_api::types::bounded_number::BoundedNumber;

use crate::client::Client;

/// A host as it appears in the cluster resource list. The usage fields are
/// `None` while the node is offline.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub online: bool,
    /// CPU usage as a fraction of `max_cpu`, so `0.5` means half of all
    /// cores are busy.
    pub cpu: Option<f64>,
    /// Number of cores.
    pub max_cpu: Option<f64>,
    /// Memory in use, in bytes.
    pub mem: Option<u64>,
    /// Total memory, in bytes.
    pub max_mem: Option<u64>,
    /// Uptime in seconds.
    pub uptime: Option<u64>,
}

/// Lists every node in the cluster, offline ones included.
pub async fn list(client: &Client) -> Result<Vec<Node>, ReqwestError> {
    let params = GetParams {
        ty: Some(Type::Node),
        ..Default::default()
    };
    let resources = ClusterClient::new(client).resources().get(params).await?;

    Ok(resources.into_iter().filter_map(node_from).collect())
}

/// Returns `None` if the resource isn't a node.
fn node_from(resource: GetOutputItems) -> Option<Node> {
    if resource.ty != Type2::Node {
        return None;
    }

    Some(Node {
        name: resource.node?,
        online: resource.status.as_deref() == Some("online"),
        cpu: resource.cpu.map(|cpu| cpu.get()),
        max_cpu: resource.maxcpu.map(|cpu| cpu.get()),
        mem: resource.mem,
        max_mem: resource.maxmem.and_then(|mem| u64::try_from(mem).ok()),
        uptime: resource.uptime.and_then(|secs| u64::try_from(secs).ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_online_and_offline_nodes() {
        let resources: Vec<GetOutputItems> = serde_json::from_str(
            r#"[
                {"id":"node/pve1","type":"node","node":"pve1","status":"online",
                 "cpu":0.05,"maxcpu":16,"mem":8589934592,"maxmem":68719476736,
                 "disk":1073741824,"maxdisk":107374182400,"uptime":86400,
                 "level":"","cgroup-mode":2},
                {"id":"node/pve2","type":"node","node":"pve2","status":"offline"}
            ]"#,
        )
        .unwrap();
        let nodes: Vec<Node> = resources.into_iter().filter_map(node_from).collect();

        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].name, "pve1");
        assert!(nodes[0].online);
        assert_eq!(nodes[0].max_cpu, Some(16.0));
        assert_eq!(nodes[0].max_mem, Some(68719476736));
        assert!(!nodes[1].online);
        assert_eq!(nodes[1].cpu, None);
    }
}
