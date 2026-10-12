// Who the signed-in user is and what they may see. Every console page asks
// here first; nothing else decides what a person is allowed to do.
//
//   admin         runs the installation: environments, organisations, users
//   owner         decides who is in an organisation
//   member        uses the organisation's machines
//   viewer        sees them
//
// A person looks at one organisation at a time (or, as an admin, at
// everything). Which one is kept in a cookie and checked on every request.

use std::error::Error;

use lemnos_auth::{Auth, NewUser, Store, User, Username};
use lemnos_db::{Database, Membership, OrganizationId, ProxmoxEnvironment, Role};
use lemnos_proxmox::guests::Guest;
use topcoat::{
    Result,
    context::Cx,
    cookie::{Cookies, SameSite, cookies, time::Duration},
    router::{
        error::{RouterErrorExt, forbidden},
        href,
    },
};

use crate::{
    app::auth::{current_user, sign_in, state},
    config::{AdminConfig, OrganizationConfig},
};

/// Cookie holding the organisation being looked at: its ID, or `all`.
const COOKIE: &str = "organization";
const EVERYTHING: &str = "all";

/// What the user is looking at.
pub enum Scope {
    /// Every environment and every machine. Admins only.
    Everything,
    /// One of the user's organisations.
    Organization(Membership),
    /// The user is in no organisation yet.
    Nothing,
}

pub struct Access {
    pub user: User,
    pub admin: bool,
    pub memberships: Vec<Membership>,
    pub scope: Scope,
}

/// An environment the user may see, and the tag a machine there has to
/// carry to be theirs. `None` means every machine.
pub struct Visible {
    pub environment: ProxmoxEnvironment,
    pub tag: Option<String>,
}

impl Visible {
    pub fn shows(&self, guest: &Guest) -> bool {
        match &self.tag {
            Some(tag) => guest.tags.iter().any(|carried| carried == tag),
            None => true,
        }
    }
}

pub fn database(cx: &Cx) -> &Database {
    state(cx).auth.store()
}

/// The signed-in user and what they may see. Visitors are sent to the
/// sign-in page instead.
pub async fn access(cx: &Cx) -> Result<Access> {
    let user = current_user(cx)
        .await?
        .ok_or_redirect(href!(sign_in::page).resolve(cx))?;
    let database = database(cx);
    let admin = database.is_admin(user.id).await?;
    let memberships = database.memberships_of(user.id).await?;

    let chosen = cookies(cx).get(COOKIE);
    let chosen = chosen.as_ref().map(|cookie| cookie.value());
    let chosen_organization = chosen
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|id| {
            memberships
                .iter()
                .find(|membership| membership.organization.id.0 == id)
        });
    let scope = match chosen_organization {
        Some(membership) => Scope::Organization(membership.clone()),
        // With no valid choice, an admin sees everything and anyone else
        // their first organisation.
        None if admin => Scope::Everything,
        None => match memberships.first() {
            Some(membership) => Scope::Organization(membership.clone()),
            None => Scope::Nothing,
        },
    };
    Ok(Access {
        user,
        admin,
        memberships,
        scope,
    })
}

/// The same, for pages only admins may open.
pub async fn admin_access(cx: &Cx) -> Result<Access> {
    let access = access(cx).await?;
    if !access.admin {
        return Err(forbidden().into());
    }
    Ok(access)
}

/// Remembers which organisation to look at. `choice` is an ID or `all`;
/// whether the user may look at it is checked when it is read back.
pub fn choose(cx: &Cx, choice: &str) {
    let jar = cookies(cx)
        .default_path("/")
        .default_http_only(true)
        .default_same_site(SameSite::Lax)
        .default_max_age(Duration::days(365));
    if choice == EVERYTHING || choice.parse::<u64>().is_ok() {
        jar.add((COOKIE, choice.to_owned()));
    }
}

impl Access {
    /// What to call the scope in the bar.
    pub fn scope_name(&self) -> &str {
        match &self.scope {
            Scope::Everything => "Everything",
            Scope::Organization(membership) => &membership.organization.name,
            Scope::Nothing => "No organisation",
        }
    }

    /// The value `choose` takes to come back to the current scope.
    pub fn scope_choice(&self) -> String {
        match &self.scope {
            Scope::Organization(membership) => membership.organization.id.0.to_string(),
            Scope::Everything | Scope::Nothing => EVERYTHING.to_owned(),
        }
    }

    /// Whether the user may change machines in the current scope.
    pub fn can_operate(&self) -> bool {
        match &self.scope {
            Scope::Everything => true,
            Scope::Organization(membership) => membership.role.can_operate(),
            Scope::Nothing => false,
        }
    }

    /// The user's role in the organisation, if they are in it.
    pub fn role_in(&self, organization: OrganizationId) -> Option<Role> {
        self.memberships
            .iter()
            .find(|membership| membership.organization.id == organization)
            .map(|membership| membership.role)
    }

    /// Whether the user may add, change and remove the organisation's members.
    pub fn can_manage_members(&self, organization: OrganizationId) -> bool {
        self.admin
            || self
                .role_in(organization)
                .is_some_and(Role::can_manage_members)
    }

    /// The environments in the current scope, ordered by name.
    pub async fn visible(&self, database: &Database) -> Result<Vec<Visible>> {
        let environments = database.proxmox_environments().await?;
        Ok(match &self.scope {
            Scope::Everything => environments
                .into_iter()
                .map(|environment| Visible {
                    environment,
                    tag: None,
                })
                .collect(),
            Scope::Organization(membership) => {
                let grants = database.grants(membership.organization.id).await?;
                environments
                    .into_iter()
                    .filter_map(|environment| {
                        let grant = grants
                            .iter()
                            .find(|grant| grant.environment == environment.id)?;
                        Some(Visible {
                            environment,
                            tag: Some(grant.tag.clone()).filter(|tag| !tag.is_empty()),
                        })
                    })
                    .collect()
            }
            Scope::Nothing => Vec::new(),
        })
    }
}

/// Makes the user an admin if theirs is the only account: on a fresh
/// installation, whoever gets there first runs it.
pub async fn first_account_runs_it(database: &Database, user: &User) -> Result<()> {
    let accounts = database.accounts().await?;
    if accounts.len() == 1 && accounts[0].user == user.id {
        database.set_admin(user.id, true).await?;
    }
    Ok(())
}

/// Puts the user in the organisations that take everyone from `provider`.
/// A role they already have there is left as it is.
pub async fn join_through(cx: &Cx, user: &User, provider: &str) -> Result<()> {
    let state = state(cx);
    let database = state.auth.store();
    let already = database.memberships_of(user.id).await?;
    for organization in state.organizations_of(provider) {
        if !already
            .iter()
            .any(|membership| membership.organization.id == organization)
        {
            database
                .set_membership(user.id, organization, Role::Member)
                .await?;
        }
    }
    Ok(())
}

/// Brings the admins named in the config file (and by `LEMNOS_ADMIN_USERNAME`
/// and `LEMNOS_ADMIN_PASSWORD`) into being. An account that does not exist
/// yet is created with the password given; one that does keeps its own.
pub async fn sync_admins(
    auth: &Auth<Database>,
    mut configured: Vec<AdminConfig>,
) -> std::result::Result<(), Box<dyn Error>> {
    if let Some(from_environment) = AdminConfig::from_environment() {
        configured.push(from_environment);
    }
    let database = auth.store();
    for admin in configured {
        let username = Username::parse(&admin.username)
            .map_err(|error| format!("admin `{}`: {error}", admin.username))?;
        let user = match database.user_by_username(&username).await? {
            Some(user) => user,
            None => {
                let password = admin.password.as_ref().ok_or_else(|| {
                    format!(
                        "admin `{username}` has no account yet; give it a first password to create one"
                    )
                })?;
                // Checked first, so a weak password doesn't leave behind an
                // account nobody can sign in to.
                auth.password_policy()
                    .check(password.expose())
                    .map_err(|error| format!("admin `{username}`: {error}"))?;
                let user = auth.create_user(NewUser::new(username.clone())).await?;
                auth.set_password(user.id, password.expose()).await?;
                eprintln!("admin account `{username}` created");
                user
            }
        };
        database.set_admin(user.id, true).await?;
    }
    Ok(())
}

/// Creates the organisations named in the config file that do not exist
/// yet, and returns which sign-in providers lead into which organisation.
pub async fn sync_organizations(
    database: &Database,
    configured: Vec<OrganizationConfig>,
) -> std::result::Result<Vec<(String, OrganizationId)>, Box<dyn Error>> {
    let mut through = Vec::new();
    for entry in configured {
        let name = entry.name.trim();
        if name.is_empty() {
            return Err("an organization in the config file has no name".into());
        }
        let organization = match database.organization_by_name(name).await? {
            Some(known) => known,
            None => database.create_organization(name).await?,
        };
        for provider in entry.providers {
            through.push((provider, organization.id));
        }
    }
    Ok(through)
}
