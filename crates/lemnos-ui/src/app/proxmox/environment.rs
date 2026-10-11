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
    view::{View, view},
};

use super::{connect, database, require_user};

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
}

/// What the overview says after a power button was used.
#[derive(Serialize)]
struct Outcome<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    notice: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

fn notice_message(code: &str) -> &'static str {
    match code {
        "done" => "Done.",
        _ => "Proxmox is still working on it. Reload in a moment to see the result.",
    }
}

struct Overview {
    nodes: Vec<Node>,
    guests: Vec<Guest>,
    storages: Vec<Storage>,
}

async fn overview(client: &Client) -> std::result::Result<Overview, lemnos_proxmox::client::Error> {
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

fn percent(fraction: Option<f64>) -> String {
    match fraction {
        Some(fraction) => format!("{:.0}%", fraction * 100.0),
        None => "-".to_owned(),
    }
}

/// `used` of `total` bytes, in GiB.
fn usage(used: Option<u64>, total: Option<u64>) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    match (used, total) {
        (Some(used), Some(total)) => {
            format!("{:.1} / {:.1} GiB", used as f64 / GIB, total as f64 / GIB)
        }
        (None, Some(total)) => format!("{:.1} GiB", total as f64 / GIB),
        _ => "-".to_owned(),
    }
}

fn kind_name(kind: GuestKind) -> &'static str {
    match kind {
        GuestKind::Qemu => "qemu",
        GuestKind::Lxc => "lxc",
    }
}

fn kind_label(kind: GuestKind) -> &'static str {
    match kind {
        GuestKind::Qemu => "VM",
        GuestKind::Lxc => "Container",
    }
}

fn status_label(guest: &Guest) -> (&'static str, String) {
    if guest.template {
        return ("badge badge-soft", "Template".to_owned());
    }
    match &guest.status {
        GuestStatus::Running => ("badge badge-success", "Running".to_owned()),
        GuestStatus::Stopped => ("badge badge-soft", "Stopped".to_owned()),
        GuestStatus::Paused => ("badge badge-warning", "Paused".to_owned()),
        GuestStatus::Other(other) => ("badge badge-soft", other.clone()),
    }
}

/// The power buttons that make sense for the guest right now, as
/// (action, label, button class).
fn power_actions(guest: &Guest) -> &'static [(&'static str, &'static str, &'static str)] {
    if guest.template {
        return &[];
    }
    match guest.status {
        GuestStatus::Running => &[
            ("shutdown", "Shut down", "button"),
            ("reboot", "Reboot", "button"),
            ("stop", "Stop", "button button-danger"),
        ],
        GuestStatus::Stopped => &[("start", "Start", "button button-primary")],
        GuestStatus::Paused | GuestStatus::Other(_) => &[("stop", "Stop", "button button-danger")],
    }
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

    Ok(view! {
        <div class="mt-12 flex flex-col gap-y-8">
            <div>
                <a
                    href=(href!(super::page))
                    class="text-sm text-secondary-300 hover:underline"
                >
                    "All environments"
                </a>
                <h2 class="text-2xl font-bold text-secondary-50">
                    (environment.name.as_str())
                </h2>
                <p class="text-secondary-300">(environment.url.as_str())</p>
                if let Some(code) = &query.notice {
                    <p class="mt-2 text-sm text-success-200">(notice_message(code))</p>
                }
                if let Some(reason) = &query.error {
                    <p class="mt-2 text-sm text-danger-200">
                        "That did not work: "
                        (reason.as_str())
                    </p>
                }
            </div>

            if let Some(reason) = &failure {
                <section class="p-6 rounded-lg bg-secondary-700 flex flex-col gap-y-2">
                    <h3 class="text-lg font-bold text-secondary-50">
                        "Lemnos can't reach this environment"
                    </h3>
                    <p class="text-danger-200">(reason.as_str())</p>
                    <p class="text-secondary-300">
                        "Check the address, the login and whether the certificate is self-signed."
                    </p>
                </section>
            }

            if let Some(overview) = &overview {
                <div class="table">
                    <div class="table-header">
                        <h3 class="text-lg font-bold text-secondary-50">"Nodes"</h3>
                    </div>
                    <table>
                        <thead>
                            <tr>
                                <th>"Name"</th>
                                <th>"Status"</th>
                                <th>"CPU"</th>
                                <th>"Memory"</th>
                            </tr>
                        </thead>
                        <tbody>
                            for node in &overview.nodes {
                                <tr>
                                    <td class="font-medium text-secondary-50">
                                        (node.name.as_str())
                                    </td>
                                    <td>
                                        if node.online {
                                            <span class="badge badge-success">"Online"</span>
                                        } else {
                                            <span class="badge badge-danger">"Offline"</span>
                                        }
                                    </td>
                                    <td>(percent(node.cpu))</td>
                                    <td>(usage(node.mem, node.max_mem))</td>
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>

                <div class="table">
                    <div class="table-header">
                        <h3 class="text-lg font-bold text-secondary-50">"Guests"</h3>
                    </div>
                    <table>
                        <thead>
                            <tr>
                                <th>"ID"</th>
                                <th>"Name"</th>
                                <th>"Kind"</th>
                                <th>"Node"</th>
                                <th>"Status"</th>
                                <th>"CPU"</th>
                                <th>"Memory"</th>
                                <th></th>
                            </tr>
                        </thead>
                        <tbody>
                            if overview.guests.is_empty() {
                                <tr>
                                    <td colspan="8" class="text-secondary-300">
                                        "No VMs or containers."
                                    </td>
                                </tr>
                            }
                            for guest in &overview.guests {
                                <tr>
                                    <td>(guest.id.vmid.to_string())</td>
                                    <td class="font-medium text-secondary-50">
                                        (guest.name.as_deref().unwrap_or("-"))
                                    </td>
                                    <td>(kind_label(guest.id.kind))</td>
                                    <td>(guest.id.node.as_str())</td>
                                    <td>
                                        <span class=(status_label(guest).0)>
                                            (status_label(guest).1)
                                        </span>
                                    </td>
                                    <td>(percent(guest.cpu))</td>
                                    <td>(usage(guest.mem, guest.max_mem))</td>
                                    <td>
                                        <div class="flex justify-end gap-x-2">
                                            for (action, label, class) in power_actions(guest) {
                                                // A form each, not links: these change state, so they must be POSTs.
                                                <form method="post" action=(href!(power, Environment(id)))>
                                                    <input
                                                        type="hidden"
                                                        name="node"
                                                        value=(guest.id.node.as_str())
                                                    >
                                                    <input
                                                        type="hidden"
                                                        name="kind"
                                                        value=(kind_name(guest.id.kind))
                                                    >
                                                    <input
                                                        type="hidden"
                                                        name="vmid"
                                                        value=(guest.id.vmid.to_string())
                                                    >
                                                    <input type="hidden" name="action" value=(*action)>
                                                    <button type="submit" class=(*class)>(*label)</button>
                                                </form>
                                            }
                                        </div>
                                    </td>
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>

                <div class="table">
                    <div class="table-header">
                        <h3 class="text-lg font-bold text-secondary-50">"Storage"</h3>
                    </div>
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
                                    <td class="font-medium text-secondary-50">
                                        (storage.name.as_str())
                                    </td>
                                    <td>(storage.node.as_str())</td>
                                    <td>(storage.kind.as_deref().unwrap_or("-"))</td>
                                    <td>(storage.content.join(", "))</td>
                                    <td>
                                        if storage.available {
                                            (usage(storage.used, storage.total))
                                        } else {
                                            <span class="badge badge-danger">"Unavailable"</span>
                                        }
                                    </td>
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>
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

    let back = href!(page, Environment(environment.id.0));
    Ok(see_other(match &outcome {
        Ok(Some(notice)) => back
            .query(Outcome {
                notice: Some(notice),
                error: None,
            })
            .resolve(cx),
        Ok(None) => return Err(bad_request("unknown power action").into()),
        Err(reason) => back
            .query(Outcome {
                notice: None,
                error: Some(reason),
            })
            .resolve(cx),
    }))
}
