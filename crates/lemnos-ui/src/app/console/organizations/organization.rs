// /organizations/{organization}: who is in one organisation, its projects,
// which environments it was given and how many machines it may create. Its
// people can read it; owners and admins change who is in it and in its
// projects; admins change what it was given.
//
//   GET  /organizations/{organization}                           the overview
//   POST /organizations/{organization}/members                   add someone, or change their role
//   POST /organizations/{organization}/members/remove            take someone out
//   POST /organizations/{organization}/projects                  start a project
//   POST /organizations/{organization}/projects/remove           end one
//   POST /organizations/{organization}/projects/members          put someone in a project
//   POST /organizations/{organization}/projects/members/remove   take them out again
//   POST /organizations/{organization}/quota                     how many machines it may create
//   POST /organizations/{organization}/environments              give an environment, or change its tag
//   POST /organizations/{organization}/environments/remove       take one back

use lemnos_auth::{Store, UserId, Username};
use lemnos_db::{
    Organization as Stored, OrganizationId, OrganizationStoreError, Project, ProjectId,
    ProxmoxEnvironmentId, Role,
};
use serde::Deserialize;
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{RouterErrorExt, SeeOther, forbidden, see_other},
        href, module_param, page, path_param, query_params, route,
    },
    view::{View, view},
};

use super::role_label;
use crate::{
    access::{Access, access, admin_access, database},
    components::notice,
};

module_param!(pub organization: u64, error = not_found);

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
}

fn error_message(code: &str) -> &'static str {
    match code {
        "user" => {
            "There is no account with that username. The person has to sign in to Lemnos once before they can be added."
        }
        "tag" => "A tag is at most 64 letters, digits, dashes, dots, underscores or plus signs.",
        "environment" => "Choose an environment.",
        "project" => "Give the project a name of at most 64 characters.",
        "project-taken" => "Another project here already has that name.",
        "outsider" => {
            "Only people in the organisation can be in its projects. Add them above first."
        }
        _ => "That did not work. Check the form and try again.",
    }
}

fn back(cx: &Cx, organization: OrganizationId, error: Option<&str>) -> String {
    let overview = href!(page, Organization(organization.0)).resolve(cx);
    match error {
        Some(code) => format!("{overview}?error={code}"),
        None => overview,
    }
}

/// The organisation in the URL, for someone who may read it.
async fn readable(cx: &Cx) -> Result<(Access, Stored)> {
    let access = access(cx).await?;
    let id = OrganizationId(*path_param::<Organization>(cx)?);
    let stored = database(cx).organization(id).await?.ok_or_not_found()?;
    if !access.admin && access.role_in(id).is_none() {
        return Err(forbidden().into());
    }
    Ok((access, stored))
}

/// The same, for someone who may change who is in it.
async fn manageable(cx: &Cx) -> Result<(Access, Stored)> {
    let (access, stored) = readable(cx).await?;
    if !access.can_manage_members(stored.id) {
        return Err(forbidden().into());
    }
    Ok((access, stored))
}

/// A granted environment, as its row shows it.
struct Given {
    id: u64,
    name: String,
    tag: String,
}

/// A project and its people, as its row shows it.
struct Team {
    project: Project,
    people: Vec<(UserId, String)>,
}

/// The project in the form, if it is one of this organisation's.
async fn own_project(cx: &Cx, organization: OrganizationId, project: u64) -> Result<Project> {
    let project = database(cx)
        .project(ProjectId(project))
        .await?
        .filter(|project| project.organization == organization)
        .ok_or_not_found()?;
    Ok(project)
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let (access, organization) = readable(cx).await?;
    let query = query_params::<Query>(cx)?;
    let database = database(cx);

    let members = database.members(organization.id).await?;
    let grants = database.grants(organization.id).await?;
    let environments = database.proxmox_environments().await?;
    let given: Vec<Given> = grants
        .iter()
        .filter_map(|entry| {
            let environment = environments
                .iter()
                .find(|environment| environment.id == entry.environment)?;
            Some(Given {
                id: environment.id.0,
                name: environment.name.clone(),
                tag: entry.tag.clone(),
            })
        })
        .collect();
    let mut teams = Vec::new();
    for project in database.projects(organization.id).await? {
        let people = database
            .project_members(project.id)
            .await?
            .into_iter()
            .map(|member| (member.user, member.username))
            .collect();
        teams.push(Team { project, people });
    }
    let quota = database.machine_quota(organization.id).await?;
    let in_use = database.organization_machines(organization.id).await?.len();
    let quota_text = match quota {
        0 => "Its people may not create machines yet; an admin can allow some.".to_owned(),
        1 => format!("Its people may create 1 machine, its projects' included. {in_use} in use."),
        _ => format!(
            "Its people may create {quota} machines, its projects' included. {in_use} in use."
        ),
    };
    let manages = access.can_manage_members(organization.id);
    let id = organization.id.0;
    let split = "grid gap-8 lg:grid-cols-[minmax(0,1fr)_24rem] items-start";

    Ok(view! {
        <div class="page">
            <div class="flex flex-col gap-y-2">
                <a href=(href!(super::page)) class="back-link self-start">"All organisations"</a>
                <div class="page-head">
                    <div>
                        <h1>(organization.name.as_str())</h1>
                        if let Some(role) = access.role_in(organization.id) {
                            <p>"You are " (role_label(role).to_lowercase()) " here."</p>
                        } else {
                            <p>"You are not in this organisation; you see it as an admin."</p>
                        }
                    </div>
                </div>
            </div>

            if let Some(code) = &query.error {
                notice(tone: "danger", message: error_message(code))
            }

            <section class="flex flex-col gap-y-3">
                <h2 class="section-title">"People"</h2>
                <div class=(if manages { split } else { "" })>
                    <div class="data data-stack">
                        <table>
                            <thead>
                                <tr>
                                    <th>"Username"</th>
                                    <th>"Name"</th>
                                    <th>"Role"</th>
                                    if manages {
                                        <th><span class="sr-only">"Change"</span></th>
                                    }
                                </tr>
                            </thead>
                            <tbody>
                                if members.is_empty() {
                                    <tr>
                                        <td colspan="4" class="empty wide">
                                            "Nobody is in this organisation yet."
                                        </td>
                                    </tr>
                                }
                                for member in &members {
                                    <tr>
                                        <td data-label="Username" class="name">(member.username.as_str())</td>
                                        <td data-label="Name">(member.name.as_str())</td>
                                        if manages {
                                            <td data-label="Role">
                                                <form method="post" action=(href!(set_member, Organization(id))) class="inline-form">
                                                    <input type="hidden" name="username" value=(member.username.as_str())>
                                                    <select name="role" class="input input-sm" aria-label=(format!("Role of {}", member.username))>
                                                        for role in Role::ALL {
                                                            if role == member.role {
                                                                <option value=(role.as_str()) selected="">(role_label(role))</option>
                                                            } else {
                                                                <option value=(role.as_str())>(role_label(role))</option>
                                                            }
                                                        }
                                                    </select>
                                                    <button type="submit" class="button button-sm">"Save"</button>
                                                </form>
                                            </td>
                                            <td class="wide">
                                                <div class="actions">
                                                    <form method="post" action=(href!(remove_member, Organization(id)))>
                                                        <input type="hidden" name="user" value=(member.user.0.to_string())>
                                                        <button type="submit" class="button button-sm button-danger">
                                                            "Remove"
                                                        </button>
                                                    </form>
                                                </div>
                                            </td>
                                        } else {
                                            <td data-label="Role">(role_label(member.role))</td>
                                        }
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </div>

                    if manages {
                        <section class="panel">
                            <h2 class="section-title">"Add a person"</h2>
                            <form method="post" action=(href!(set_member, Organization(id))) class="flex flex-col gap-y-4">
                                <div class="field">
                                    <label for="username" class="label">"Username"</label>
                                    <input type="text" id="username" name="username" class="input" maxlength="64" autocomplete="off" required="">
                                    <p class="hint">"They need to have signed in to Lemnos once."</p>
                                </div>
                                <div class="field">
                                    <label for="role" class="label">"Role"</label>
                                    <select id="role" name="role" class="input">
                                        <option value="member" selected="">"Member: uses the machines"</option>
                                        <option value="viewer">"Viewer: sees them, changes nothing"</option>
                                        <option value="owner">"Owner: also manages people"</option>
                                    </select>
                                </div>
                                <button type="submit" class="button button-primary self-start">"Add person"</button>
                            </form>
                        </section>
                    }
                </div>
            </section>

            <section class="flex flex-col gap-y-3">
                <h2 class="section-title">"Projects"</h2>
                <p class="text-ink-muted max-w-[65ch]">
                    "A project is a few of the organisation's people with machines of their own. Everyone in the organisation sees them; only the project's people (and owners) change them."
                </p>
                <div class=(if manages { split } else { "" })>
                    <div class="data data-stack">
                        <table>
                            <thead>
                                <tr>
                                    <th>"Project"</th>
                                    <th>"People"</th>
                                    if manages {
                                        <th><span class="sr-only">"Change"</span></th>
                                    }
                                </tr>
                            </thead>
                            <tbody>
                                if teams.is_empty() {
                                    <tr>
                                        <td colspan="3" class="empty wide">"No projects yet."</td>
                                    </tr>
                                }
                                for team in &teams {
                                    <tr>
                                        <td data-label="Project" class="name">(team.project.name.as_str())</td>
                                        <td data-label="People">
                                            if team.people.is_empty() {
                                                "Nobody yet"
                                            }
                                            if manages {
                                                <div class="flex flex-wrap gap-2">
                                                    for (user, username) in &team.people {
                                                        <form method="post" action=(href!(remove_project_member, Organization(id)))>
                                                            <input type="hidden" name="project" value=(team.project.id.0.to_string())>
                                                            <input type="hidden" name="user" value=(user.0.to_string())>
                                                            <button
                                                                type="submit"
                                                                class="button button-sm"
                                                                aria-label=(format!("Take {username} out of {}", team.project.name))
                                                                title="Take out of the project"
                                                            >
                                                                (username.as_str()) " ×"
                                                            </button>
                                                        </form>
                                                    }
                                                </div>
                                            } else {
                                                (team.people.iter().map(|(_, username)| username.as_str()).collect::<Vec<_>>().join(", "))
                                            }
                                        </td>
                                        if manages {
                                            <td class="wide">
                                                <div class="actions">
                                                    <form method="post" action=(href!(add_project_member, Organization(id))) class="inline-form">
                                                        <input type="hidden" name="project" value=(team.project.id.0.to_string())>
                                                        <select name="user" class="input input-sm" aria-label=(format!("Person to add to {}", team.project.name))>
                                                            for member in &members {
                                                                <option value=(member.user.0.to_string())>(member.username.as_str())</option>
                                                            }
                                                        </select>
                                                        <button type="submit" class="button button-sm">"Add"</button>
                                                    </form>
                                                    <form method="post" action=(href!(remove_project, Organization(id)))>
                                                        <input type="hidden" name="project" value=(team.project.id.0.to_string())>
                                                        <button type="submit" class="button button-sm button-danger">"End project"</button>
                                                    </form>
                                                </div>
                                            </td>
                                        }
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </div>

                    if manages {
                        <section class="panel">
                            <h2 class="section-title">"Start a project"</h2>
                            <form method="post" action=(href!(add_project, Organization(id))) class="flex flex-col gap-y-4">
                                <div class="field">
                                    <label for="project-name" class="label">"Name"</label>
                                    <input type="text" id="project-name" name="name" class="input" maxlength="64" autocomplete="off" required="">
                                    <p class="hint">"Ending a project later leaves its machines to the organisation."</p>
                                </div>
                                <button type="submit" class="button button-primary self-start">"Start project"</button>
                            </form>
                        </section>
                    }
                </div>
            </section>

            <section class="flex flex-col gap-y-3">
                <h2 class="section-title">"Machines it may create"</h2>
                <p class="text-ink-muted max-w-[65ch]">(quota_text.as_str())</p>
                if access.admin {
                    <form method="post" action=(href!(set_quota, Organization(id))) class="inline-form">
                        <input type="number" name="quota" class="input input-sm w-24" min="0" max="10000" value=(quota.to_string()) aria-label="Machines it may create">
                        <button type="submit" class="button button-sm">"Save"</button>
                    </form>
                }
            </section>

            <section class="flex flex-col gap-y-3">
                <h2 class="section-title">"Environments"</h2>
                <div class=(if access.admin { split } else { "" })>
                    <div class="data data-stack">
                        <table>
                            <thead>
                                <tr>
                                    <th>"Environment"</th>
                                    <th>"Machines"</th>
                                    if access.admin {
                                        <th><span class="sr-only">"Take back"</span></th>
                                    }
                                </tr>
                            </thead>
                            <tbody>
                                if given.is_empty() {
                                    <tr>
                                        <td colspan="3" class="empty wide">
                                            "This organisation has not been given an environment, so its people see no machines."
                                        </td>
                                    </tr>
                                }
                                for entry in &given {
                                    <tr>
                                        <td data-label="Environment" class="name">(entry.name.as_str())</td>
                                        <td data-label="Machines">
                                            if entry.tag.is_empty() {
                                                "Every machine"
                                            } else {
                                                "Tagged "
                                                <span class="figure">(entry.tag.as_str())</span>
                                            }
                                        </td>
                                        if access.admin {
                                            <td class="wide">
                                                <div class="actions">
                                                    <form method="post" action=(href!(revoke, Organization(id)))>
                                                        <input type="hidden" name="environment" value=(entry.id.to_string())>
                                                        <button type="submit" class="button button-sm button-danger">
                                                            "Take back"
                                                        </button>
                                                    </form>
                                                </div>
                                            </td>
                                        }
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </div>

                    if access.admin {
                        <section class="panel">
                            <h2 class="section-title">"Give an environment"</h2>
                            if environments.is_empty() {
                                <p class="text-ink-muted">
                                    "No environment is connected yet. "
                                    <a href=(href!(super::super::environments::page)) class="link">"Add one first"</a>
                                </p>
                            } else {
                                <form method="post" action=(href!(grant, Organization(id))) class="flex flex-col gap-y-4">
                                    <div class="field">
                                        <label for="environment" class="label">"Environment"</label>
                                        <select id="environment" name="environment" class="input" required="">
                                            for environment in &environments {
                                                <option value=(environment.id.0.to_string())>(environment.name.as_str())</option>
                                            }
                                        </select>
                                    </div>
                                    <div class="field">
                                        <label for="tag" class="label">"Tag on its machines"</label>
                                        <input type="text" id="tag" name="tag" class="input figure" maxlength="64" placeholder="e.g. org-platform" autocomplete="off">
                                        <p class="hint">"Only machines carrying this tag belong to the organisation. Leave empty to give it every machine in the environment."</p>
                                    </div>
                                    <button type="submit" class="button button-primary self-start">"Give environment"</button>
                                </form>
                            }
                        </section>
                    }
                </div>
            </section>
        </div>
    })
}

#[derive(Deserialize)]
pub struct MemberForm {
    username: String,
    role: String,
}

// Adds the person, or changes the role of someone who is already in.
#[route(POST "./members")]
pub async fn set_member(cx: &Cx, Form(input): Form<MemberForm>) -> Result<SeeOther> {
    let (_, organization) = manageable(cx).await?;
    let database = database(cx);
    let Some(role) = Role::parse(&input.role) else {
        return Ok(see_other(back(cx, organization.id, Some("role"))));
    };
    let user = match Username::parse(&input.username) {
        Ok(username) => database.user_by_username(&username).await?,
        Err(_) => None,
    };
    let Some(user) = user else {
        return Ok(see_other(back(cx, organization.id, Some("user"))));
    };
    database
        .set_membership(user.id, organization.id, role)
        .await?;
    Ok(see_other(back(cx, organization.id, None)))
}

#[derive(Deserialize)]
pub struct RemoveMemberForm {
    user: u64,
}

#[route(POST "./members/remove")]
pub async fn remove_member(cx: &Cx, Form(input): Form<RemoveMemberForm>) -> Result<SeeOther> {
    let (_, organization) = manageable(cx).await?;
    database(cx)
        .remove_membership(UserId(input.user), organization.id)
        .await?;
    Ok(see_other(back(cx, organization.id, None)))
}

#[derive(Deserialize)]
pub struct ProjectForm {
    name: String,
}

#[route(POST "./projects")]
pub async fn add_project(cx: &Cx, Form(input): Form<ProjectForm>) -> Result<SeeOther> {
    let (_, organization) = manageable(cx).await?;
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Ok(see_other(back(cx, organization.id, Some("project"))));
    }
    Ok(see_other(
        match database(cx).create_project(organization.id, name).await {
            Ok(_) => back(cx, organization.id, None),
            Err(OrganizationStoreError::NameTaken) => {
                back(cx, organization.id, Some("project-taken"))
            }
            Err(error) => return Err(error.into()),
        },
    ))
}

#[derive(Deserialize)]
pub struct WhichProject {
    project: u64,
}

#[route(POST "./projects/remove")]
pub async fn remove_project(cx: &Cx, Form(input): Form<WhichProject>) -> Result<SeeOther> {
    let (_, organization) = manageable(cx).await?;
    let project = own_project(cx, organization.id, input.project).await?;
    database(cx).delete_project(project.id).await?;
    Ok(see_other(back(cx, organization.id, None)))
}

#[derive(Deserialize)]
pub struct ProjectMemberForm {
    project: u64,
    user: u64,
}

#[route(POST "./projects/members")]
pub async fn add_project_member(cx: &Cx, Form(input): Form<ProjectMemberForm>) -> Result<SeeOther> {
    let (_, organization) = manageable(cx).await?;
    let project = own_project(cx, organization.id, input.project).await?;
    let database = database(cx);
    let user = UserId(input.user);
    let member = database
        .members(organization.id)
        .await?
        .iter()
        .any(|member| member.user == user);
    if !member {
        return Ok(see_other(back(cx, organization.id, Some("outsider"))));
    }
    database.add_project_member(project.id, user).await?;
    Ok(see_other(back(cx, organization.id, None)))
}

#[route(POST "./projects/members/remove")]
pub async fn remove_project_member(
    cx: &Cx,
    Form(input): Form<ProjectMemberForm>,
) -> Result<SeeOther> {
    let (_, organization) = manageable(cx).await?;
    let project = own_project(cx, organization.id, input.project).await?;
    database(cx)
        .remove_project_member(project.id, UserId(input.user))
        .await?;
    Ok(see_other(back(cx, organization.id, None)))
}

#[derive(Deserialize)]
pub struct QuotaForm {
    quota: u64,
}

#[route(POST "./quota")]
pub async fn set_quota(cx: &Cx, Form(input): Form<QuotaForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    let (_, organization) = readable(cx).await?;
    database(cx)
        .set_machine_quota(organization.id, input.quota)
        .await?;
    Ok(see_other(back(cx, organization.id, None)))
}

#[derive(Deserialize)]
pub struct GrantForm {
    environment: u64,
    tag: String,
}

/// Proxmox keeps tags in lower case; so does Lemnos.
fn parse_tag(input: &str) -> Option<String> {
    let tag = input.trim().to_lowercase();
    let allowed =
        |character: char| character.is_alphanumeric() || matches!(character, '-' | '_' | '.' | '+');
    (tag.chars().count() <= 64 && tag.chars().all(allowed)).then_some(tag)
}

#[route(POST "./environments")]
pub async fn grant(cx: &Cx, Form(input): Form<GrantForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    let (_, organization) = readable(cx).await?;
    let database = database(cx);
    let Some(tag) = parse_tag(&input.tag) else {
        return Ok(see_other(back(cx, organization.id, Some("tag"))));
    };
    let environment = ProxmoxEnvironmentId(input.environment);
    if database.proxmox_environment(environment).await?.is_none() {
        return Ok(see_other(back(cx, organization.id, Some("environment"))));
    }
    database
        .grant_environment(organization.id, environment, &tag)
        .await?;
    Ok(see_other(back(cx, organization.id, None)))
}

#[derive(Deserialize)]
pub struct RevokeForm {
    environment: u64,
}

#[route(POST "./environments/remove")]
pub async fn revoke(cx: &Cx, Form(input): Form<RevokeForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    let (_, organization) = readable(cx).await?;
    database(cx)
        .revoke_environment(organization.id, ProxmoxEnvironmentId(input.environment))
        .await?;
    Ok(see_other(back(cx, organization.id, None)))
}

#[cfg(test)]
mod tests {
    use super::parse_tag;

    #[test]
    fn tags_are_trimmed_and_lowercased() {
        assert_eq!(parse_tag(" Org-Platform "), Some("org-platform".to_owned()));
        assert_eq!(parse_tag(""), Some(String::new()));
    }

    #[test]
    fn odd_tags_are_refused() {
        assert_eq!(parse_tag("two words"), None);
        assert_eq!(parse_tag("a;b"), None);
        assert_eq!(parse_tag(&"a".repeat(65)), None);
    }
}
