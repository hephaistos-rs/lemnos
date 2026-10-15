// /machines: every machine the signed-in user may see, across environments.

use lemnos_proxmox::guests::Guest;
use topcoat::{
    Result,
    context::Cx,
    router::{error::see_other, href, page, query_params},
    view::{View, view},
};

use super::{
    environments::{
        self,
        environment::{
            self, Environment, fraction, is_running, kind_label, percent, power_buttons,
            power_notice, power_problem, row_class, state_label, usage,
        },
    },
    organizations, welcome,
};
use crate::{
    access::{Scope, Visible, access, database},
    components::{heat, notice, problem},
};

#[query_params(error = bad_request)]
struct Query {
    notice: Option<String>,
    error: Option<String>,
    // Which power button it was, and on which machine.
    action: Option<String>,
    machine: Option<String>,
}

/// The environment's name and why it could not be read.
struct Unreachable {
    message: String,
    reason: String,
    // Its own page, and the words for the link to it; empty for people who
    // may not open it.
    page: String,
    open: String,
}

/// One environment and what could be read from it: the user's guests there,
/// or why not.
struct Loaded {
    visible: Visible,
    guests: std::result::Result<Vec<Guest>, String>,
}

async fn load(visible: Visible) -> Loaded {
    let guests = match environments::connect(&visible.environment).await {
        Ok(client) => environment::overview(&client)
            .await
            .map(|overview| overview.guests),
        Err(error) => Err(error),
    };
    let guests = guests
        .map(|mut guests| {
            guests.retain(|guest| visible.shows(guest));
            guests
        })
        .map_err(|error| {
            eprintln!(
                "Proxmox environment `{}` failed: {error:?}",
                visible.environment.name
            );
            error.to_string()
        });
    Loaded { visible, guests }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

// Someone with nothing to look at yet is shown what to do about it instead.
#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let access = access(cx).await?;
    let query = query_params::<Query>(cx)?;
    let database = database(cx);

    let visible = access.visible(database).await?;
    let nothing_connected =
        access.admin && visible.is_empty() && database.proxmox_environments().await?.is_empty();
    if matches!(access.scope, Scope::Nothing) || nothing_connected {
        return Err(see_other(href!(welcome::page).resolve(cx)).into());
    }

    let mut loaded = Vec::new();
    // One after the other: there are only ever a handful of environments.
    for entry in visible {
        loaded.push(load(entry).await);
    }
    let machines: usize = loaded
        .iter()
        .filter_map(|entry| entry.guests.as_ref().ok())
        .map(Vec::len)
        .sum();
    let running = loaded
        .iter()
        .filter_map(|entry| entry.guests.as_ref().ok())
        .flatten()
        .filter(|guest| is_running(guest))
        .count();
    let summary = format!(
        "{} in {}, {running} running.",
        plural(machines, "machine", "machines"),
        plural(loaded.len(), "environment", "environments"),
    );
    let power_done = query
        .notice
        .as_deref()
        .map(|code| power_notice(code, query.action.as_deref(), query.machine.as_deref()));
    let power_failed = power_problem(query.machine.as_deref());
    let unreachable: Vec<Unreachable> = loaded
        .iter()
        .filter_map(|entry| {
            let environment = &entry.visible.environment;
            Some(Unreachable {
                reason: entry.guests.as_ref().err()?.clone(),
                message: format!(
                    "Lemnos can't reach {}, so its machines are missing below.",
                    environment.name,
                ),
                page: if access.admin {
                    href!(environment::page, Environment(environment.id.0))
                        .resolve(cx)
                        .to_string()
                } else {
                    String::new()
                },
                open: format!("Check {}", environment.name),
            })
        })
        .collect();
    // An organisation that was given no environment has no list to show.
    let own_organization = match &access.scope {
        Scope::Organization(membership) => Some(membership.organization.id.0),
        Scope::Everything | Scope::Nothing => None,
    };

    Ok(view! {
        <div class="page">
            <div class="page-head">
                <div>
                    <h1>"Machines"</h1>
                    if !loaded.is_empty() {
                        <p>(summary.as_str())</p>
                    }
                </div>
            </div>

            if let Some(message) = &power_done {
                notice(tone: "success", message: message.as_str())
            }
            if let Some(reason) = &query.error {
                problem(message: power_failed.as_str(), detail: reason.as_str(), link: ("", ""))
            }
            for entry in &unreachable {
                problem(
                    message: entry.message.as_str(),
                    detail: entry.reason.as_str(),
                    link: (entry.page.as_str(), entry.open.as_str()),
                )
            }

            if loaded.is_empty() {
                <div class="panel items-start">
                    <h2 class="section-title">"No environments yet"</h2>
                    <p class="text-ink-muted max-w-[65ch]">
                        (access.scope_name())
                        " has not been given an environment, so there are no machines to show. An admin can give it one."
                    </p>
                    if let Some(organization) = own_organization {
                        <a
                            href=(href!(
                                organizations::organization::page,
                                organizations::organization::Organization(organization),
                            ))
                            class="button"
                        >
                            "Open the organisation"
                        </a>
                    }
                </div>
            } else {
                <div class="data data-stack data-machines">
                    <table>
                        <thead>
                            <tr>
                                <th>"State"</th>
                                <th>"ID"</th>
                                <th>"Name"</th>
                                <th>"Kind"</th>
                                <th>"Environment"</th>
                                <th>"CPU"</th>
                                <th>"Memory"</th>
                                <th><span class="sr-only">"Power"</span></th>
                            </tr>
                        </thead>
                        <tbody>
                            if machines == 0 {
                                <tr>
                                    <td colspan="8" class="empty wide">
                                        "No VMs or containers to show."
                                    </td>
                                </tr>
                            }
                            for entry in &loaded {
                                if let Ok(guests) = &entry.guests {
                                    for guest in guests {
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
                                            <td data-label="Environment" class="meta">
                                                // Only admins can open an environment.
                                                if access.admin {
                                                    <a
                                                        href=(href!(
                                                            environment::page,
                                                            Environment(entry.visible.environment.id.0),
                                                        ))
                                                        class="link"
                                                    >
                                                        (entry.visible.environment.name.as_str())
                                                    </a>
                                                } else {
                                                    (entry.visible.environment.name.as_str())
                                                }
                                            </td>
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
                                                // Viewers look; they get no buttons.
                                                if access.can_operate() {
                                                    power_buttons(
                                                        environment: entry.visible.environment.id.0,
                                                        guest: guest,
                                                        back: "machines",
                                                    )
                                                }
                                            </td>
                                        </tr>
                                    }
                                }
                            }
                        </tbody>
                    </table>
                </div>
            }
        </div>
    })
}
