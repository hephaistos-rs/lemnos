// Machine records an admin has to look at, on the environment's page.
//
//   POST /environments/{environment}/records/confirm   this record is that guest:
//                                                      tag it with a fresh marker
//   POST /environments/{environment}/records/release   forget the record
//
// A record applies to a guest only while the guest carries the record's
// marker (see `access::applies_to`). Those that don't are listed here:
// records from before markers ("Unverified"), and records whose guest lost
// its marker in Proxmox ("Marker missing"). Until an admin confirms or
// releases one, it gives nobody anything.

use lemnos_db::MachineRecord;
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
    if guests.iter().any(|guest| applies_to(record, guest)) {
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
    vmid: String,
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

#[route(POST "./records/confirm")]
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

#[route(POST "./records/release")]
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
        MachineRecord {
            environment: ProxmoxEnvironmentId(1),
            machine: "100".into(),
            name: "scratch".into(),
            owner: Owner::Person(UserId(7)),
            created_by: None,
            marker: marker.map(str::to_owned),
        }
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
    fn a_record_whose_guest_is_gone_is_not_listed() {
        assert_eq!(standing(&record(None), &[guest(101, &[])]), None);
        assert_eq!(standing(&record(Some(MARKER)), &[]), None);
    }
}
