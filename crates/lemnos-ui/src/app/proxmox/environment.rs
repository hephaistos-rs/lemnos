// /proxmox/{environment}: what runs on one environment (its nodes, guests
// and storage), with buttons to power guests on and off.
//
//   GET  /proxmox/{environment}         the overview
//   POST /proxmox/{environment}/power   start, shut down, reboot or stop a guest

use std::time::Duration;

use lemnos_db::{ProxmoxEnvironment, ProxmoxEnvironmentId};
use lemnos_proxmox::{
    client::Client,
    guests::{self, Guest, GuestId, GuestKind, GuestStatus, VmId},
    nodes::{self, Node},
    storage::{self, Storage},
    tasks,
};
use serde::{Deserialize, Serialize};
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{RouterErrorExt, SeeOther, bad_request, see_other},
        href, module_param, page, path_param, query_params, route,
    },
    view::{View, component, view},
};

use super::{connect, database, require_user};
use crate::components::{heat, notice, problem};

module_param!(pub environment: u64, error = not_found);

/// How long a power button waits for Proxmox to finish before the page is
/// shown again. A clean shutdown can take longer; it then carries on in the
/// background.
const POWER_WAIT: Duration = Duration::from_secs(20);

async fn stored(cx: &Cx) -> Result<ProxmoxEnvironment> {
    let id = ProxmoxEnvironmentId(*path_param::<Environment>(cx)?);
    Ok(database(cx)
        .proxmox_environment(id)
        .await?
        .ok_or_not_found()?)
}

#[query_params(error = bad_request)]
struct Query {
    notice: Option<String>,
    error: Option<String>,
    // Which power button it was, and on which machine.
    action: Option<String>,
    machine: Option<String>,
}

/// What the overview says after a power button was used.
#[derive(Serialize)]
struct Outcome<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    notice: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    action: &'a str,
    machine: &'a str,
}

/// What happened to `machine` after a power button, in words. `code` is
/// "done", or anything else while it is still under way.
pub fn power_notice(code: &str, action: Option<&str>, machine: Option<&str>) -> String {
    let machine = machine.unwrap_or("The machine");
    if code != "done" {
        return format!(
            "{machine} is taking longer than usual. Reload in a moment to see the result."
        );
    }
    match action {
        Some("start") => format!("{machine} is running."),
        Some("resume") => format!("{machine} is running again."),
        Some("reboot") => format!("{machine} was rebooted."),
        Some("shutdown") => format!("{machine} was shut down."),
        Some("stop") => format!("{machine} was stopped."),
        _ => format!("{machine} was updated."),
    }
}

/// The same for a power button that failed. The reason itself goes below it.
pub fn power_problem(machine: Option<&str>) -> String {
    format!(
        "{} did not change. Try again, or check the environment it runs in.",
        machine.unwrap_or("The machine")
    )
}

/// "idle" for a guest that is not running, so its row steps back.
pub fn row_class(guest: &Guest) -> &'static str {
    if is_running(guest) { "" } else { "idle" }
}

/// What to call the guest when talking about it.
fn spoken_name(guest: &Guest) -> String {
    guest
        .name
        .clone()
        .unwrap_or_else(|| guest.id.vmid.to_string())
}

pub struct Overview {
    pub nodes: Vec<Node>,
    pub guests: Vec<Guest>,
    pub storages: Vec<Storage>,
}

pub async fn overview(client: &Client) -> std::result::Result<Overview, lemnos_proxmox::client::Error> {
    let (nodes, guests, storages) = tokio::join!(
        nodes::list(client),
        guests::list(client),
        storage::list(client)
    );
    let mut guests = guests?;
    guests.sort_by_key(|guest| guest.id.vmid);
    Ok(Overview {
        nodes: nodes?,
        guests,
        storages: storages?,
    })
}

pub fn percent(fraction: Option<f64>) -> String {
    match fraction {
        Some(fraction) => format!("{:.0}%", fraction * 100.0),
        None => "-".to_owned(),
    }
}

/// `used` of `total` bytes, in GiB.
pub fn usage(used: Option<u64>, total: Option<u64>) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    match (used, total) {
        (Some(used), Some(total)) => {
            format!("{:.1} / {:.1} GiB", used as f64 / GIB, total as f64 / GIB)
        }
        (None, Some(total)) => format!("{:.1} GiB", total as f64 / GIB),
        _ => "-".to_owned(),
    }
}

/// How full something is, from 0 to 1, for a heat strip.
pub fn fraction(used: Option<u64>, total: Option<u64>) -> Option<f64> {
    match (used, total) {
        (Some(used), Some(total)) if total > 0 => Some(used as f64 / total as f64),
        _ => None,
    }
}

fn kind_name(kind: GuestKind) -> &'static str {
    match kind {
        GuestKind::Qemu => "qemu",
        GuestKind::Lxc => "lxc",
    }
}

pub fn kind_label(kind: GuestKind) -> &'static str {
    match kind {
        GuestKind::Qemu => "VM",
        GuestKind::Lxc => "Container",
    }
}

/// The guest's state as (class for the mark, word beside it).
pub fn state_label(guest: &Guest) -> (&'static str, String) {
    if guest.template {
        return ("state state-template", "Template".to_owned());
    }
    match &guest.status {
        GuestStatus::Running => ("state state-running", "Running".to_owned()),
        GuestStatus::Stopped => ("state state-stopped", "Stopped".to_owned()),
        GuestStatus::Paused => ("state state-paused", "Paused".to_owned()),
        GuestStatus::Other(other) => ("state state-template", other.clone()),
    }
}

/// Only a running guest uses any CPU or memory worth drawing.
pub fn is_running(guest: &Guest) -> bool {
    !guest.template && matches!(guest.status, GuestStatus::Running)
}

/// The power buttons that make sense for the guest right now, as
/// (action, label, button class).
fn power_actions(guest: &Guest) -> &'static [(&'static str, &'static str, &'static str)] {
    if guest.template {
        return &[];
    }
    match guest.status {
        GuestStatus::Running => &[
            ("shutdown", "Shut down", "button button-sm"),
            ("reboot", "Reboot", "button button-sm"),
            ("stop", "Stop", "button button-sm button-danger"),
        ],
        GuestStatus::Stopped => &[("start", "Start", "button button-sm button-primary")],
        GuestStatus::Paused => &[
            ("resume", "Resume", "button button-sm button-primary"),
            ("stop", "Stop", "button button-sm button-danger"),
        ],
        GuestStatus::Other(_) => &[("stop", "Stop", "button button-sm button-danger")],
    }
}

// The power buttons for one guest. `back` says which page to return to:
// "machines" for the list of every machine, anything else for the
// environment's own page.
#[component]
pub async fn power_buttons(environment: u64, guest: &Guest, back: &str) -> Result<impl View> {
    Ok(view! {
        <div class="actions">
            for (action, label, class) in power_actions(guest) {
                // A form each, not links: these change state, so they must be POSTs.
                <form method="post" action=(href!(power, Environment(environment)))>
                    <input type="hidden" name="node" value=(guest.id.node.as_str())>
                    <input type="hidden" name="kind" value=(kind_name(guest.id.kind))>
                    <input type="hidden" name="vmid" value=(guest.id.vmid.to_string())>
                    <input type="hidden" name="back" value=(back)>
                    <input type="hidden" name="machine" value=(spoken_name(guest))>
                    <input type="hidden" name="action" value=(*action)>
                    <button type="submit" class=(*class)>(*label)</button>
                </form>
            }
        </div>
    })
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    require_user(cx).await?;
    let query = query_params::<Query>(cx)?;
    let environment = stored(cx).await?;
    let id = environment.id.0;

    // A Proxmox that can't be reached is shown as a message, not as a
    // broken page: it is the first thing to check on a new environment.
    let loaded = match connect(&environment).await {
        Ok(client) => overview(&client).await,
        Err(error) => Err(error),
    };
    let (overview, failure) = match loaded {
        Ok(overview) => (Some(overview), None),
        Err(error) => {
            eprintln!(
                "Proxmox environment `{}` failed: {error:?}",
                environment.name
            );
            (None, Some(error.to_string()))
        }
    };
    let power_done = query
        .notice
        .as_deref()
        .map(|code| power_notice(code, query.action.as_deref(), query.machine.as_deref()));
    let power_failed = power_problem(query.machine.as_deref());
    let unreachable = format!(
        "Lemnos can't reach {}. Check its address, its login and whether its certificate is self-signed.",
        environment.name
    );

    Ok(view! {
        <div class="page">
            <div class="flex flex-col gap-y-2">
                <a href=(href!(super::page)) class="back-link self-start">"All environments"</a>
                <div class="page-head">
                    <div>
                        <h1>(environment.name.as_str())</h1>
                        <p class="figure">(environment.url.as_str())</p>
                    </div>
                </div>
            </div>

            if let Some(message) = &power_done {
                notice(tone: "success", message: message.as_str())
            }
            if let Some(reason) = &query.error {
                problem(message: power_failed.as_str(), detail: reason.as_str(), link: ("", ""))
            }

            if let Some(reason) = &failure {
                <section class="flex flex-col items-start gap-y-4">
                    problem(message: unreachable.as_str(), detail: reason.as_str(), link: ("", ""))
                    <a href=(href!(page, Environment(id))) class="button">"Try again"</a>
                </section>
            }

            if let Some(overview) = &overview {
                <section class="flex flex-col gap-y-3">
                    <h2 class="section-title">"Machines"</h2>
                    <div class="data data-stack data-machines">
                        <table>
                            <thead>
                                <tr>
                                    <th>"State"</th>
                                    <th>"ID"</th>
                                    <th>"Name"</th>
                                    <th>"Kind"</th>
                                    <th>"Node"</th>
                                    <th>"CPU"</th>
                                    <th>"Memory"</th>
                                    <th><span class="sr-only">"Power"</span></th>
                                </tr>
                            </thead>
                            <tbody>
                                if overview.guests.is_empty() {
                                    <tr>
                                        <td colspan="8" class="empty wide">
                                            "This environment has no VMs or containers yet."
                                        </td>
                                    </tr>
                                }
                                for guest in &overview.guests {
                                    <tr class=(row_class(guest))>
                                        <td data-label="State">
                                            <span class=(state_label(guest).0)>
                                                (state_label(guest).1)
                                            </span>
                                        </td>
                                        <td data-label="ID" class="figure meta">
                                            (guest.id.vmid.to_string())
                                        </td>
                                        <td data-label="Name" class="name">
                                            (guest.name.as_deref().unwrap_or("-"))
                                        </td>
                                        <td data-label="Kind" class="meta">(kind_label(guest.id.kind))</td>
                                        <td data-label="Node" class="meta">(guest.id.node.as_str())</td>
                                        <td data-label="CPU" class=(if is_running(guest) { "" } else { "blank" })>
                                            if is_running(guest) {
                                                heat(fraction: guest.cpu, figure: percent(guest.cpu))
                                            }
                                        </td>
                                        <td data-label="Memory">
                                            if is_running(guest) {
                                                heat(
                                                    fraction: fraction(guest.mem, guest.max_mem),
                                                    figure: usage(guest.mem, guest.max_mem),
                                                )
                                            } else {
                                                heat(fraction: None, figure: usage(None, guest.max_mem))
                                            }
                                        </td>
                                        <td class="wide">
                                            power_buttons(environment: id, guest: guest, back: "environment")
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </div>
                </section>

                <section class="flex flex-col gap-y-3">
                    <h2 class="section-title">"Nodes"</h2>
                    <div class="data data-stack">
                        <table>
                            <thead>
                                <tr>
                                    <th>"State"</th>
                                    <th>"Name"</th>
                                    <th>"CPU"</th>
                                    <th>"Memory"</th>
                                </tr>
                            </thead>
                            <tbody>
                                for node in &overview.nodes {
                                    <tr>
                                        <td data-label="State">
                                            if node.online {
                                                <span class="state state-running">"Online"</span>
                                            } else {
                                                <span class="state state-down">"Offline"</span>
                                            }
                                        </td>
                                        <td data-label="Name" class="name">(node.name.as_str())</td>
                                        <td data-label="CPU">
                                            heat(fraction: node.cpu, figure: percent(node.cpu))
                                        </td>
                                        <td data-label="Memory">
                                            heat(
                                                fraction: fraction(node.mem, node.max_mem),
                                                figure: usage(node.mem, node.max_mem),
                                            )
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </div>
                </section>

                <section class="flex flex-col gap-y-3">
                    <h2 class="section-title">"Storage"</h2>
                    <div class="data data-stack">
                        <table>
                            <thead>
                                <tr>
                                    <th>"Name"</th>
                                    <th>"Node"</th>
                                    <th>"Type"</th>
                                    <th>"Holds"</th>
                                    <th>"Used"</th>
                                </tr>
                            </thead>
                            <tbody>
                                for storage in &overview.storages {
                                    <tr>
                                        <td data-label="Name" class="name">(storage.name.as_str())</td>
                                        <td data-label="Node">(storage.node.as_str())</td>
                                        <td data-label="Type">
                                            (storage.kind.as_deref().unwrap_or("-"))
                                        </td>
                                        <td data-label="Holds">(storage.content.join(", "))</td>
                                        <td data-label="Used">
                                            if storage.available {
                                                heat(
                                                    fraction: fraction(storage.used, storage.total),
                                                    figure: usage(storage.used, storage.total),
                                                )
                                            } else {
                                                <span class="state state-down">"Unavailable"</span>
                                            }
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </div>
                </section>
            }
        </div>
    })
}

#[derive(Deserialize)]
pub struct PowerForm {
    node: String,
    kind: String,
    vmid: u64,
    action: String,
    // "machines" when the button was on the list of every machine.
    back: Option<String>,
    // The guest's name, only to say afterwards what happened to it.
    machine: Option<String>,
}

#[route(POST "./power")]
pub async fn power(cx: &Cx, Form(input): Form<PowerForm>) -> Result<SeeOther> {
    require_user(cx).await?;
    let environment = stored(cx).await?;
    let guest = GuestId {
        node: input.node,
        vmid: VmId::try_from(i128::from(input.vmid))
            .map_err(|_| bad_request("invalid guest ID"))?,
        kind: match input.kind.as_str() {
            "qemu" => GuestKind::Qemu,
            "lxc" => GuestKind::Lxc,
            _ => return Err(bad_request("unknown guest kind").into()),
        },
    };

    let outcome = async {
        let client = connect(&environment)
            .await
            .map_err(|error| error.to_string())?;
        let started = match input.action.as_str() {
            "start" => guests::start(&client, &guest).await,
            "shutdown" => guests::shutdown(&client, &guest).await,
            "reboot" => guests::reboot(&client, &guest).await,
            "stop" => guests::stop(&client, &guest).await,
            "resume" => guests::resume(&client, &guest).await,
            _ => return Ok(None),
        };
        let task = started.map_err(|error| error.to_string())?;
        match tasks::wait(&client, &task, POWER_WAIT).await {
            Ok(()) => Ok(Some("done")),
            Err(tasks::Error::TimedOut(_)) => Ok(Some("started")),
            Err(error) => Err(error.to_string()),
        }
    };
    let outcome: std::result::Result<Option<&str>, String> = outcome.await;

    let machine = input.machine.as_deref().unwrap_or_default();
    let outcome = match &outcome {
        Ok(Some(code)) => Outcome {
            notice: Some(code),
            error: None,
            action: &input.action,
            machine,
        },
        Ok(None) => return Err(bad_request("unknown power action").into()),
        Err(reason) => Outcome {
            notice: None,
            error: Some(reason),
            action: &input.action,
            machine,
        },
    };
    // Back to the page the button was on.
    Ok(see_other(if input.back.as_deref() == Some("machines") {
        href!(crate::app::home).query(outcome).resolve(cx)
    } else {
        href!(page, Environment(environment.id.0))
            .query(outcome)
            .resolve(cx)
    }))
}
