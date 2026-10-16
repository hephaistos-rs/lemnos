//! Who owns the machines Lemnos created, the projects inside organisations,
//! and how many machines an organisation may create.
//!
//! A machine belongs to one of three owners:
//!
//! ```text
//!   person         a personal machine, in the environment set aside for them
//!   project        a group of people inside an organisation
//!   organisation   everyone in it
//! ```
//!
//! Only machines created through Lemnos have a record here. The rest are
//! still found by the tag their organisation was given (see
//! [`Grant`](crate::Grant)), and count as the organisation's own.

use std::time::{SystemTime, UNIX_EPOCH};

use lemnos_auth::UserId;
use sqlx::{Row, any::AnyRow};

use crate::{
    Database, OrganizationId, OrganizationStoreError, ProxmoxEnvironmentId, organizations,
};

type Result<T> = std::result::Result<T, OrganizationStoreError>;

/// The setting that names the environment personal machines go to.
const PERSONAL_ENVIRONMENT: &str = "personal_environment";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProjectId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub organization: OrganizationId,
    /// No two projects of one organisation share a name.
    pub name: String,
}

/// One of a project's people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectMember {
    pub user: UserId,
    pub username: String,
    pub name: String,
}

/// Whose a machine is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    Person(UserId),
    Project {
        organization: OrganizationId,
        project: ProjectId,
    },
    Organization(OrganizationId),
}

impl Owner {
    /// The organisation the machine counts against, if any.
    pub fn organization(self) -> Option<OrganizationId> {
        match self {
            Self::Person(_) => None,
            Self::Project { organization, .. } | Self::Organization(organization) => {
                Some(organization)
            }
        }
    }
}

/// A machine Lemnos created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineRecord {
    pub environment: ProxmoxEnvironmentId,
    /// The provider's ID for it; for Proxmox, the VM ID.
    pub machine: String,
    /// The name it was created with.
    pub name: String,
    pub owner: Owner,
    /// Who pressed the button. `None` once they are gone.
    pub created_by: Option<UserId>,
    /// The marker tag the guest was given (see `lemnos_core::marker`). The
    /// record applies to a guest only while the guest carries it. `None`
    /// for records from before markers: unverified until an admin confirms.
    pub marker: Option<String>,
}

fn id(value: u64) -> i64 {
    value as i64
}

fn store_error(error: sqlx::Error) -> OrganizationStoreError {
    organizations::store_error(error)
}

const MACHINE_COLUMNS: &str =
    "environment_id, machine, name, user_id, organization_id, project_id, created_by, marker";

fn record_from(row: &AnyRow) -> Result<MachineRecord> {
    let get = |index: usize| -> Result<Option<u64>> {
        Ok(row
            .try_get::<Option<i64>, _>(index)
            .map_err(store_error)?
            .map(|value| value as u64))
    };
    let user = get(3)?;
    let organization = get(4)?;
    let project = get(5)?;
    let owner = match (project, organization, user) {
        (Some(project), Some(organization), _) => Owner::Project {
            organization: OrganizationId(organization),
            project: ProjectId(project),
        },
        (None, Some(organization), _) => Owner::Organization(OrganizationId(organization)),
        (_, None, Some(user)) => Owner::Person(UserId(user)),
        _ => return Err(OrganizationStoreError::UnknownOwner),
    };
    Ok(MachineRecord {
        environment: ProxmoxEnvironmentId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
        machine: row.try_get(1).map_err(store_error)?,
        name: row.try_get(2).map_err(store_error)?,
        owner,
        created_by: get(6)?.map(UserId),
        marker: row.try_get(7).map_err(store_error)?,
    })
}

fn project_from(row: &AnyRow) -> Result<Project> {
    Ok(Project {
        id: ProjectId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
        organization: OrganizationId(row.try_get::<i64, _>(1).map_err(store_error)? as u64),
        name: row.try_get(2).map_err(store_error)?,
    })
}

impl Database {
    // --- projects ---

    pub async fn create_project(
        &self,
        organization: OrganizationId,
        name: &str,
    ) -> Result<Project> {
        // A random ID, for the same reason users get one.
        let project = Project {
            id: ProjectId(rand::random_range(1..=i64::MAX as u64)),
            organization,
            name: name.to_owned(),
        };
        self.query("INSERT INTO projects (id, organization_id, name) VALUES (?, ?, ?)")
            .bind(id(project.id.0))
            .bind(id(organization.0))
            .bind(project.name.clone())
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(project)
    }

    /// The organisation's projects, ordered by name.
    pub async fn projects(&self, organization: OrganizationId) -> Result<Vec<Project>> {
        self.query(
            "SELECT id, organization_id, name FROM projects
             WHERE organization_id = ? ORDER BY name",
        )
        .bind(id(organization.0))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(project_from)
        .collect()
    }

    pub async fn project(&self, project: ProjectId) -> Result<Option<Project>> {
        self.query("SELECT id, organization_id, name FROM projects WHERE id = ?")
            .bind(id(project.0))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .as_ref()
            .map(project_from)
            .transpose()
    }

    /// The projects the user is in, ordered by name.
    pub async fn projects_of(&self, user: UserId) -> Result<Vec<Project>> {
        self.query(
            "SELECT projects.id, projects.organization_id, projects.name
             FROM project_members
             JOIN projects ON projects.id = project_members.project_id
             WHERE project_members.user_id = ?
             ORDER BY projects.name",
        )
        .bind(id(user.0))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(project_from)
        .collect()
    }

    /// Forgets the project and who was in it. Its machines become the
    /// organisation's.
    pub async fn delete_project(&self, project: ProjectId) -> Result<()> {
        // Spelled out; see `delete_organization`.
        for statement in [
            "UPDATE machines SET project_id = NULL WHERE project_id = ?",
            "DELETE FROM project_members WHERE project_id = ?",
            "DELETE FROM projects WHERE id = ?",
        ] {
            self.query(statement)
                .bind(id(project.0))
                .execute(&self.pool)
                .await
                .map_err(store_error)?;
        }
        Ok(())
    }

    /// Puts the user in the project. Being in it already is not an error.
    pub async fn add_project_member(&self, project: ProjectId, user: UserId) -> Result<()> {
        let mut transaction = self.pool.begin().await.map_err(store_error)?;
        self.query("DELETE FROM project_members WHERE project_id = ? AND user_id = ?")
            .bind(id(project.0))
            .bind(id(user.0))
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
        self.query("INSERT INTO project_members (project_id, user_id) VALUES (?, ?)")
            .bind(id(project.0))
            .bind(id(user.0))
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
        transaction.commit().await.map_err(store_error)
    }

    pub async fn remove_project_member(&self, project: ProjectId, user: UserId) -> Result<()> {
        self.query("DELETE FROM project_members WHERE project_id = ? AND user_id = ?")
            .bind(id(project.0))
            .bind(id(user.0))
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    /// The people in the project, ordered by username.
    pub async fn project_members(&self, project: ProjectId) -> Result<Vec<ProjectMember>> {
        self.query(
            "SELECT users.id, users.username, users.name
             FROM project_members
             JOIN users ON users.id = project_members.user_id
             WHERE project_members.project_id = ?
             ORDER BY users.username",
        )
        .bind(id(project.0))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(ProjectMember {
                user: UserId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
                username: row.try_get(1).map_err(store_error)?,
                name: row.try_get(2).map_err(store_error)?,
            })
        })
        .collect()
    }

    // --- quotas ---

    /// How many machines the organisation's people may create, its projects'
    /// included.
    pub async fn machine_quota(&self, organization: OrganizationId) -> Result<u64> {
        let quota: Option<i64> = self
            .query("SELECT machine_quota FROM organizations WHERE id = ?")
            .bind(id(organization.0))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(|row| row.try_get(0))
            .transpose()
            .map_err(store_error)?;
        Ok(quota.map_or(0, |quota| quota.max(0) as u64))
    }

    pub async fn set_machine_quota(&self, organization: OrganizationId, quota: u64) -> Result<()> {
        self.query("UPDATE organizations SET machine_quota = ? WHERE id = ?")
            .bind(quota.min(i64::MAX as u64) as i64)
            .bind(id(organization.0))
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    // --- machines ---

    /// Remembers who owns a machine Lemnos just created.
    pub async fn record_machine(&self, record: &MachineRecord) -> Result<()> {
        let (user, organization, project) = match record.owner {
            Owner::Person(user) => (Some(user.0), None, None),
            Owner::Project {
                organization,
                project,
            } => (None, Some(organization.0), Some(project.0)),
            Owner::Organization(organization) => (None, Some(organization.0), None),
        };
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        self.query(&format!(
            "INSERT INTO machines ({MACHINE_COLUMNS}, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(id(record.environment.0))
        .bind(record.machine.clone())
        .bind(record.name.clone())
        .bind(user.map(id))
        .bind(organization.map(id))
        .bind(project.map(id))
        .bind(record.created_by.map(|user| id(user.0)))
        .bind(record.marker.clone())
        .bind(id(created_at))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// The user's personal machines.
    pub async fn personal_machines(&self, user: UserId) -> Result<Vec<MachineRecord>> {
        self.query(&format!(
            "SELECT {MACHINE_COLUMNS} FROM machines
             WHERE user_id = ? AND organization_id IS NULL
             ORDER BY name"
        ))
        .bind(id(user.0))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(record_from)
        .collect()
    }

    /// The machines the organisation and its projects own.
    pub async fn organization_machines(
        &self,
        organization: OrganizationId,
    ) -> Result<Vec<MachineRecord>> {
        self.query(&format!(
            "SELECT {MACHINE_COLUMNS} FROM machines WHERE organization_id = ? ORDER BY name"
        ))
        .bind(id(organization.0))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(record_from)
        .collect()
    }

    /// Every machine Lemnos created in the environment.
    pub async fn environment_machines(
        &self,
        environment: ProxmoxEnvironmentId,
    ) -> Result<Vec<MachineRecord>> {
        self.query(&format!(
            "SELECT {MACHINE_COLUMNS} FROM machines WHERE environment_id = ? ORDER BY name"
        ))
        .bind(id(environment.0))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(record_from)
        .collect()
    }

    /// Sets the marker of a record, once an admin confirmed which guest it
    /// is and Lemnos tagged that guest with it.
    pub async fn set_machine_marker(
        &self,
        environment: ProxmoxEnvironmentId,
        machine: &str,
        marker: &str,
    ) -> Result<()> {
        self.query("UPDATE machines SET marker = ? WHERE environment_id = ? AND machine = ?")
            .bind(marker.to_owned())
            .bind(id(environment.0))
            .bind(machine.to_owned())
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    /// Forgets a machine, for instance because it was deleted.
    pub async fn forget_machine(
        &self,
        environment: ProxmoxEnvironmentId,
        machine: &str,
    ) -> Result<()> {
        self.query("DELETE FROM machines WHERE environment_id = ? AND machine = ?")
            .bind(id(environment.0))
            .bind(machine.to_owned())
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    // --- settings ---

    /// The environment personal machines are created in, if an admin chose
    /// one and it still exists.
    pub async fn personal_environment(&self) -> Result<Option<ProxmoxEnvironmentId>> {
        let value: Option<String> = self
            .query("SELECT value FROM settings WHERE name = ?")
            .bind(PERSONAL_ENVIRONMENT.to_owned())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(|row| row.try_get(0))
            .transpose()
            .map_err(store_error)?;
        let Some(environment) = value
            .and_then(|value| value.parse().ok())
            .map(ProxmoxEnvironmentId)
        else {
            return Ok(None);
        };
        let exists = self
            .query("SELECT id FROM proxmox_environments WHERE id = ?")
            .bind(id(environment.0))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .is_some();
        Ok(exists.then_some(environment))
    }

    pub async fn set_personal_environment(
        &self,
        environment: Option<ProxmoxEnvironmentId>,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await.map_err(store_error)?;
        self.query("DELETE FROM settings WHERE name = ?")
            .bind(PERSONAL_ENVIRONMENT.to_owned())
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
        if let Some(environment) = environment {
            self.query("INSERT INTO settings (name, value) VALUES (?, ?)")
                .bind(PERSONAL_ENVIRONMENT.to_owned())
                .bind(environment.0.to_string())
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
        }
        transaction.commit().await.map_err(store_error)
    }
}
