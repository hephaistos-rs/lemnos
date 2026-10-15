// /machines/remove: deleting a machine Lemnos created, disks and all.
//
//   GET  /machines/remove?environment=...&vmid=...   asks to be sure
//   POST /machines/remove                            deletes it
//
// Only machines with an owner on record can be deleted here, and only by
// that owner, the owners of its organisation (or the people of its project)
// and admins. Proxmox refuses while the machine is running.

use std::time::Duration;

use lemnos_proxmox::guests::{self, Guest, GuestStatus};
use serde::{Deserialize, Serialize};
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{RouterErrorExt, SeeOther, forbidden, see_other},
        href, page, query_params, route,
    },
    view::{View, view},
};

use super::super::environments::{self, environment::kind_label};
use crate::{
    access::{Visible, access, database},
    components::problem,
};

/// How long the button waits for Proxmox to delete the machine.
const REMOVE_WAIT: Duration = Duration::from_secs(30);

/// The machine, as the link to this page names it.
#[derive(Serialize)]
pub struct Target<'a> {
    environment: u64,
    vmid: &'a str,
}

/// The address of this page for one machine.
pub fn link(cx: &Cx, environment: u64, vmid: &str) -> String {
    href!(page).query(Target { environment, vmid }).resolve(cx)
}

#[query_params(error = bad_request)]
struct Query {
    environment: u64,
    vmid: String,
    error: Option<String>,
}

/// The machine named, if the user may delete it: the environment it is in,
/// and the machine as Proxmox lists it. Anything else is forbidden.
async fn deletable(cx: &Cx, environment: u64, vmid: &str) -> Result<(Visible, Guest)> {
    let access = access(cx).await?;
    let visible = access
        .visible(database(cx))
        .await?
        .into_iter()
        .find(|visible| visible.environment.id.0 == environment)
        .ok_or_forbidden()?;
    let client = environments::connect(&visible.environment)
        .await
        .map_err(|_| forbidden())?;
    let guest = guests::list(&client)
        .await
        .map_err(|_| forbidden())?
        .into_iter()
        .find(|guest| guest.id.vmid.to_string() == vmid && visible.shows(guest))
        .ok_or_forbidden()?;
    if !access.can_delete(&visible, &guest) {
        return Err(forbidden().into());
    }
    Ok((visible, guest))
}

fn spoken_name(guest: &Guest) -> String {
    guest
        .name
        .clone()
        .unwrap_or_else(|| guest.id.vmid.to_string())
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let query = query_params::<Query>(cx)?;
    let (visible, guest) = deletable(cx, query.environment, &query.vmid).await?;
    let name = spoken_name(&guest);
    let running = guest.status != GuestStatus::Stopped;

    Ok(view! {
        <div class="page-narrow">
            <div class="flex flex-col gap-y-2">
                <a href=(href!(super::page)) class="back-link self-start">"All machines"</a>
                <div class="page-head">
                    <div>
                        <h1>"Delete " (name.as_str()) "?"</h1>
                        <p>
                            (kind_label(guest.id.kind)) " " (guest.id.vmid.to_string())
                            " in " (visible.environment.name.as_str())
                        </p>
                    </div>
                </div>
            </div>

            if let Some(reason) = &query.error {
                problem(message: "The machine was not deleted.", detail: reason.as_str(), link: ("", ""))
            }

            <section class="panel">
                <p class="max-w-[65ch]">
                    "The machine and all of its disks and snapshots are removed for good. This can't be undone."
                </p>
                if running {
                    <p class="text-ink-muted max-w-[65ch]">"Shut it down first: a running machine can't be deleted."</p>
                    <a href=(href!(super::page)) class="button self-start">"Go to machines"</a>
                } else {
                    <form method="post" action=(href!(remove)) class="flex flex-wrap gap-2">
                        <input type="hidden" name="environment" value=(visible.environment.id.0.to_string())>
                        <input type="hidden" name="vmid" value=(guest.id.vmid.to_string())>
                        <button type="submit" class="button button-danger">"Delete for good"</button>
                        <a href=(href!(super::page)) class="button">"Keep it"</a>
                    </form>
                }
            </section>
        </div>
    })
}

#[derive(Deserialize)]
pub struct RemoveForm {
    environment: u64,
    vmid: String,
}

#[derive(Serialize)]
struct Failed<'a> {
    environment: u64,
    vmid: &'a str,
    error: &'a str,
}

/// What the machines page says afterwards; see `environment::power_notice`.
#[derive(Serialize)]
struct Done<'a> {
    notice: &'a str,
    action: &'a str,
    machine: &'a str,
}

#[route(POST)]
pub async fn remove(cx: &Cx, Form(input): Form<RemoveForm>) -> Result<SeeOther> {
    let (visible, guest) = deletable(cx, input.environment, &input.vmid).await?;
    let name = spoken_name(&guest);
    let outcome = async {
        let client = environments::connect(&visible.environment)
            .await
            .map_err(|error| error.to_string())?;
        let task = guests::destroy(&client, &guest.id, true)
            .await
            .map_err(|error| error.to_string())?;
        lemnos_proxmox::tasks::wait(&client, &task, REMOVE_WAIT)
            .await
            .or_else(|error| match error {
                // Proxmox is still at it; it will finish.
                lemnos_proxmox::tasks::Error::TimedOut(_) => Ok(()),
                other => Err(other.to_string()),
            })
    };
    let outcome: std::result::Result<(), String> = outcome.await;
    if let Err(reason) = outcome {
        return Ok(see_other(
            href!(page)
                .query(Failed {
                    environment: input.environment,
                    vmid: &input.vmid,
                    error: &reason,
                })
                .resolve(cx),
        ));
    }
    database(cx)
        .forget_machine(visible.environment.id, &input.vmid)
        .await?;
    Ok(see_other(
        href!(super::page)
            .query(Done {
                notice: "done",
                action: "delete",
                machine: &name,
            })
            .resolve(cx),
    ))
}
