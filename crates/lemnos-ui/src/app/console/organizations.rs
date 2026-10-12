// /organizations: the groups of people who share machines. An admin sees
// all of them and adds or removes one; anyone else sees the ones they are
// in. `organization.rs` shows who is in one and what it was given.

use lemnos_db::{OrganizationId, OrganizationStoreError};
use serde::Deserialize;
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{SeeOther, see_other},
        href, page, query_params, route,
    },
    view::{View, view},
};

use crate::{
    access::{access, admin_access, database},
    components::notice,
};

pub mod organization;

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
}

fn error_message(code: &str) -> &'static str {
    match code {
        "taken" => "Another organisation already has that name.",
        _ => "Give the organisation a name of at most 64 characters.",
    }
}

fn back(cx: &Cx, error: Option<&str>) -> String {
    let list = href!(page).resolve(cx);
    match error {
        Some(code) => format!("{list}?error={code}"),
        None => list,
    }
}

/// One row of the list.
struct Row {
    id: u64,
    name: String,
    // For an admin: how many people and environments. For anyone else:
    // their own role.
    members: usize,
    environments: usize,
    role: &'static str,
}

/// The role as it is shown to people.
pub fn role_label(role: lemnos_db::Role) -> &'static str {
    match role {
        lemnos_db::Role::Owner => "Owner",
        lemnos_db::Role::Member => "Member",
        lemnos_db::Role::Viewer => "Viewer",
    }
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let access = access(cx).await?;
    let query = query_params::<Query>(cx)?;
    let database = database(cx);

    let mut rows = Vec::new();
    if access.admin {
        for known in database.organizations().await? {
            rows.push(Row {
                id: known.id.0,
                members: database.members(known.id).await?.len(),
                environments: database.grants(known.id).await?.len(),
                role: access.role_in(known.id).map_or("", role_label),
                name: known.name,
            });
        }
    } else {
        for membership in &access.memberships {
            rows.push(Row {
                id: membership.organization.id.0,
                name: membership.organization.name.clone(),
                members: 0,
                environments: 0,
                role: role_label(membership.role),
            });
        }
    }

    Ok(view! {
        <div class="page">
            <div class="page-head">
                <div>
                    <h1>"Organisations"</h1>
                    if access.admin {
                        <p>"Groups of people who share machines. Each is given environments and sees its own machines there."</p>
                    } else {
                        <p>"The organisations you are in."</p>
                    }
                </div>
            </div>

            // The list takes the room that is left; the form keeps its width.
            <div class=(if access.admin { "grid gap-8 lg:grid-cols-[minmax(0,1fr)_24rem] items-start" } else { "" })>
                <div class="data data-stack">
                    <table>
                        <thead>
                            <tr>
                                <th>"Name"</th>
                                if access.admin {
                                    <th>"People"</th>
                                    <th>"Environments"</th>
                                } else {
                                    <th>"Your role"</th>
                                }
                                if access.admin {
                                    <th><span class="sr-only">"Remove"</span></th>
                                }
                            </tr>
                        </thead>
                        <tbody>
                            if rows.is_empty() {
                                <tr>
                                    <td colspan="4" class="empty wide">
                                        if access.admin {
                                            "No organisations yet. Add the first one with the form."
                                        } else {
                                            "You are not in an organisation yet. An admin can add you."
                                        }
                                    </td>
                                </tr>
                            }
                            for row in &rows {
                                <tr>
                                    <td data-label="Name" class="name">
                                        <a
                                            href=(href!(organization::page, organization::Organization(row.id)))
                                            class="link"
                                        >
                                            (row.name.as_str())
                                        </a>
                                    </td>
                                    if access.admin {
                                        <td data-label="People" class="figure">(row.members.to_string())</td>
                                        <td data-label="Environments" class="figure">(row.environments.to_string())</td>
                                        <td class="wide">
                                            <div class="actions">
                                                <form method="post" action=(href!(remove))>
                                                    <input type="hidden" name="id" value=(row.id.to_string())>
                                                    <button type="submit" class="button button-sm button-danger">
                                                        "Remove"
                                                    </button>
                                                </form>
                                            </div>
                                        </td>
                                    } else {
                                        <td data-label="Your role">(row.role)</td>
                                    }
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>

                if access.admin {
                    <section class="panel">
                        <h2 class="section-title">"Add an organisation"</h2>
                        if let Some(code) = &query.error {
                            notice(tone: "danger", message: error_message(code))
                        }
                        <form method="post" class="flex flex-col gap-y-4">
                            <div class="field">
                                <label for="name" class="label">"Name"</label>
                                <input
                                    type="text"
                                    id="name"
                                    name="name"
                                    class="input"
                                    maxlength="64"
                                    placeholder="e.g. Platform"
                                    required=""
                                >
                                <p class="hint">"People and environments are added on its own page."</p>
                            </div>
                            <button type="submit" class="button button-primary self-start">
                                "Add organisation"
                            </button>
                        </form>
                    </section>
                }
            </div>
            if access.admin {
                <p class="text-meta text-ink-faint">
                    "Removing an organisation takes its people out of it. Accounts, environments and machines stay as they are."
                </p>
            }
        </div>
    })
}

#[derive(Deserialize)]
pub struct NewOrganizationForm {
    name: String,
}

#[route(POST)]
pub async fn add(cx: &Cx, Form(input): Form<NewOrganizationForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Ok(see_other(back(cx, Some("name"))));
    }
    Ok(see_other(
        match database(cx).create_organization(name).await {
            Ok(created) => {
                href!(organization::page, organization::Organization(created.id.0)).resolve(cx)
            }
            Err(OrganizationStoreError::NameTaken) => back(cx, Some("taken")),
            Err(error) => return Err(error.into()),
        },
    ))
}

#[derive(Deserialize)]
pub struct RemoveForm {
    id: u64,
}

#[route(POST "./remove")]
pub async fn remove(cx: &Cx, Form(input): Form<RemoveForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    database(cx)
        .delete_organization(OrganizationId(input.id))
        .await?;
    Ok(see_other(back(cx, None)))
}
