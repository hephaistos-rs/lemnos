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
use lemnos_core::marker;

use crate::client::{Client, no_data};
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
    /// set: call [`tag_new_copy`] once it is.
    pub finished: bool,
}

/// Makes a new guest from `template`: a full copy with the next free ID,
/// named `name` and carrying `tags` (and none of the template's; see
/// [`tag_new_copy`]). Waits up to `timeout` for the copy.
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
    tag_new_copy(client, &guest, tags).await?;
    Ok(Created {
        guest,
        finished: true,
    })
}

/// The guest's tags, as its config holds them.
pub async fn tags(client: &Client, guest: &GuestId) -> Result<Vec<String>, ReqwestError> {
    Ok(read_tags(client, guest).await?.0)
}

/// Adds `tags` to the ones the guest carries. Nothing it carries is taken
/// off.
pub async fn add_tags(
    client: &Client,
    guest: &GuestId,
    tags: &[String],
) -> Result<(), ReqwestError> {
    write_tags(client, guest, |current| merged(current, tags)).await
}

/// Gives a guest Lemnos just copied its own tags. A copy starts out with
/// its template's, which aren't the new machine's: one could be another
/// organisation's, and put the machine in that organisation's view. So
/// those go, except markers, which Lemnos never takes off.
pub async fn tag_new_copy(
    client: &Client,
    guest: &GuestId,
    tags: &[String],
) -> Result<(), ReqwestError> {
    write_tags(client, guest, |current| replaced(current, tags)).await
}

/// `current` with `add` after it, each tag once.
fn merged(current: &[String], add: &[String]) -> Vec<String> {
    let mut tags = current.to_vec();
    for tag in add {
        if !tags.contains(tag) {
            tags.push(tag.clone());
        }
    }
    tags
}

/// `tags`, and whatever markers `current` has that `tags` doesn't.
fn replaced(current: &[String], tags: &[String]) -> Vec<String> {
    replaced_markers_kept(current, merged(&[], tags))
}

/// Splits a config's `tags` value. Proxmox separates them with `;`, and
/// also takes `,` and spaces.
fn split_tags(value: Option<&str>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|tags| tags.split([';', ',', ' ']))
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The guest's tags and its config's digest.
async fn read_tags(
    client: &Client,
    guest: &GuestId,
) -> Result<(Vec<String>, String), ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    let (tags, digest) = match guest.kind {
        GuestKind::Qemu => {
            let config = node
                .qemu()
                .vmid(guest.vmid)
                .config()
                .get(Default::default())
                .await?;
            (config.tags, config.digest)
        }
        GuestKind::Lxc => {
            let config = node
                .lxc()
                .vmid(guest.vmid)
                .config()
                .get(Default::default())
                .await?;
            (config.tags, config.digest)
        }
    };
    Ok((split_tags(tags.as_deref()), digest))
}

/// Reads the guest's tags, has `change` work out the new ones, and writes
/// them back. The write carries the digest of the config that was read, so
/// if anything changed it in between, Proxmox refuses instead of losing it.
/// Markers the guest had are kept whatever `change` returns.
async fn write_tags(
    client: &Client,
    guest: &GuestId,
    change: impl FnOnce(&[String]) -> Vec<String>,
) -> Result<(), ReqwestError> {
    let (current, digest) = read_tags(client, guest).await?;
    let next = replaced_markers_kept(&current, change(&current));
    if next == current {
        return Ok(());
    }
    // An empty list can't be written as such; the setting is removed instead.
    let (tags, delete) = if next.is_empty() {
        (None, Some("tags".to_owned()))
    } else {
        (Some(next.join(";")), None)
    };
    let bad_digest =
        || ReqwestError::Other("Proxmox returned a config digest that can't be sent back");
    let node = NodesClient::new(client).node(&guest.node);
    match guest.kind {
        GuestKind::Qemu => {
            let params = qemu::vmid::config::PutParams {
                tags,
                delete,
                digest: Some(digest.try_into().map_err(|_| bad_digest())?),
                ..Default::default()
            };
            no_data(node.qemu().vmid(guest.vmid).config().put(params)).await
        }
        GuestKind::Lxc => {
            let params = lxc::vmid::config::PutParams {
                tags,
                delete,
                digest: Some(digest.try_into().map_err(|_| bad_digest())?),
                ..Default::default()
            };
            no_data(node.lxc().vmid(guest.vmid).config().put(params)).await
        }
    }
}

/// `next`, with any marker from `current` it left out put back.
fn replaced_markers_kept(current: &[String], next: Vec<String>) -> Vec<String> {
    let markers: Vec<String> = current
        .iter()
        .filter(|tag| marker::is_marker(tag))
        .cloned()
        .collect();
    merged(&next, &markers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|tag| tag.to_string()).collect()
    }

    const MARKER: &str = "lemnos-3f9a0c7e21bd";

    #[test]
    fn adding_keeps_what_is_there() {
        assert_eq!(
            merged(&tags(&["k3s", MARKER]), &tags(&["team-a", "k3s"])),
            tags(&["k3s", MARKER, "team-a"])
        );
    }

    #[test]
    fn a_new_copy_drops_its_template_tags_but_not_markers() {
        assert_eq!(
            replaced(
                &tags(&["template", "team-b", MARKER]),
                &tags(&["team-a", "personal-jo"])
            ),
            tags(&["team-a", "personal-jo", MARKER])
        );
    }

    #[test]
    fn a_marker_is_never_taken_off() {
        // Whatever the change asks for.
        assert_eq!(
            replaced_markers_kept(&tags(&["k3s", MARKER]), Vec::new()),
            tags(&[MARKER])
        );
        assert_eq!(
            replaced_markers_kept(&tags(&["k3s"]), Vec::new()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn config_tags_split_like_proxmox_does() {
        assert_eq!(
            split_tags(Some("noble;template,ubuntu  x")),
            tags(&["noble", "template", "ubuntu", "x"])
        );
        assert!(split_tags(None).is_empty());
    }
}
