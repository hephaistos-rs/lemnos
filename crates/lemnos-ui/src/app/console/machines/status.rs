// /machines/status: a machine Lemnos is creating, or failed to create.
//
//   GET  /machines/status?environment=...&record=...   where it is; a Refresh header reloads it while pending
//   POST /machines/status/retry                        forgets a failed one, back to the form
//   POST /machines/status/remove                       forgets a failed one
//
// Creating a machine only reserves it and starts the copy (see `creation`);
// this is the page that shows how it goes. Once the machine exists, it
// sends people on to the machines page.

use lemnos_db::{MachineRecord, MachineStatus, ProxmoxEnvironmentId};
use serde::{Deserialize, Serialize};
use topcoat::{
    Result,
    context::Cx,
    router::{
        HeaderName, HeaderValue,
        content::Form,
        error::{RouterErrorExt, SeeOther, forbidden, see_other},
        href, page, query_params,
        response::response_headers,
        route,
    },
    view::{View, view},
};

use crate::{
    access::{Access, access, database, owner_value},
    components::{notice, problem},
    creation,
};

/// How often a pending machine's page reloads itself, in seconds: the
/// value of its `Refresh` header.
const REFRESH_EVERY: &str = "10";

/// The record, as links to this page name it.
#[derive(Serialize)]
struct Target<'a> {
    environment: u64,
    record: &'a str,
}

/// The address of this page for one record.
pub fn link(cx: &Cx, environment: u64, record: &str) -> String {
    href!(page)
        .query(Target {
            environment,
            record,
        })
        .resolve(cx)
}

/// What the machines page says afterwards; see `environment::power_notice`.
#[derive(Serialize)]
struct Done<'a> {
    notice: &'a str,
    action: &'a str,
    machine: &'a str,
}

#[derive(Serialize)]
struct ForOwner<'a> {
    owner: &'a str,
}

#[query_params(error = bad_request)]
struct Query {
    environment: u64,
    record: String,
}

/// The record named, if the user may see it. Anything else is forbidden,
/// so as not to tell which records exist. A pending record is renamed to
/// its VM ID once the machine exists, so the old name finds it by marker.
async fn visible_record(cx: &Cx, environment: u64, key: &str) -> Result<(Access, MachineRecord)> {
    let access = access(cx).await?;
    let database = database(cx);
    let environment = ProxmoxEnvironmentId(environment);
    let record = match database.machine_record(environment, key).await? {
        Some(record) => Some(record),
        None => match key.strip_prefix("pending-") {
            Some(marker) => database.machine_by_marker(environment, marker).await?,
            None => None,
        },
    };
    let record = record
        .filter(|record| access.can_see_record_of(record.owner))
        .ok_or_forbidden()?;
    Ok((access, record))
}

/// The machines page, saying the machine was created.
fn created(cx: &Cx, record: &MachineRecord) -> SeeOther {
    see_other(
        href!(super::page)
            .query(Done {
                notice: "done",
                action: "create",
                machine: &record.name,
            })
            .resolve(cx),
    )
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let query = query_params::<Query>(cx)?;
    let (access, mut record) = visible_record(cx, query.environment, &query.record).await?;
    let database = database(cx);

    // Past a minute, look at Proxmox here rather than only wait for the
    // background job: it may be gone, if Lemnos restarted.
    if record.status == MachineStatus::Pending && creation::age(&record) >= creation::LOOK_AFTER {
        creation::reconcile(database, &record).await;
        let marker = record.marker.clone().unwrap_or_default();
        if let Some(current) = database
            .machine_by_marker(record.environment, &marker)
            .await?
        {
            record = current;
        }
    }
    if record.status == MachineStatus::Active {
        return Err(created(cx, &record).into());
    }

    let environment = database
        .proxmox_environment(record.environment)
        .await?
        .map_or_else(
            || "its environment".to_owned(),
            |environment| environment.name,
        );
    let here = link(cx, record.environment.0, &record.machine);
    let may_manage = access.may_create_for(record.owner);
    let reason = record
        .failure
        .clone()
        .unwrap_or_else(|| "Creating the machine didn't work.".to_owned());
    let detail = record.failure_detail.clone().unwrap_or_default();
    let pending = record.status == MachineStatus::Pending;
    if pending {
        // The browser reloads the page by itself, with no script; the link
        // on it does the same by hand.
        response_headers(cx).append(
            HeaderName::from_static("refresh"),
            HeaderValue::from_static(REFRESH_EVERY),
        );
    }

    Ok(view! {
        <div class="page-narrow">
            <div class="flex flex-col gap-y-2">
                <a href=(href!(super::page)) class="back-link self-start">"All machines"</a>
                <div class="page-head">
                    <div>
                        <h1>(record.name.as_str())</h1>
                        <p>"In " (environment.as_str())</p>
                    </div>
                </div>
            </div>

            if pending {
                <section class="panel items-start">
                    <h2 class="section-title">"Being created"</h2>
                    <p class="text-ink-muted max-w-[65ch]">
                        "Lemnos is copying the template into a machine of its own. That usually takes a minute or two, longer for a large disk. This page shows the result when it is done; it reloads every few seconds, or use the button."
                    </p>
                    <a href=(here.as_str()) class="button">"Refresh"</a>
                </section>
            } else {
                <section class="flex flex-col items-start gap-y-4">
                    // The raw error is for admins; everyone else gets the words.
                    if access.admin {
                        problem(message: reason.as_str(), detail: detail.as_str(), link: ("", ""))
                    } else {
                        notice(tone: "danger", message: reason.as_str())
                    }
                    <p class="text-ink-muted max-w-[65ch]">
                        "The machine wasn't created, and doesn't count against anyone. Try again, or remove this entry. If it keeps failing, ask an admin to look at the environment."
                    </p>
                    if may_manage {
                        <div class="actions">
                            <form method="post" action=(href!(retry))>
                                <input type="hidden" name="environment" value=(record.environment.0.to_string())>
                                <input type="hidden" name="record" value=(record.machine.as_str())>
                                <button type="submit" class="button button-primary">"Try again"</button>
                            </form>
                            <form method="post" action=(href!(remove))>
                                <input type="hidden" name="environment" value=(record.environment.0.to_string())>
                                <input type="hidden" name="record" value=(record.machine.as_str())>
                                <button type="submit" class="button">"Remove this entry"</button>
                            </form>
                        </div>
                    }
                </section>
            }
        </div>
    })
}

#[derive(Deserialize)]
pub struct RecordForm {
    environment: u64,
    record: String,
}

/// A failed record the user may clear, after checking both.
async fn clearable(cx: &Cx, input: &RecordForm) -> Result<MachineRecord> {
    let (access, record) = visible_record(cx, input.environment, &input.record).await?;
    if record.status != MachineStatus::Failed || !access.may_create_for(record.owner) {
        return Err(forbidden().into());
    }
    database(cx)
        .forget_machine(record.environment, &record.machine)
        .await?;
    Ok(record)
}

// Clears the failed entry and goes back to the form for the same owner.
#[route(POST "./retry")]
pub async fn retry(cx: &Cx, Form(input): Form<RecordForm>) -> Result<SeeOther> {
    let record = clearable(cx, &input).await?;
    Ok(see_other(
        href!(super::new::page)
            .query(ForOwner {
                owner: &owner_value(record.owner),
            })
            .resolve(cx),
    ))
}

#[route(POST "./remove")]
pub async fn remove(cx: &Cx, Form(input): Form<RecordForm>) -> Result<SeeOther> {
    clearable(cx, &input).await?;
    Ok(see_other(href!(super::page).resolve(cx)))
}
