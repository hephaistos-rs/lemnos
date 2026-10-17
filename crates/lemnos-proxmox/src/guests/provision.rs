//! Making new guests for people: which templates there are to start from,
//! and turning one into a guest of its own.
//!
//! This is the whole of what Lemnos needs from a provider to hand out
//! machines: a list of starting points, and "make me one of those, called
//! this, marked with these tags". Everything else (who may, and how many) is
//! decided above this crate.

use proxmox_api::ReqwestError;
use proxmox_api::cluster::ClusterClient;
use proxmox_api::cluster::nextid::GetParams as NextIdParams;
use proxmox_api::nodes::NodesClient;
use proxmox_api::nodes::node::{lxc, qemu};
use proxmox_api::types::bounded_string::BoundedString;

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
}

/// The ID Proxmox would give a new guest now. Another request can be handed
/// the same one before either copy starts; see [`id_taken`].
pub async fn next_free_id(client: &Client) -> Result<VmId, CreateError> {
    let next = ClusterClient::new(client)
        .nextid()
        .get(NextIdParams::default())
        .await?;
    VmId::try_from(i128::from(next)).map_err(|_| CreateError::BadId(next))
}

/// Starts a full copy of `template` to `vmid`, named `name`. Returns at
/// once with the task doing it, on the template's node; the copy carries
/// the template's tags until [`tag_new_copy`] gives it its own.
pub async fn start_copy(
    client: &Client,
    template: &GuestId,
    vmid: VmId,
    name: &str,
) -> Result<tasks::Task, ReqwestError> {
    let options = CloneOptions {
        name: Some(name.to_owned()),
        full: Some(true),
        ..Default::default()
    };
    clone(client, template, vmid, options).await
}

/// Whether a copy was refused because a guest has its ID already: what
/// happens when two requests were given the same [`next_free_id`].
pub fn id_taken(error: &ReqwestError) -> bool {
    match error {
        ReqwestError::UnknownFailure(_, Some(message)) => message.contains("already exists"),
        _ => false,
    }
}

/// Whether `guest` is what `task` made, as far as Proxmox can tell: it is
/// called `name`, and (for a VM) its config was created no earlier than the
/// task started. Proxmox doesn't record which task made a guest, so this is
/// what there is to go on; containers have no creation time to compare.
pub async fn made_by(
    client: &Client,
    guest: &GuestId,
    name: &str,
    task_started: i64,
) -> Result<bool, ReqwestError> {
    let node = NodesClient::new(client).node(&guest.node);
    Ok(match guest.kind {
        GuestKind::Qemu => {
            let config = node
                .qemu()
                .vmid(guest.vmid)
                .config()
                .get(Default::default())
                .await?;
            config.name.as_deref() == Some(name)
                && created_at(config.meta.as_deref()).is_some_and(|ctime| ctime >= task_started)
        }
        GuestKind::Lxc => {
            let config = node
                .lxc()
                .vmid(guest.vmid)
                .config()
                .get(Default::default())
                .await?;
            config
                .hostname
                .is_some_and(|hostname| hostname.get_value() == name)
        }
    })
}

/// The `ctime` in a VM's `meta` setting, e.g.
/// `creation-qemu=9.0.2,ctime=1729000000`.
fn created_at(meta: Option<&str>) -> Option<i64> {
    meta?
        .split(',')
        .find_map(|part| part.strip_prefix("ctime="))?
        .parse()
        .ok()
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
    fn a_taken_id_is_recognised() {
        use proxmox_api::ReqwestError::UnknownFailure;
        use reqwest::StatusCode;

        let taken = UnknownFailure(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some(
                "VM 105 already exists on node 'git'
"
                .into(),
            ),
        );
        assert!(id_taken(&taken));
        let other = UnknownFailure(StatusCode::INTERNAL_SERVER_ERROR, Some("disk full".into()));
        assert!(!id_taken(&other));
        assert!(!id_taken(&UnknownFailure(StatusCode::FORBIDDEN, None)));
    }

    #[test]
    fn creation_time_comes_from_meta() {
        assert_eq!(
            created_at(Some("creation-qemu=9.0.2,ctime=1729000000")),
            Some(1729000000)
        );
        assert_eq!(created_at(Some("creation-qemu=9.0.2")), None);
        assert_eq!(created_at(None), None);
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
