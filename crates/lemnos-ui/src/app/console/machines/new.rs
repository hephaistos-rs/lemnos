// /machines/new: a new machine, copied from a template, for the user
// themselves, one of their projects or one of their organisations.
//
//   GET  /machines/new?owner=...   the form, for one owner
//   POST /machines/new             creates it
//
// Personal machines go to the one environment an admin set aside for them
// (an admin's own can go anywhere). Organisation and project machines go to
// an environment the organisation was given, and carry its tag there.
// Copying is `lemnos-proxmox`'s job; this page decides who may, where, and
// how many.

use std::time::Duration;

use lemnos_db::{Database, MachineRecord, OrganizationId, Owner, ProxmoxEnvironment};
use lemnos_proxmox::guests::{self, Guest, GuestId, GuestKind, VmId, provision};
use serde::{Deserialize, Serialize};
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{SeeOther, forbidden, see_other},
        href, page, query_params, route,
    },
    view::{View, view},
};

use super::super::environments::{self, environment::kind_label};
use crate::{
    access::{
        Access, PERSONAL_LIMIT, Scope, access, choice_for, choose, database, owner_value,
        parse_owner,
    },
    components::{notice, problem},
};

/// How long the button waits for Proxmox to copy the template. A large disk
/// takes longer; the copy then carries on in the background.
const CREATE_WAIT: Duration = Duration::from_secs(30);

#[query_params(error = bad_request)]
struct Query {
    owner: Option<String>,
    error: Option<String>,
    // What Proxmox said, when it was Proxmox that refused.
    detail: Option<String>,
}

fn error_message(code: &str) -> &'static str {
    match code {
        "name" => {
            "A name is 1 to 63 lower-case letters, digits and dashes, and does not start or end with a dash."
        }
        "template" => "Choose one of the templates.",
        "full" => {
            "There is no room for another machine. Delete one first, or ask an admin for more."
        }
        "nowhere" => "There is no environment to create it in.",
        "unreachable" => "Lemnos can't reach that environment right now.",
        _ => "Creating the machine did not work.",
    }
}

#[derive(Serialize)]
struct Back<'a> {
    owner: &'a str,
    error: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
}

fn back(cx: &Cx, owner: Owner, error: &str, detail: Option<&str>) -> SeeOther {
    see_other(
        href!(page)
            .query(Back {
                owner: &owner_value(owner),
                error,
                detail,
            })
            .resolve(cx),
    )
}

/// An owner the user may create machines for, as the picker shows it.
struct Choice {
    owner: Owner,
    value: String,
    name: String,
}

async fn choices(database: &Database, access: &Access) -> Result<Vec<Choice>> {
    let mut choices = vec![Choice {
        owner: Owner::Person(access.user.id),
        value: owner_value(Owner::Person(access.user.id)),
        name: "Personal".to_owned(),
    }];
    let organizations = if access.admin {
        database.organizations().await?
    } else {
        access
            .memberships
            .iter()
            .map(|membership| membership.organization.clone())
            .collect()
    };
    for organization in organizations {
        let owner = Owner::Organization(organization.id);
        if access.may_create_for(owner) {
            choices.push(Choice {
                owner,
                value: owner_value(owner),
                name: organization.name.clone(),
            });
        }
        for project in database.projects(organization.id).await? {
            let owner = Owner::Project {
                organization: organization.id,
                project: project.id,
            };
            if access.may_create_for(owner) {
                choices.push(Choice {
                    owner,
                    value: owner_value(owner),
                    name: format!("{} / {}", organization.name, project.name),
                });
            }
        }
    }
    Ok(choices)
}

/// An environment machines for the owner can go to, and the tag they get
/// there so the organisation sees them.
struct Destination {
    environment: ProxmoxEnvironment,
    tag: Option<String>,
}

async fn destinations(
    database: &Database,
    access: &Access,
    owner: Owner,
) -> Result<Vec<Destination>> {
    let environments = database.proxmox_environments().await?;
    let untagged = |environment| Destination {
        environment,
        tag: None,
    };
    Ok(match owner.organization() {
        // An admin's management machines can go anywhere.
        None if access.admin => environments.into_iter().map(untagged).collect(),
        None => {
            let personal = database.personal_environment().await?;
            environments
                .into_iter()
                .filter(|environment| Some(environment.id) == personal)
                .map(untagged)
                .collect()
        }
        Some(organization) => {
            let grants = database.grants(organization).await?;
            environments
                .into_iter()
                .filter_map(|environment| {
                    let grant = grants
                        .iter()
                        .find(|grant| grant.environment == environment.id)?;
                    Some(Destination {
                        environment,
                        tag: Some(grant.tag.clone()).filter(|tag| !tag.is_empty()),
                    })
                })
                .collect()
        }
    })
}

/// The templates a machine can be copied from. Where the organisation only
/// sees machines with its tag, that is templates with its tag and templates
/// with no tags at all, which are there for everyone.
async fn templates(destination: &Destination) -> std::result::Result<Vec<Guest>, String> {
    let client = environments::connect(&destination.environment)
        .await
        .map_err(|error| error.to_string())?;
    let mut templates = provision::templates(&client)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(tag) = &destination.tag {
        templates.retain(|template| {
            template.tags.is_empty() || template.tags.iter().any(|carried| carried == tag)
        });
    }
    Ok(templates)
}

/// The records that count against their owner. A record whose guest is
/// gone (deleted behind Lemnos' back) is forgotten.
///
/// One whose ID points at a guest without its marker still counts, and is
/// left for an admin to confirm or release (see the environment's page). It
/// gives its owner nothing, but a copy that outlasted `CREATE_WAIT` looks
/// just like that until its tags are set, and must not make room for
/// another. An environment that can't be reached keeps its records too.
async fn prune(database: &Database, records: Vec<MachineRecord>) -> Result<Vec<MachineRecord>> {
    let mut kept = Vec::new();
    let mut checked: Vec<(lemnos_db::ProxmoxEnvironmentId, Option<Vec<Guest>>)> = Vec::new();
    for record in records {
        if !checked.iter().any(|(id, _)| *id == record.environment) {
            let existing = match database.proxmox_environment(record.environment).await? {
                Some(environment) => match environments::connect(&environment).await {
                    Ok(client) => guests::list(&client).await.ok(),
                    Err(_) => None,
                },
                None => None,
            };
            checked.push((record.environment, existing));
        }
        let existing = checked
            .iter()
            .find(|(id, _)| *id == record.environment)
            .and_then(|(_, existing)| existing.as_ref());
        let Some(existing) = existing else {
            kept.push(record);
            continue;
        };
        if existing
            .iter()
            .any(|guest| guest.id.vmid.to_string() == record.machine)
        {
            kept.push(record);
        } else {
            database
                .forget_machine(record.environment, &record.machine)
                .await?;
        }
    }
    Ok(kept)
}

/// How many machines the owner has, and how many they may have (`None`:
/// as many as they like).
async fn room(
    database: &Database,
    access: &Access,
    owner: Owner,
) -> Result<(usize, Option<usize>)> {
    let records = match owner {
        Owner::Person(user) => database.personal_machines(user).await?,
        Owner::Project { organization, .. } | Owner::Organization(organization) => {
            database.organization_machines(organization).await?
        }
    };
    let used = prune(database, records).await?.len();
    let limit = match owner.organization() {
        _ if access.admin => None,
        None => Some(PERSONAL_LIMIT),
        Some(organization) => Some(quota(database, organization).await?),
    };
    Ok((used, limit))
}

async fn quota(database: &Database, organization: OrganizationId) -> Result<usize> {
    Ok(usize::try_from(database.machine_quota(organization).await?).unwrap_or(usize::MAX))
}

/// A name Proxmox takes for a VM and as a container's hostname.
fn parse_name(input: &str) -> Option<String> {
    let name = input.trim().to_lowercase();
    let allowed = |character: char| character.is_ascii_alphanumeric() || character == '-';
    (!name.is_empty()
        && name.len() <= 63
        && name.chars().all(allowed)
        && !name.starts_with('-')
        && !name.ends_with('-'))
    .then_some(name)
}

/// Text as a Proxmox tag: lower case, with anything it does not allow
/// turned into dashes.
fn tag_safe(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '+') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

/// The tags a new machine gets: its organisation's, so the organisation
/// sees it, and one naming its owner, for whoever looks in Proxmox itself.
fn tags_for(owner_tag: String, destination: &Destination) -> Vec<String> {
    let mut tags: Vec<String> = destination.tag.iter().cloned().collect();
    tags.push(owner_tag);
    tags
}

/// The template's value in the form: environment, node, kind and ID.
fn template_value(environment: &ProxmoxEnvironment, template: &Guest) -> String {
    let kind = match template.id.kind {
        GuestKind::Qemu => "qemu",
        GuestKind::Lxc => "lxc",
    };
    format!(
        "{}:{}:{kind}:{}",
        environment.id.0, template.id.node, template.id.vmid
    )
}

fn parse_template(value: &str) -> Option<(u64, GuestId)> {
    let mut parts = value.split(':');
    let environment = parts.next()?.parse().ok()?;
    let node = parts.next()?.to_owned();
    let kind = match parts.next()? {
        "qemu" => GuestKind::Qemu,
        "lxc" => GuestKind::Lxc,
        _ => return None,
    };
    let vmid = VmId::try_from(parts.next()?.parse::<i128>().ok()?).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((environment, GuestId { node, vmid, kind }))
}

/// One environment on the form, and its templates or why they are missing.
struct Offer {
    destination: Destination,
    templates: std::result::Result<Vec<Guest>, String>,
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let access = access(cx).await?;
    let query = query_params::<Query>(cx)?;
    let database = database(cx);

    let choices = choices(database, &access).await?;
    // The owner asked for, or else the one that fits what is being looked at.
    let looking_at = match &access.scope {
        Scope::Organization(membership) => {
            owner_value(Owner::Organization(membership.organization.id))
        }
        Scope::Everything | Scope::Personal => owner_value(Owner::Person(access.user.id)),
    };
    let wanted = query.owner.clone().unwrap_or(looking_at);
    // Personal comes first and is always there.
    let chosen = choices
        .iter()
        .position(|choice| choice.value == wanted)
        .unwrap_or(0);
    let owner = choices[chosen].owner;
    let chosen_value = choices[chosen].value.clone();
    let chosen_name = choices[chosen].name.clone();

    let mut offers = Vec::new();
    for destination in destinations(database, &access, owner).await? {
        offers.push(Offer {
            templates: templates(&destination).await,
            destination,
        });
    }
    let any_template = offers.iter().any(|offer| {
        offer
            .templates
            .as_ref()
            .is_ok_and(|templates| !templates.is_empty())
    });
    let (used, limit) = room(database, &access, owner).await?;
    let full = limit.is_some_and(|limit| used >= limit);
    let personal = owner.organization().is_none();
    let counted = match limit {
        Some(limit) if personal => format!(
            "You have {used} of the {limit} personal machine{} you may have.",
            if limit == 1 { "" } else { "s" }
        ),
        Some(limit) => format!(
            "{used} of the {limit} machines this organisation may create are in use, its projects' included."
        ),
        None if access.admin => {
            format!("{used} so far. As an admin you may create as many as you need.")
        }
        None => String::new(),
    };
    let detail = query.detail.clone().unwrap_or_default();
    // Environments whose templates could not be read, as (message, reason).
    let unreachable: Vec<(String, String)> = offers
        .iter()
        .filter_map(|offer| {
            let reason = offer.templates.as_ref().err()?;
            Some((
                format!(
                    "Lemnos can't reach {}, so its templates are missing.",
                    offer.destination.environment.name
                ),
                reason.clone(),
            ))
        })
        .collect();

    Ok(view! {
        <div class="page-mid">
            <div class="flex flex-col gap-y-2">
                <a href=(href!(super::page)) class="back-link self-start">"All machines"</a>
                <div class="page-head">
                    <div>
                        <h1>"New machine"</h1>
                        <p>"A copy of a template, stopped and ready to start."</p>
                    </div>
                </div>
            </div>

            if let Some(code) = &query.error {
                if detail.is_empty() {
                    notice(tone: "danger", message: error_message(code))
                } else {
                    problem(message: error_message(code), detail: detail.as_str(), link: ("", ""))
                }
            }

            if choices.len() > 1 {
                <nav class="choices" aria-label="Whose machine">
                    for choice in &choices {
                        <a
                            href=(href!(page).query(Owned { owner: &choice.value }))
                            class="button button-sm"
                            aria-current=(if choice.value == chosen_value { "true" } else { "false" })
                        >
                            (choice.name.as_str())
                        </a>
                    }
                </nav>
            }

            <section class="panel">
                <h2 class="section-title">
                    if personal { "Personal machine" } else { "For " (chosen_name.as_str()) }
                </h2>
                if !counted.is_empty() {
                    <p class="text-ink-muted">(counted.as_str())</p>
                }

                // An admin's own machines can go to any environment, so for
                // them there is only none when none is connected.
                if offers.is_empty() && personal && access.admin {
                    <p class="text-ink-muted max-w-[65ch]">"No environment is connected yet."</p>
                    <a href=(href!(environments::page)) class="button self-start">"Add an environment"</a>
                } else if offers.is_empty() && personal {
                    <p class="text-ink-muted max-w-[65ch]">
                        "An admin has not chosen where personal machines go yet."
                    </p>
                } else if offers.is_empty() {
                    <p class="text-ink-muted max-w-[65ch]">
                        "This organisation has not been given an environment yet, so there is nowhere to put the machine."
                    </p>
                } else if full {
                    <p class="text-ink-muted max-w-[65ch]">
                        if personal {
                            "Delete the one you have to make a new one."
                        } else {
                            "Delete one of the organisation's machines, or ask an admin to raise the limit."
                        }
                    </p>
                    <a href=(href!(super::page)) class="button self-start">"Go to machines"</a>
                } else {
                    for (message, reason) in &unreachable {
                        problem(message: message.as_str(), detail: reason.as_str(), link: ("", ""))
                    }
                    if any_template {
                        <form method="post" action=(href!(create)) class="flex flex-col gap-y-4">
                            <input type="hidden" name="owner" value=(chosen_value.as_str())>
                            <div class="field">
                                <label for="template" class="label">"Template"</label>
                                <select id="template" name="template" class="input" required="">
                                    for offer in &offers {
                                        if let Ok(templates) = &offer.templates {
                                            if !templates.is_empty() {
                                                <optgroup label=(offer.destination.environment.name.as_str())>
                                                    for template in templates {
                                                        <option value=(template_value(&offer.destination.environment, template))>
                                                            (template.name.as_deref().unwrap_or("Unnamed"))
                                                            " (" (kind_label(template.id.kind)) " " (template.id.vmid.to_string()) ")"
                                                        </option>
                                                    }
                                                </optgroup>
                                            }
                                        }
                                    }
                                </select>
                            </div>
                            <div class="field">
                                <label for="name" class="label">"Name"</label>
                                <input type="text" id="name" name="name" class="input figure" maxlength="63" placeholder="e.g. build-01" autocomplete="off" required="">
                                <p class="hint">"Lower-case letters, digits and dashes. It is also the hostname."</p>
                            </div>
                            <button type="submit" class="button button-primary self-start">"Create machine"</button>
                        </form>
                    } else {
                        <p class="text-ink-muted max-w-[65ch]">
                            "There are no templates to copy yet. In Proxmox, turn a VM or container into a template to offer it here."
                        </p>
                    }
                }
            </section>
        </div>
    })
}

#[derive(Serialize)]
struct Owned<'a> {
    owner: &'a str,
}

#[derive(Deserialize)]
pub struct NewMachineForm {
    owner: String,
    template: String,
    name: String,
}

/// What the machines page says afterwards; see `environment::power_notice`.
#[derive(Serialize)]
struct Done<'a> {
    notice: &'a str,
    action: &'a str,
    machine: &'a str,
}

#[route(POST)]
pub async fn create(cx: &Cx, Form(input): Form<NewMachineForm>) -> Result<SeeOther> {
    let access = access(cx).await?;
    let database = database(cx);
    let Some(owner) = parse_owner(database, &access.user, &input.owner).await? else {
        return Err(forbidden().into());
    };
    if !access.may_create_for(owner) {
        return Err(forbidden().into());
    }
    let Some(name) = parse_name(&input.name) else {
        return Ok(back(cx, owner, "name", None));
    };
    let Some((environment, template)) = parse_template(&input.template) else {
        return Ok(back(cx, owner, "template", None));
    };
    let Some(destination) = destinations(database, &access, owner)
        .await?
        .into_iter()
        .find(|destination| destination.environment.id.0 == environment)
    else {
        return Ok(back(cx, owner, "nowhere", None));
    };
    let (used, limit) = room(database, &access, owner).await?;
    if limit.is_some_and(|limit| used >= limit) {
        return Ok(back(cx, owner, "full", None));
    }
    // The form names the template, so it is checked against what is offered.
    let offered = match templates(&destination).await {
        Ok(offered) => offered,
        Err(reason) => return Ok(back(cx, owner, "unreachable", Some(&reason))),
    };
    if !offered.iter().any(|known| known.id == template) {
        return Ok(back(cx, owner, "template", None));
    }

    let owner_tag = match owner {
        Owner::Person(_) => format!("personal-{}", tag_safe(access.user.username.as_str())),
        Owner::Project { project, .. } => {
            let project = database.project(project).await?;
            format!(
                "project-{}",
                tag_safe(project.as_ref().map_or("", |project| project.name.as_str()))
            )
        }
        Owner::Organization(_) => "lemnos".to_owned(),
    };
    // Ties the record to this guest and no other; see `lemnos_core::marker`.
    let marker = lemnos_core::marker::generate();
    let mut tags = tags_for(owner_tag, &destination);
    tags.push(marker.clone());
    let client = match environments::connect(&destination.environment).await {
        Ok(client) => client,
        Err(error) => return Ok(back(cx, owner, "unreachable", Some(&error.to_string()))),
    };
    let created = match provision::create_from_template(
        &client,
        &template,
        &name,
        &tags,
        CREATE_WAIT,
    )
    .await
    {
        Ok(created) => created,
        Err(error) => return Ok(back(cx, owner, "failed", Some(&error.to_string()))),
    };
    database
        .record_machine(&MachineRecord {
            environment: destination.environment.id,
            machine: created.guest.vmid.to_string(),
            name: name.clone(),
            owner,
            created_by: Some(access.user.id),
            marker: Some(marker),
        })
        .await?;

    // Show the new machine among the ones it belongs with.
    if !matches!(access.scope, Scope::Everything) {
        choose(cx, &choice_for(owner));
    }
    Ok(see_other(
        href!(super::page)
            .query(Done {
                notice: if created.finished { "done" } else { "copying" },
                action: "create",
                machine: &name,
            })
            .resolve(cx),
    ))
}

#[cfg(test)]
mod tests {
    use super::{parse_name, parse_template, tag_safe};

    #[test]
    fn names_are_hostnames() {
        assert_eq!(parse_name(" Build-01 "), Some("build-01".to_owned()));
        assert_eq!(parse_name(""), None);
        assert_eq!(parse_name("-build"), None);
        assert_eq!(parse_name("build-"), None);
        assert_eq!(parse_name("two words"), None);
        assert_eq!(parse_name("build_01"), None);
        assert_eq!(parse_name(&"a".repeat(64)), None);
    }

    #[test]
    fn templates_round_trip_through_the_form() {
        let (environment, template) = parse_template("7:pve1:qemu:9000").unwrap();
        assert_eq!(environment, 7);
        assert_eq!(template.node, "pve1");
        assert_eq!(template.vmid.to_string(), "9000");
        assert!(parse_template("7:pve1:qemu").is_none());
        assert!(parse_template("7:pve1:disk:9000").is_none());
        assert!(parse_template("7:pve1:qemu:9000:extra").is_none());
    }

    #[test]
    fn tags_only_hold_what_proxmox_allows() {
        assert_eq!(tag_safe("Alice@Lemnos.test"), "alice-lemnos.test");
    }
}
