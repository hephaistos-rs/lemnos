// /machines: every machine the signed-in user may see, across environments.
// `new.rs` creates one; `remove.rs` deletes one Lemnos created.

use std::collections::HashMap;

use lemnos_auth::UserId;
use lemnos_db::{Database, MachineRecord, OrganizationId, Owner, ProjectId};
use lemnos_proxmox::guests::{Guest, GuestStatus};
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
    access::{Access, Scope, Visible, access, database},
    components::{heat, notice, problem},
};

pub mod new;
pub mod remove;

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

/// The names owners are shown by.
pub struct Owners {
    organizations: HashMap<OrganizationId, String>,
    projects: HashMap<ProjectId, String>,
    // Only filled in for admins, who see everyone's personal machines.
    people: HashMap<UserId, String>,
}

impl Owners {
    pub async fn load(database: &Database, access: &Access) -> Result<Self> {
        let organizations = if access.admin {
            database.organizations().await?
        } else {
            access
                .memberships
                .iter()
                .map(|membership| membership.organization.clone())
                .collect()
        };
        let mut projects = HashMap::new();
        for organization in &organizations {
            for project in database.projects(organization.id).await? {
                projects.insert(project.id, project.name);
            }
        }
        let mut people = HashMap::new();
        if access.admin {
            for account in database.accounts().await? {
                people.insert(account.user, account.username);
            }
        }
        Ok(Self {
            organizations: organizations
                .into_iter()
                .map(|organization| (organization.id, organization.name))
                .collect(),
            projects,
            people,
        })
    }

    /// Whose the machine is, in words. A machine Lemnos did not create is
    /// the organisation's when looking at one, and nobody's in particular
    /// otherwise.
    pub fn label(&self, access: &Access, record: Option<&MachineRecord>) -> String {
        let unknown = || "-".to_owned();
        match record.map(|record| record.owner) {
            Some(Owner::Person(user)) if user == access.user.id => "You".to_owned(),
            Some(Owner::Person(user)) => self.people.get(&user).cloned().unwrap_or_else(unknown),
            Some(Owner::Project { project, .. }) => self
                .projects
                .get(&project)
                .map_or_else(unknown, |name| format!("Project {name}")),
            Some(Owner::Organization(organization)) => self
                .organizations
                .get(&organization)
                .cloned()
                .unwrap_or_else(unknown),
            None => match &access.scope {
                Scope::Organization(membership) => membership.organization.name.clone(),
                Scope::Everything | Scope::Personal => unknown(),
            },
        }
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

// An admin of an installation with nothing connected yet is shown what to
// do about it instead.
#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let access = access(cx).await?;
    let query = query_params::<Query>(cx)?;
    let database = database(cx);

    let visible = access.visible(database).await?;
    let nothing_connected =
        access.admin && visible.is_empty() && database.proxmox_environments().await?.is_empty();
    if nothing_connected {
        return Err(see_other(href!(welcome::page).resolve(cx)).into());
    }
    let owners = Owners::load(database, &access).await?;
    let personal = matches!(access.scope, Scope::Personal);

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
        Scope::Everything | Scope::Personal => None,
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
                <a href=(href!(new::page)) class="button button-primary">"New machine"</a>
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

            if loaded.is_empty() && personal {
                <div class="panel items-start">
                    <h2 class="section-title">"No machines yet"</h2>
                    <p class="text-ink-muted max-w-[65ch]">
                        "Your personal machines show up here. Everyone gets one to start with; machines for an organisation or a project are made for it instead."
                    </p>
                    <a href=(href!(new::page)) class="button button-primary">"Create your machine"</a>
                </div>
            } else if loaded.is_empty() {
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
                                <th>"Owner"</th>
                                <th>"Environment"</th>
                                <th>"CPU"</th>
                                <th>"Memory"</th>
                                <th><span class="sr-only">"Power"</span></th>
                            </tr>
                        </thead>
                        <tbody>
                            if machines == 0 {
                                <tr>
                                    <td colspan="9" class="empty wide">
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
                                            <td data-label="Owner" class="meta">
                                                (owners.label(&access, entry.visible.record(guest)))
                                            </td>
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
                                                <div class="flex flex-wrap items-center gap-2">
                                                    // Viewers look; they get no buttons.
                                                    if access.can_operate(&entry.visible, guest) {
                                                        power_buttons(
                                                            environment: entry.visible.environment.id.0,
                                                            guest: guest,
                                                            back: "machines",
                                                        )
                                                    }
                                                    // Proxmox only deletes a stopped machine.
                                                    if access.can_delete(&entry.visible, guest)
                                                        && guest.status == GuestStatus::Stopped
                                                    {
                                                        <a
                                                            href=(remove::link(
                                                                cx,
                                                                entry.visible.environment.id.0,
                                                                &guest.id.vmid.to_string(),
                                                            ))
                                                            class="button button-sm button-danger"
                                                        >
                                                            "Delete"
                                                        </a>
                                                    }
                                                </div>
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
