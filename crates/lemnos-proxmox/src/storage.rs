//! Where disks, ISOs and backups are kept, and how full it is.

use proxmox_api::ReqwestError;
use proxmox_api::cluster::ClusterClient;
use proxmox_api::cluster::resources::{GetOutputItems, GetParams, Type, Type2};

use crate::client::Client;

/// A storage as one node sees it.
#[derive(Debug, Clone)]
pub struct Storage {
    pub name: String,
    pub node: String,
    /// The backend, e.g. `dir`, `lvmthin`, `zfspool` or `nfs`.
    pub kind: Option<String>,
    /// What may be stored here, e.g. `images`, `iso` or `backup`.
    pub content: Vec<String>,
    /// Whether the node can reach it right now.
    pub available: bool,
    /// Space in use, in bytes.
    pub used: Option<u64>,
    /// Total size, in bytes.
    pub total: Option<u64>,
}

/// Lists every storage on every node.
///
/// A storage shared between nodes, such as an NFS share, is listed once per
/// node that has it, each time with the same name.
pub async fn list(client: &Client) -> Result<Vec<Storage>, ReqwestError> {
    let params = GetParams {
        ty: Some(Type::Storage),
        ..Default::default()
    };
    let resources = ClusterClient::new(client).resources().get(params).await?;

    Ok(resources.into_iter().filter_map(storage_from).collect())
}

/// Returns `None` if the resource isn't a storage.
fn storage_from(resource: GetOutputItems) -> Option<Storage> {
    if resource.ty != Type2::Storage {
        return None;
    }

    Some(Storage {
        name: resource.storage?,
        node: resource.node?,
        kind: resource.plugintype,
        content: resource
            .content
            .iter()
            .flat_map(|content| content.split(','))
            .filter(|content| !content.is_empty())
            .map(str::to_string)
            .collect(),
        available: resource.status.as_deref() == Some("available"),
        used: resource.disk,
        total: resource.maxdisk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_storage() {
        let resources: Vec<GetOutputItems> = serde_json::from_str(
            r#"[
                {"id":"storage/pve1/local","type":"storage","node":"pve1",
                 "storage":"local","status":"available","plugintype":"dir",
                 "content":"iso,vztmpl,backup","shared":0,
                 "disk":10737418240,"maxdisk":107374182400},
                {"id":"node/pve1","type":"node","node":"pve1","status":"online"}
            ]"#,
        )
        .unwrap();
        let storages: Vec<Storage> = resources.into_iter().filter_map(storage_from).collect();

        assert_eq!(storages.len(), 1);
        assert_eq!(storages[0].name, "local");
        assert_eq!(storages[0].node, "pve1");
        assert_eq!(storages[0].kind.as_deref(), Some("dir"));
        assert_eq!(storages[0].content, ["iso", "vztmpl", "backup"]);
        assert!(storages[0].available);
        assert_eq!(storages[0].used, Some(10737418240));
        assert_eq!(storages[0].total, Some(107374182400));
    }
}
