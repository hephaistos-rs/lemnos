// Who the signed-in user is and what they may see. Every console page asks
// here first; nothing else decides what a person is allowed to do.
//
//   admin         runs the installation: environments, organisations, users
//   owner         decides who is in an organisation and its projects
//   member        uses the organisation's machines
//   viewer        sees them
//
// A machine belongs to a person, a project or an organisation (see
// `lemnos_db::Owner`). Everyone may have one personal machine; admins as
// many as they like. Organisations and their projects share a quota an
// admin sets.
//
// A person looks at their own machines, one organisation at a time, or (as
// an admin) everything. Which one is kept in a cookie and checked on every
// request.

use std::error::Error;

use lemnos_auth::{Auth, NewUser, Store, User, Username};
use lemnos_db::{
    Database, MachineRecord, Membership, OrganizationId, Owner, Project, ProxmoxEnvironment, Role,
};
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

/// Cookie holding what is being looked at: an organisation's ID, `personal`
/// or `all`.
const COOKIE: &str = "organization";
const EVERYTHING: &str = "all";
const PERSONAL: &str = "personal";

/// How many personal machines anyone but an admin may have.
pub const PERSONAL_LIMIT: usize = 1;

/// What the user is looking at.
pub enum Scope {
    /// Every environment and every machine. Admins only.
    Everything,
    /// The user's own machines.
    Personal,
    /// One of the user's organisations.
    Organization(Membership),
}

pub struct Access {
    pub user: User,
    pub admin: bool,
    pub memberships: Vec<Membership>,
    /// The projects the user is in, across organisations.
    pub projects: Vec<Project>,
    pub scope: Scope,
}

/// Which of an environment's machines are in view.
pub enum Selector {
    Every,
    /// Those carrying the tag, and those Lemnos created for the owner.
    Tagged(String),
    /// Only those Lemnos created for the owner.
    Recorded,
}

/// An environment the user may see, which of its machines, and who owns the
/// ones Lemnos created there.
pub struct Visible {
    pub environment: ProxmoxEnvironment,
    pub selector: Selector,
    pub records: Vec<MachineRecord>,
}

/// Whether the record is about this guest: the same ID, and the guest still
/// carries the record's marker tag. The ID alone is never enough, since
/// Proxmox gives a freed ID to the next guest made, whoever's it is. A record
/// without a marker (from before there were any) applies to no guest until
/// an admin confirms it; one whose marker was taken off in Proxmox stops
/// applying. Either way the machine drops out of its owner's view and is
/// shown to nobody as theirs.
pub fn applies_to(record: &MachineRecord, guest: &Guest) -> bool {
    record.machine == guest.id.vmid.to_string()
        && record
            .marker
            .as_ref()
            .is_some_and(|marker| guest.tags.contains(marker))
}

impl Visible {
    /// Who owns the guest, if Lemnos created it.
    pub fn record(&self, guest: &Guest) -> Option<&MachineRecord> {
        self.records.iter().find(|record| applies_to(record, guest))
    }

    pub fn shows(&self, guest: &Guest) -> bool {
        match &self.selector {
            Selector::Every => true,
            Selector::Tagged(tag) => {
                guest.tags.iter().any(|carried| carried == tag) || self.record(guest).is_some()
            }
            Selector::Recorded => self.record(guest).is_some(),
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
    let projects = database.projects_of(user.id).await?;

    let chosen = cookies(cx).get(COOKIE);
    let chosen = chosen.as_ref().map(|cookie| cookie.value().to_owned());
    let chosen_organization = chosen
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|id| {
            memberships
                .iter()
                .find(|membership| membership.organization.id.0 == id)
        });
    let scope = match (chosen.as_deref(), chosen_organization) {
        (_, Some(membership)) => Scope::Organization(membership.clone()),
        (Some(PERSONAL), None) => Scope::Personal,
        (Some(EVERYTHING), None) if admin => Scope::Everything,
        // With no valid choice, an admin sees everything, anyone else their
        // first organisation, and someone in none their own machines.
        _ if admin => Scope::Everything,
        _ => match memberships.first() {
            Some(membership) => Scope::Organization(membership.clone()),
            None => Scope::Personal,
        },
    };
    Ok(Access {
        user,
        admin,
        memberships,
        projects,
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

/// Remembers what to look at. `choice` is an organisation's ID, `personal`
/// or `all`; whether the user may look at it is checked when it is read back.
pub fn choose(cx: &Cx, choice: &str) {
    let jar = cookies(cx)
        .default_path("/")
        .default_http_only(true)
        .default_same_site(SameSite::Lax)
        .default_max_age(Duration::days(365));
    if choice == EVERYTHING || choice == PERSONAL || choice.parse::<u64>().is_ok() {
        jar.add((COOKIE, choice.to_owned()));
    }
}

/// The value `choose` takes to look at the owner's machines.
pub fn choice_for(owner: Owner) -> String {
    match owner.organization() {
        Some(organization) => organization.0.to_string(),
        None => PERSONAL.to_owned(),
    }
}

impl Access {
    /// What to call the scope in the bar.
    pub fn scope_name(&self) -> &str {
        match &self.scope {
            Scope::Everything => "Everything",
            Scope::Personal => "Personal",
            Scope::Organization(membership) => &membership.organization.name,
        }
    }

    /// The value `choose` takes to come back to the current scope.
    pub fn scope_choice(&self) -> String {
        match &self.scope {
            Scope::Organization(membership) => membership.organization.id.0.to_string(),
            Scope::Personal => PERSONAL.to_owned(),
            Scope::Everything => EVERYTHING.to_owned(),
        }
    }

    /// Everything the user can switch the scope to, as (value for
    /// `choose`, name).
    pub fn scopes(&self) -> Vec<(String, String)> {
        let mut scopes = Vec::new();
        if self.admin {
            scopes.push((EVERYTHING.to_owned(), "Everything".to_owned()));
        }
        scopes.push((PERSONAL.to_owned(), "Personal".to_owned()));
        for membership in &self.memberships {
            scopes.push((
                membership.organization.id.0.to_string(),
                membership.organization.name.clone(),
            ));
        }
        scopes
    }

    fn in_project(&self, project: lemnos_db::ProjectId) -> bool {
        self.projects.iter().any(|known| known.id == project)
    }

    /// Whether the user may power the guest on and off. `visible` is the
    /// environment it was found in.
    pub fn can_operate(&self, visible: &Visible, guest: &Guest) -> bool {
        if self.admin {
            return true;
        }
        let owner = visible.record(guest).map(|record| record.owner);
        match &self.scope {
            Scope::Everything => false,
            Scope::Personal => owner == Some(Owner::Person(self.user.id)),
            Scope::Organization(membership) => match owner {
                // A project's machines are for its people, and for the
                // organisation's owners.
                Some(Owner::Project { project, .. }) => {
                    membership.role == Role::Owner
                        || (membership.role.can_operate() && self.in_project(project))
                }
                _ => membership.role.can_operate(),
            },
        }
    }

    /// Whether the user may delete the guest. Only machines Lemnos created
    /// can be deleted here.
    pub fn can_delete(&self, visible: &Visible, guest: &Guest) -> bool {
        let Some(record) = visible.record(guest) else {
            return false;
        };
        if self.admin {
            return true;
        }
        match record.owner {
            Owner::Person(user) => user == self.user.id,
            Owner::Organization(organization) => self.role_in(organization) == Some(Role::Owner),
            Owner::Project {
                organization,
                project,
            } => {
                self.role_in(organization) == Some(Role::Owner)
                    || (self.in_project(project)
                        && self.role_in(organization).is_some_and(Role::can_operate))
            }
        }
    }

    /// Whether the user may create machines for the owner, leaving aside
    /// how many there already are.
    pub fn may_create_for(&self, owner: Owner) -> bool {
        if self.admin {
            return true;
        }
        match owner {
            Owner::Person(user) => user == self.user.id,
            Owner::Organization(organization) => {
                self.role_in(organization).is_some_and(Role::can_operate)
            }
            Owner::Project {
                organization,
                project,
            } => match self.role_in(organization) {
                Some(Role::Owner) => true,
                Some(role) => role.can_operate() && self.in_project(project),
                None => false,
            },
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
        let mut visible = Vec::new();
        match &self.scope {
            Scope::Everything => {
                for environment in environments {
                    visible.push(Visible {
                        records: database.environment_machines(environment.id).await?,
                        environment,
                        selector: Selector::Every,
                    });
                }
            }
            // Wherever the user's own machines are: usually only the
            // environment for personal machines, but an admin can put theirs
            // anywhere.
            Scope::Personal => {
                let records = database.personal_machines(self.user.id).await?;
                for environment in environments {
                    let here: Vec<_> = records
                        .iter()
                        .filter(|record| record.environment == environment.id)
                        .cloned()
                        .collect();
                    if !here.is_empty() {
                        visible.push(Visible {
                            environment,
                            selector: Selector::Recorded,
                            records: here,
                        });
                    }
                }
            }
            Scope::Organization(membership) => {
                let organization = membership.organization.id;
                let grants = database.grants(organization).await?;
                let records = database.organization_machines(organization).await?;
                for environment in environments {
                    let Some(grant) = grants
                        .iter()
                        .find(|grant| grant.environment == environment.id)
                    else {
                        continue;
                    };
                    visible.push(Visible {
                        selector: if grant.tag.is_empty() {
                            Selector::Every
                        } else {
                            Selector::Tagged(grant.tag.clone())
                        },
                        records: records
                            .iter()
                            .filter(|record| record.environment == environment.id)
                            .cloned()
                            .collect(),
                        environment,
                    });
                }
            }
        }
        Ok(visible)
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
    // Easy to miss when a local config file predates `[[admin]]`.
    if configured.is_empty() {
        let any_admin = database
            .accounts()
            .await?
            .iter()
            .any(|account| account.admin);
        if !any_admin {
            eprintln!(
                "no admin is configured and none exists yet; the first account created becomes one"
            );
        }
    }
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

/// The owner of a new machine, as it travels in forms and links:
/// `personal`, `organization-{id}` or `project-{id}`.
pub fn owner_value(owner: Owner) -> String {
    match owner {
        Owner::Person(_) => PERSONAL.to_owned(),
        Owner::Organization(organization) => format!("organization-{}", organization.0),
        Owner::Project { project, .. } => format!("project-{}", project.0),
    }
}

/// Reads [`owner_value`] back. A project is looked up to find its
/// organisation; one that does not exist gives `None`.
pub async fn parse_owner(database: &Database, user: &User, value: &str) -> Result<Option<Owner>> {
    if value == PERSONAL {
        return Ok(Some(Owner::Person(user.id)));
    }
    if let Some(id) = value
        .strip_prefix("organization-")
        .and_then(|id| id.parse().ok())
    {
        let organization = database.organization(OrganizationId(id)).await?;
        return Ok(organization.map(|organization| Owner::Organization(organization.id)));
    }
    if let Some(id) = value
        .strip_prefix("project-")
        .and_then(|id| id.parse().ok())
    {
        let project = database.project(lemnos_db::ProjectId(id)).await?;
        return Ok(project.map(|project| Owner::Project {
            organization: project.organization,
            project: project.id,
        }));
    }
    Ok(None)
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

#[cfg(test)]
mod tests {
    use lemnos_auth::UserId;
    use lemnos_db::ProxmoxEnvironmentId;
    use lemnos_proxmox::guests::{GuestId, GuestKind, GuestStatus, VmId};

    use super::*;

    const MARKER: &str = "lemnos-3f9a0c7e21bd";

    fn guest(vmid: u32, tags: &[&str]) -> Guest {
        Guest {
            id: GuestId {
                node: "pve".into(),
                vmid: VmId::try_from(i128::from(vmid)).unwrap(),
                kind: GuestKind::Qemu,
            },
            name: Some("scratch".into()),
            status: GuestStatus::Stopped,
            template: false,
            cpu: None,
            max_cpu: None,
            mem: None,
            max_mem: None,
            uptime: None,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        }
    }

    fn record(vmid: &str, marker: Option<&str>) -> MachineRecord {
        MachineRecord::active(
            ProxmoxEnvironmentId(1),
            vmid,
            "scratch",
            Owner::Person(UserId(7)),
            Some(UserId(7)),
            marker.map(str::to_owned),
        )
    }

    #[test]
    fn the_same_id_and_marker_match() {
        assert!(applies_to(
            &record("100", Some(MARKER)),
            &guest(100, &["personal-jo", MARKER])
        ));
    }

    #[test]
    fn the_same_id_alone_does_not() {
        // The ID was handed to someone else's new guest.
        assert!(!applies_to(
            &record("100", Some(MARKER)),
            &guest(100, &["team-b"])
        ));
        // Another marker: another machine of Lemnos's.
        assert!(!applies_to(
            &record("100", Some(MARKER)),
            &guest(100, &["lemnos-000000000000"])
        ));
        // A record from before markers.
        assert!(!applies_to(&record("100", None), &guest(100, &[MARKER])));
    }

    #[test]
    fn the_marker_on_another_id_does_not() {
        assert!(!applies_to(
            &record("100", Some(MARKER)),
            &guest(101, &[MARKER])
        ));
    }
}
