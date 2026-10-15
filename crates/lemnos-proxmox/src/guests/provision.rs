//! Making new guests for people: which templates there are to start from,
//! and turning one into a guest of its own.
//!
//! This is the whole of what Lemnos needs from a provider to hand out
//! machines: a list of starting points, and "make me one of those, called
//! this, marked with these tags". Everything else (who may, and how many) is
//! decided above this crate.

use std::time::Duration;

use proxmox_api::ReqwestError;
use proxmox_api::cluster::ClusterClient;
use proxmox_api::cluster::nextid::GetParams as NextIdParams;
use proxmox_api::nodes::NodesClient;
use proxmox_api::nodes::node::{lxc, qemu};

use super::{CloneOptions, Guest, GuestId, GuestKind, VmId, clone, list};
use crate::client::Client;
use crate::tasks;

/// Every template in the cluster, ordered by ID.
pub async fn templates(client: &Client) -> Result<Vec<Guest>, ReqwestError> {
    let mut templates: Vec<Guest> = list(client)
        .await?
        .into_iter()
        .filter(|guest| guest.template)
        .collect();
    templates.sort_by_key(|guest| guest.id.vmid);
    Ok(templates)
}

#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    #[error(transparent)]
    Api(#[from] ReqwestError),
    #[error("Proxmox returned an ID out of range: {0}")]
    BadId(i64),
    #[error("copying the template failed: {0}")]
    Copy(tasks::Error),
}

/// A guest [`create_from_template`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub guest: GuestId,
    /// `false` when the copy was still running at the timeout. The guest
    /// exists, but is locked until Proxmox is done, and its tags were not
    /// set: call [`set_tags`] once it is.
    pub finished: bool,
}

/// Makes a new guest from `template`: a full copy with the next free ID,
/// named `name` and carrying exactly `tags`. Waits up to `timeout` for the
/// copy.
pub async fn create_from_template(
    client: &Client,
    template: &GuestId,
    name: &str,
    tags: &[String],
    timeout: Duration,
) -> Result<Created, CreateError> {
    let next = ClusterClient::new(client)
        .nextid()
        .get(NextIdParams::default())
        .await?;
    let vmid = VmId::try_from(i128::from(next)).map_err(|_| CreateError::BadId(next))?;
    let options = CloneOptions {
        name: Some(name.to_owned()),
        full: Some(true),
        ..Default::default()
    };
    let task = clone(client, template, vmid, options).await?;
    // The copy is made on the template's node.
    let guest = GuestId {
        node: template.node.clone(),
        vmid,
        kind: template.kind,
    };
    match tasks::wait(client, &task, timeout).await {
        Ok(()) => {}
        // Still copying: the config is locked, so tags can't be set yet.
        Err(tasks::Error::TimedOut(_)) => {
            return Ok(Created {
                guest,
                finished: false,
            });
        }
        Err(error) => return Err(CreateError::Copy(error)),
    }
    set_tags(client, &guest, tags).await?;
    Ok(Created {
        guest,
        finished: true,
    })
}

/// Replaces the guest's tags.
pub async fn set_tags(
    client: &Client,
    guest: &GuestId,
    tags: &[String],
) -> Result<(), ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let joined = Some(tags.join(";"));
    match guest.kind {
        GuestKind::Qemu => {
            let params = qemu::vmid::config::PutParams {
                tags: joined,
                ..Default::default()
            };
            node.qemu().vmid(guest.vmid).config().put(params).await
        }
        GuestKind::Lxc => {
            let params = lxc::vmid::config::PutParams {
                tags: joined,
                ..Default::default()
            };
            node.lxc().vmid(guest.vmid).config().put(params).await
        }
    }
}
