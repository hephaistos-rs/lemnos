// Machine records an admin has to look at, on the environment's page.
//
//   POST /environments/{environment}/records/confirm   this record is that guest:
//                                                      tag it with a fresh marker
//   POST /environments/{environment}/records/release   forget the record
//
//   POST /environments/{environment}/records/delete    delete what a failed
//                                                      creation left behind
//
// A record applies to a guest only while the guest carries the record's
// marker (see `access::applies_to`). Active records that don't are listed
// here: records from before markers ("Unverified"), and records whose guest
// lost its marker in Proxmox ("Marker missing"). Until an admin confirms or
// releases one, it gives nobody anything. Pending records are never listed:
// their copy simply isn't tagged yet.
//
// Failed creations that left a guest behind are listed too, so it can be
// deleted on purpose rather than lingering.

use lemnos_db::{MachineRecord, MachineStatus};
use lemnos_proxmox::guests::{self, Guest, provision};
use serde::{Deserialize, Serialize};
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{SeeOther, see_other},
        href, route,
    },
    view::{View, component, view},
};

use super::{Environment, connect, stored};
use crate::{
    access::{Access, admin_access, applies_to, database},
    app::console::machines::Owners,
};

/// Why a record needs looking at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// From before markers: there is a guest with its ID, but nothing says
    /// it is the one Lemnos created.
    Unverified,
    /// The guest with its ID doesn't carry its marker (any more). Taken off
    /// in Proxmox, the ID handed to another guest, or a copy whose tags
    /// weren't set yet.
    MarkerMissing,
}

impl Standing {
    fn label(self) -> &'static str {
        match self {
            Self::Unverified => "Unverified",
            Self::MarkerMissing => "Marker missing",
        }
    }

    fn explanation(self) -> &'static str {
        match self {
            Self::Unverified => {
                "Created before Lemnos marked its machines. Confirm it if this is still the machine it made."
            }
            Self::MarkerMissing => {
                "The machine with this ID lacks the tag Lemnos gave it. It may have been removed in Proxmox, or the ID now belongs to another machine."
            }
        }
    }
}

/// Where the record stands against the guests that are there: `None` when
/// it applies to one (all is well) or its guest is gone (it is forgotten the
/// next time quotas are counted).
pub fn standing(record: &MachineRecord, guests: &[Guest]) -> Option<Standing> {
    // A pending machine has no tags until its copy is done, and a failed
    // one never existed as far as its owner goes: only an active record can
    // be missing its marker.
    if record.status != MachineStatus::Active
        || guests.iter().any(|guest| applies_to(record, guest))
    {
        return None;
    }
    guests
        .iter()
        .any(|guest| guest.id.vmid.to_string() == record.machine)
        .then_some(if record.marker.is_none() {
            Standing::Unverified
        } else {
            Standing::MarkerMissing
        })
}

/// One row of the list: the record, why it is there, and the guest that has
/// its ID now.
pub struct ToCheck {
    pub record: MachineRecord,
    pub standing: Standing,
    pub guest: Guest,
}

/// A failed creation, and the guest it may have left behind.
pub struct Leftover {
    pub record: MachineRecord,
    pub guest: Guest,
    /// The guest carries the record's marker: it is certainly what the
    /// creation made. Otherwise it only has the ID the copy was going to.
    pub certain: bool,
}

/// What a failed record left behind among `guests`, if anything.
pub fn leftover_of<'a>(record: &MachineRecord, guests: &'a [Guest]) -> Option<(&'a Guest, bool)> {
    if record.status != MachineStatus::Failed {
        return None;
    }
    let marked = record
        .marker
        .as_ref()
        .and_then(|marker| guests.iter().find(|guest| guest.tags.contains(marker)));
    if let Some(guest) = marked {
        return Some((guest, true));
    }
    let vmid = record.vmid.as_deref()?;
    guests
        .iter()
        .find(|guest| guest.id.vmid.to_string() == vmid)
        .map(|guest| (guest, false))
}

/// The environment's failed creations that left something behind.
pub async fn leftovers(cx: &Cx, environment: u64, guests: &[Guest]) -> Result<Vec<Leftover>> {
    let records = database(cx)
        .environment_machines(lemnos_db::ProxmoxEnvironmentId(environment))
        .await?;
    Ok(records
        .into_iter()
        .filter_map(|record| {
            let (guest, certain) = leftover_of(&record, guests)?;
            Some(Leftover {
                guest: guest.clone(),
                certain,
                record,
            })
        })
        .collect())
}

#[component]
pub async fn leftovers_to_check(
    environment: u64,
    entries: &[Leftover],
    access: &Access,
    owners: &Owners,
) -> Result<impl View> {
    Ok(view! {
        <section class="flex flex-col gap-y-3">
            <h2 class="section-title">"Left behind by failed creations"</h2>
            <p class="text-ink-muted max-w-[65ch]">
                "Creating these machines failed, so they count against nobody, but Proxmox has a machine for them. Where it carries the creation's marker, it is certainly that machine and can be deleted here. Otherwise it only has the ID the copy was going to; check it in Proxmox before deleting it there."
            </p>
            <div class="data data-stack">
                <table>
                    <thead>
                        <tr>
                            <th>"ID"</th>
                            <th>"Was to be"</th>
                            <th>"Now called"</th>
                            <th>"Owner"</th>
                            <th>"Why it failed"</th>
                            <th><span class="sr-only">"Actions"</span></th>
                        </tr>
                    </thead>
                    <tbody>
                        for entry in entries {
                            <tr>
                                <td data-label="ID" class="figure meta">(entry.guest.id.vmid.to_string())</td>
                                <td data-label="Was to be" class="name">(entry.record.name.as_str())</td>
                                <td data-label="Now called">(entry.guest.name.as_deref().unwrap_or("-"))</td>
                                <td data-label="Owner">(owners.label(access, Some(&entry.record)))</td>
                                <td data-label="Why it failed">
                                    (entry.record.failure.as_deref().unwrap_or("-"))
                                    if let Some(detail) = &entry.record.failure_detail {
                                        <p class="hint figure">(detail.as_str())</p>
                                    }
                                </td>
                                <td class="wide">
                                    <div class="actions">
                                        if entry.certain {
                                            <form method="post" action=(href!(delete, Environment(environment)))>
                                                <input type="hidden" name="record" value=(entry.record.machine.as_str())>
                                                <button type="submit" class="button button-sm button-danger">"Delete the machine"</button>
                                            </form>
                                        } else {
                                            <span class="text-meta text-ink-faint">"Not certainly it"</span>
                                        }
                                        <form method="post" action=(href!(release, Environment(environment)))>
                                            <input type="hidden" name="vmid" value=(entry.record.machine.as_str())>
                                            <button type="submit" class="button button-sm">"Remove the entry"</button>
                                        </form>
                                    </div>
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            </div>
        </section>
    })
}

/// The environment's records that need an admin, given its guests.
pub async fn to_check(cx: &Cx, environment: u64, guests: &[Guest]) -> Result<Vec<ToCheck>> {
    let records = database(cx)
        .environment_machines(lemnos_db::ProxmoxEnvironmentId(environment))
        .await?;
    Ok(records
        .into_iter()
        .filter_map(|record| {
            let standing = standing(&record, guests)?;
            let guest = guests
                .iter()
                .find(|guest| guest.id.vmid.to_string() == record.machine)?
                .clone();
            Some(ToCheck {
                record,
                standing,
                guest,
            })
        })
        .collect())
}

#[component]
pub async fn records_to_check(
    environment: u64,
    entries: &[ToCheck],
    access: &Access,
    owners: &Owners,
) -> Result<impl View> {
    Ok(view! {
        <section class="flex flex-col gap-y-3">
            <h2 class="section-title">"Records to check"</h2>
            <p class="text-ink-muted max-w-[65ch]">
                "Lemnos can't tell whether these records still belong to the machine with their ID, so nobody sees them as theirs. Confirm a record to tag that machine and give it back to its owner, or release it to forget it. Neither changes the machine itself, apart from the tag."
            </p>
            <div class="data data-stack">
                <table>
                    <thead>
                        <tr>
                            <th>"State"</th>
                            <th>"ID"</th>
                            <th>"Recorded as"</th>
                            <th>"Now called"</th>
                            <th>"Owner"</th>
                            <th><span class="sr-only">"Actions"</span></th>
                        </tr>
                    </thead>
                    <tbody>
                        for entry in entries {
                            <tr>
                                <td data-label="State">
                                    <span class="state state-down">(entry.standing.label())</span>
                                    <p class="hint">(entry.standing.explanation())</p>
                                </td>
                                <td data-label="ID" class="figure meta">(entry.record.machine.as_str())</td>
                                <td data-label="Recorded as" class="name">(entry.record.name.as_str())</td>
                                <td data-label="Now called">(entry.guest.name.as_deref().unwrap_or("-"))</td>
                                <td data-label="Owner">(owners.label(access, Some(&entry.record)))</td>
                                <td class="wide">
                                    <div class="actions">
                                        <form method="post" action=(href!(confirm, Environment(environment)))>
                                            <input type="hidden" name="vmid" value=(entry.record.machine.as_str())>
                                            <input type="hidden" name="name" value=(entry.guest.name.as_deref().unwrap_or(""))>
                                            <button type="submit" class="button button-sm">"Confirm"</button>
                                        </form>
                                        <form method="post" action=(href!(release, Environment(environment)))>
                                            <input type="hidden" name="vmid" value=(entry.record.machine.as_str())>
                                            <button type="submit" class="button button-sm button-danger">"Release"</button>
                                        </form>
                                    </div>
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            </div>
        </section>
    })
}

#[derive(Deserialize)]
pub struct ConfirmForm {
    vmid: String,
    // The guest's name when the list was shown, so a guest that took the ID
    // in the meantime isn't confirmed by mistake.
    name: String,
}

#[derive(Deserialize)]
pub struct ReleaseForm {
    // The key the record goes by; for an active one, its VM ID.
    vmid: String,
}

#[derive(Deserialize)]
pub struct DeleteForm {
    record: String,
}

#[derive(Serialize)]
struct Outcome<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    notice: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

fn back(cx: &Cx, environment: u64, outcome: Outcome) -> SeeOther {
    see_other(
        href!(super::page, Environment(environment))
            .query(outcome)
            .resolve(cx),
    )
}

#[route(POST "./confirm")]
pub async fn confirm(cx: &Cx, Form(input): Form<ConfirmForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    let environment = stored(cx).await?;
    let id = environment.id.0;
    let failed = |reason: &str| {
        back(
            cx,
            id,
            Outcome {
                notice: None,
                error: Some(reason),
            },
        )
    };

    let client = match connect(&environment).await {
        Ok(client) => client,
        Err(error) => return Ok(failed(&error.to_string())),
    };
    let guests = match guests::list(&client).await {
        Ok(guests) => guests,
        Err(error) => return Ok(failed(&error.to_string())),
    };
    let Some(record) = database(cx)
        .environment_machines(environment.id)
        .await?
        .into_iter()
        .find(|record| record.machine == input.vmid)
    else {
        return Ok(failed("that record is gone already"));
    };
    if standing(&record, &guests).is_none() {
        return Ok(failed("that record needs nothing done"));
    }
    let Some(guest) = guests
        .iter()
        .find(|guest| guest.id.vmid.to_string() == input.vmid)
    else {
        return Ok(failed("there is no machine with that ID any more"));
    };
    if guest.name.as_deref().unwrap_or("") != input.name {
        return Ok(failed(
            "the machine with that ID was renamed or replaced since the list was shown; look again",
        ));
    }

    let marker = lemnos_core::marker::generate();
    if let Err(error) = provision::add_tags(&client, &guest.id, std::slice::from_ref(&marker)).await
    {
        return Ok(failed(&error.to_string()));
    }
    database(cx)
        .set_machine_marker(environment.id, &record.machine, &marker)
        .await?;
    Ok(back(
        cx,
        id,
        Outcome {
            notice: Some("confirmed"),
            error: None,
        },
    ))
}

#[route(POST "./release")]
pub async fn release(cx: &Cx, Form(input): Form<ReleaseForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    let environment = stored(cx).await?;
    database(cx)
        .forget_machine(environment.id, &input.vmid)
        .await?;
    Ok(back(
        cx,
        environment.id.0,
        Outcome {
            notice: Some("released"),
            error: None,
        },
    ))
}

/// How long deleting a leftover waits for Proxmox before showing the page.
const DELETE_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

// Deletes the guest a failed creation left behind, disks and all. Only one
// that carries the record's marker: anything less isn't certainly it.
#[route(POST "./delete")]
pub async fn delete(cx: &Cx, Form(input): Form<DeleteForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    let environment = stored(cx).await?;
    let id = environment.id.0;
    let failed = |reason: &str| {
        back(
            cx,
            id,
            Outcome {
                notice: None,
                error: Some(reason),
            },
        )
    };
    let Some(record) = database(cx)
        .machine_record(environment.id, &input.record)
        .await?
    else {
        return Ok(failed("that entry is gone already"));
    };
    let client = match connect(&environment).await {
        Ok(client) => client,
        Err(error) => return Ok(failed(&error.to_string())),
    };
    let guests = match guests::list(&client).await {
        Ok(guests) => guests,
        Err(error) => return Ok(failed(&error.to_string())),
    };
    let Some((guest, true)) = leftover_of(&record, &guests) else {
        return Ok(failed(
            "no machine carries this creation's marker, so Lemnos won't delete one for it",
        ));
    };
    let outcome = async {
        let task = guests::destroy(&client, &guest.id, true)
            .await
            .map_err(|error| error.to_string())?;
        match lemnos_proxmox::tasks::wait(&client, &task, DELETE_WAIT).await {
            Ok(()) | Err(lemnos_proxmox::tasks::Error::TimedOut(_)) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    };
    if let Err(reason) = outcome.await {
        return Ok(failed(&reason));
    }
    Ok(back(
        cx,
        id,
        Outcome {
            notice: Some("left-deleted"),
            error: None,
        },
    ))
}

#[cfg(test)]
mod tests {
    use lemnos_auth::UserId;
    use lemnos_db::{Owner, ProxmoxEnvironmentId};
    use lemnos_proxmox::guests::{GuestId, GuestKind, GuestStatus, VmId};

    use super::*;

    const MARKER: &str = "lemnos-3f9a0c7e21bd";

    fn guest(vmid: u32, tags: &[&str]) -> Guest {
        Guest {
            id: GuestId {
                node: "pve".into(),
                vmid: VmId::try_from(i128::from(vmid)).unwrap(),
                kind: GuestKind::Qemu,
            },
            name: Some("scratch".into()),
            status: GuestStatus::Stopped,
            template: false,
            cpu: None,
            max_cpu: None,
            mem: None,
            max_mem: None,
            uptime: None,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        }
    }

    fn record(marker: Option<&str>) -> MachineRecord {
        MachineRecord::active(
            ProxmoxEnvironmentId(1),
            "100",
            "scratch",
            Owner::Person(UserId(7)),
            None,
            marker.map(str::to_owned),
        )
    }

    #[test]
    fn a_matching_record_needs_nothing() {
        assert_eq!(
            standing(&record(Some(MARKER)), &[guest(100, &[MARKER])]),
            None
        );
    }

    #[test]
    fn an_old_record_is_unverified_and_a_lost_marker_missing() {
        assert_eq!(
            standing(&record(None), &[guest(100, &["personal-jo"])]),
            Some(Standing::Unverified)
        );
        assert_eq!(
            standing(&record(Some(MARKER)), &[guest(100, &["team-b"])]),
            Some(Standing::MarkerMissing)
        );
    }

    #[test]
    fn a_pending_or_failed_record_is_never_marker_missing() {
        // A copy still running: its guest has the template's tags only.
        for status in [MachineStatus::Pending, MachineStatus::Failed] {
            let record = MachineRecord {
                status,
                ..record(Some(MARKER))
            };
            assert_eq!(standing(&record, &[guest(100, &["template"])]), None);
        }
    }

    #[test]
    fn what_a_failed_creation_left_is_found_by_marker_first() {
        let failed = MachineRecord {
            status: MachineStatus::Failed,
            vmid: Some("105".into()),
            ..record(Some(MARKER))
        };
        // Carrying the marker: certainly it, wherever it is.
        let marked = [guest(107, &[MARKER]), guest(105, &[])];
        let (found, certain) = leftover_of(&failed, &marked).unwrap();
        assert_eq!((found.id.vmid.to_string().as_str(), certain), ("107", true));
        // Only the ID: maybe.
        let only_the_id = [guest(105, &["template"])];
        let (found, certain) = leftover_of(&failed, &only_the_id).unwrap();
        assert_eq!(
            (found.id.vmid.to_string().as_str(), certain),
            ("105", false)
        );
        assert!(leftover_of(&failed, &[guest(110, &[])]).is_none());
        // Only failed creations leave anything behind.
        let pending = MachineRecord {
            status: MachineStatus::Pending,
            ..failed.clone()
        };
        assert!(leftover_of(&pending, &marked).is_none());
    }

    #[test]
    fn a_record_whose_guest_is_gone_is_not_listed() {
        assert_eq!(standing(&record(None), &[guest(101, &[])]), None);
        assert_eq!(standing(&record(Some(MARKER)), &[]), None);
    }
}
