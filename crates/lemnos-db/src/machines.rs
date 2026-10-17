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
use lemnos_core::marker;
use sqlx::{Row, any::AnyRow};

use crate::{
    Database, Dialect, OrganizationId, OrganizationStoreError, ProxmoxEnvironmentId, organizations,
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

/// Where a machine Lemnos creates is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MachineStatus {
    /// Reserved, and being copied. Counts against its owner.
    Pending,
    /// It exists. Counts against its owner.
    Active,
    /// Creating it failed. Doesn't count; kept so its owner hears why.
    Failed,
}

impl MachineStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "active" => Some(Self::Active),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// A machine Lemnos created, or is creating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineRecord {
    pub environment: ProxmoxEnvironmentId,
    /// The provider's ID for it; for Proxmox, the VM ID. Until it is active,
    /// `pending-` and its marker instead: the key the record goes by.
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
    pub status: MachineStatus,
    /// The VM ID it is (being) copied to, once one was chosen. The same as
    /// `machine` once active.
    pub vmid: Option<String>,
    /// The task copying it (a Proxmox UPID), once the copy started.
    pub task: Option<String>,
    /// The tags it is to carry once copied, its marker included.
    pub tags: Vec<String>,
    /// The description its copy is given, naming its marker. `None` for
    /// records from before Lemnos gave copies one.
    pub description: Option<String>,
    /// Why it failed, in words its owner can read.
    pub failure: Option<String>,
    /// The error behind that, for admins.
    pub failure_detail: Option<String>,
    /// Seconds since the Unix epoch.
    pub created_at: u64,
    pub updated_at: u64,
}

impl MachineRecord {
    /// A record of a machine that already exists, as `record_machine`
    /// takes it.
    pub fn active(
        environment: ProxmoxEnvironmentId,
        machine: impl Into<String>,
        name: impl Into<String>,
        owner: Owner,
        created_by: Option<UserId>,
        marker: Option<String>,
    ) -> Self {
        let machine = machine.into();
        Self {
            environment,
            vmid: Some(machine.clone()),
            machine,
            name: name.into(),
            owner,
            created_by,
            marker,
            status: MachineStatus::Active,
            task: None,
            tags: Vec::new(),
            description: None,
            failure: None,
            failure_detail: None,
            created_at: 0,
            updated_at: 0,
        }
    }
}

/// A machine to reserve room for, before anything is copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub environment: ProxmoxEnvironmentId,
    pub name: String,
    pub owner: Owner,
    pub created_by: Option<UserId>,
    /// The marker it will carry; see `lemnos_core::marker`.
    pub marker: String,
    /// Every tag it will carry, the marker included.
    pub tags: Vec<String>,
}

/// How many machines the owner may have, for [`Database::reserve_machine`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// As many as they like: admins.
    Unlimited,
    AtMost(u64),
    /// The organisation's quota, read in the same transaction.
    OrganizationQuota,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The key a record goes by until its machine exists.
fn pending_key(marker: &str) -> String {
    format!("pending-{marker}")
}

fn id(value: u64) -> i64 {
    value as i64
}

fn store_error(error: sqlx::Error) -> OrganizationStoreError {
    organizations::store_error(error)
}

const MACHINE_COLUMNS: &str = "environment_id, machine, name, user_id, organization_id, \
     project_id, created_by, marker, status, vmid, task, tags, failure, failure_detail, \
     created_at, updated_at, description";

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
    let text = |index: usize| -> Result<Option<String>> { row.try_get(index).map_err(store_error) };
    let status: String = row.try_get(8).map_err(store_error)?;
    Ok(MachineRecord {
        environment: ProxmoxEnvironmentId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
        machine: row.try_get(1).map_err(store_error)?,
        name: row.try_get(2).map_err(store_error)?,
        owner,
        created_by: get(6)?.map(UserId),
        marker: text(7)?,
        status: MachineStatus::parse(&status).ok_or(OrganizationStoreError::UnknownStatus)?,
        vmid: text(9)?,
        task: text(10)?,
        tags: text(11)?
            .map(|tags| {
                tags.split(';')
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        failure: text(12)?,
        failure_detail: text(13)?,
        created_at: get(14)?.unwrap_or(0),
        updated_at: get(15)?.unwrap_or(0),
        description: text(16)?,
    })
}

fn owner_columns(owner: Owner) -> (Option<u64>, Option<u64>, Option<u64>) {
    match owner {
        Owner::Person(user) => (Some(user.0), None, None),
        Owner::Project {
            organization,
            project,
        } => (None, Some(organization.0), Some(project.0)),
        Owner::Organization(organization) => (None, Some(organization.0), None),
    }
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

    /// Remembers who owns a machine that exists already, Lemnos' or not.
    /// New machines go through [`reserve_machine`](Self::reserve_machine).
    pub async fn record_machine(&self, record: &MachineRecord) -> Result<()> {
        let created_at = now();
        self.insert_machine(
            &self.pool,
            &MachineRecord {
                created_at,
                updated_at: created_at,
                ..record.clone()
            },
        )
        .await
    }

    async fn insert_machine<'c, E>(&self, executor: E, record: &MachineRecord) -> Result<()>
    where
        E: sqlx::Executor<'c, Database = sqlx::Any>,
    {
        let (user, organization, project) = owner_columns(record.owner);
        let tags = (!record.tags.is_empty()).then(|| record.tags.join(";"));
        self.query(&format!(
            "INSERT INTO machines ({MACHINE_COLUMNS})
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(id(record.environment.0))
        .bind(record.machine.clone())
        .bind(record.name.clone())
        .bind(user.map(id))
        .bind(organization.map(id))
        .bind(project.map(id))
        .bind(record.created_by.map(|user| id(user.0)))
        .bind(record.marker.clone())
        .bind(record.status.as_str())
        .bind(record.vmid.clone())
        .bind(record.task.clone())
        .bind(tags)
        .bind(record.failure.clone())
        .bind(record.failure_detail.clone())
        .bind(id(record.created_at))
        .bind(id(record.updated_at))
        .bind(record.description.clone())
        .execute(executor)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// Takes a slot for a new machine, before anything is copied: in one
    /// transaction, it locks the owner, counts their pending and active
    /// machines, and adds a pending record if there is room. Two requests
    /// for the last slot can't both get it. Failed records don't count.
    ///
    /// The lock: PostgreSQL and MariaDB lock the owner's row with `SELECT
    /// ... FOR UPDATE`. SQLite has no row locks, so the transaction starts
    /// with `BEGIN IMMEDIATE`, which takes the database's write lock up
    /// front; a second one waits for the first to commit.
    pub async fn reserve_machine(
        &self,
        reservation: &Reservation,
        limit: Limit,
    ) -> Result<MachineRecord> {
        let mut transaction = match self.dialect {
            Dialect::Sqlite => self.pool.begin_with("BEGIN IMMEDIATE").await,
            Dialect::Postgres | Dialect::MySql => self.pool.begin().await,
        }
        .map_err(store_error)?;

        let (lock, owner_id, counted) = match reservation.owner {
            Owner::Person(user) => (
                "SELECT id FROM users WHERE id = ?",
                user.0,
                "SELECT COUNT(*) FROM machines
                 WHERE user_id = ? AND organization_id IS NULL
                   AND status IN ('pending', 'active')",
            ),
            Owner::Organization(organization) | Owner::Project { organization, .. } => (
                "SELECT id FROM organizations WHERE id = ?",
                organization.0,
                "SELECT COUNT(*) FROM machines
                 WHERE organization_id = ? AND status IN ('pending', 'active')",
            ),
        };
        if self.dialect != Dialect::Sqlite {
            self.query(&format!("{lock} FOR UPDATE"))
                .bind(id(owner_id))
                .fetch_optional(&mut *transaction)
                .await
                .map_err(store_error)?;
        }
        let limit = match limit {
            Limit::Unlimited => None,
            Limit::AtMost(limit) => Some(limit),
            Limit::OrganizationQuota => {
                let quota: Option<i64> = self
                    .query("SELECT machine_quota FROM organizations WHERE id = ?")
                    .bind(id(owner_id))
                    .fetch_optional(&mut *transaction)
                    .await
                    .map_err(store_error)?
                    .map(|row| row.try_get(0))
                    .transpose()
                    .map_err(store_error)?;
                Some(quota.map_or(0, |quota| quota.max(0) as u64))
            }
        };
        if let Some(limit) = limit {
            let used: i64 = self
                .query(counted)
                .bind(id(owner_id))
                .fetch_one(&mut *transaction)
                .await
                .map_err(store_error)?
                .try_get(0)
                .map_err(store_error)?;
            if used.max(0) as u64 >= limit {
                return Err(OrganizationStoreError::QuotaReached);
            }
        }

        let created_at = now();
        let record = MachineRecord {
            environment: reservation.environment,
            machine: pending_key(&reservation.marker),
            name: reservation.name.clone(),
            owner: reservation.owner,
            created_by: reservation.created_by,
            marker: Some(reservation.marker.clone()),
            status: MachineStatus::Pending,
            vmid: None,
            task: None,
            tags: reservation.tags.clone(),
            description: Some(marker::description(&reservation.marker)),
            failure: None,
            failure_detail: None,
            created_at,
            updated_at: created_at,
        };
        self.insert_machine(&mut *transaction, &record).await?;
        transaction.commit().await.map_err(store_error)?;
        Ok(record)
    }

    /// Notes the VM ID a pending machine is being copied to, and once the
    /// copy started, its task. Called before each try, so a guest that
    /// turns up at the ID can be traced back even if Lemnos stops halfway.
    pub async fn machine_started(
        &self,
        environment: ProxmoxEnvironmentId,
        key: &str,
        vmid: &str,
        task: Option<&str>,
    ) -> Result<()> {
        self.query(
            "UPDATE machines SET vmid = ?, task = ?, updated_at = ?
             WHERE environment_id = ? AND machine = ? AND status = 'pending'",
        )
        .bind(vmid.to_owned())
        .bind(task.map(str::to_owned))
        .bind(id(now()))
        .bind(id(environment.0))
        .bind(key.to_owned())
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// Marks a pending machine as existing, under its VM ID. Any other
    /// active record of that ID is of a guest deleted behind Lemnos' back
    /// (the ID was free to copy to), so it goes. Returns the record, or
    /// `None` if it wasn't pending (any more).
    pub async fn activate_machine(
        &self,
        environment: ProxmoxEnvironmentId,
        key: &str,
    ) -> Result<Option<MachineRecord>> {
        let mut transaction = self.pool.begin().await.map_err(store_error)?;
        let Some(record) = self
            .query(&format!(
                "SELECT {MACHINE_COLUMNS} FROM machines
                 WHERE environment_id = ? AND machine = ? AND status = 'pending'"
            ))
            .bind(id(environment.0))
            .bind(key.to_owned())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(store_error)?
            .as_ref()
            .map(record_from)
            .transpose()?
        else {
            return Ok(None);
        };
        let Some(vmid) = record.vmid.clone() else {
            return Ok(None);
        };
        self.query(
            "DELETE FROM machines WHERE environment_id = ? AND machine = ? AND status = 'active'",
        )
        .bind(id(environment.0))
        .bind(vmid.clone())
        .execute(&mut *transaction)
        .await
        .map_err(store_error)?;
        let updated_at = now();
        self.query(
            "UPDATE machines SET machine = ?, status = 'active', updated_at = ?
             WHERE environment_id = ? AND machine = ?",
        )
        .bind(vmid.clone())
        .bind(id(updated_at))
        .bind(id(environment.0))
        .bind(key.to_owned())
        .execute(&mut *transaction)
        .await
        .map_err(store_error)?;
        transaction.commit().await.map_err(store_error)?;
        Ok(Some(MachineRecord {
            machine: vmid,
            status: MachineStatus::Active,
            updated_at,
            ..record
        }))
    }

    /// Marks a pending machine as failed, which frees its slot. `reason` is
    /// for its owner; `detail` for admins. Returns whether it was pending.
    pub async fn fail_machine(
        &self,
        environment: ProxmoxEnvironmentId,
        key: &str,
        reason: &str,
        detail: &str,
    ) -> Result<bool> {
        let done = self
            .query(
                "UPDATE machines SET status = 'failed', failure = ?, failure_detail = ?,
                     updated_at = ?
                 WHERE environment_id = ? AND machine = ? AND status = 'pending'",
            )
            .bind(reason.to_owned())
            .bind(detail.to_owned())
            .bind(id(now()))
            .bind(id(environment.0))
            .bind(key.to_owned())
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(done.rows_affected() > 0)
    }

    /// One record, by the key it goes by (see [`MachineRecord::machine`]).
    pub async fn machine_record(
        &self,
        environment: ProxmoxEnvironmentId,
        key: &str,
    ) -> Result<Option<MachineRecord>> {
        self.query(&format!(
            "SELECT {MACHINE_COLUMNS} FROM machines WHERE environment_id = ? AND machine = ?"
        ))
        .bind(id(environment.0))
        .bind(key.to_owned())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .as_ref()
        .map(record_from)
        .transpose()
    }

    /// The record carrying `marker`, whatever key it goes by now: a pending
    /// record is renamed to its VM ID once active.
    pub async fn machine_by_marker(
        &self,
        environment: ProxmoxEnvironmentId,
        marker: &str,
    ) -> Result<Option<MachineRecord>> {
        self.query(&format!(
            "SELECT {MACHINE_COLUMNS} FROM machines WHERE environment_id = ? AND marker = ?"
        ))
        .bind(id(environment.0))
        .bind(marker.to_owned())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .as_ref()
        .map(record_from)
        .transpose()
    }

    /// Every machine still being created, in every environment.
    pub async fn pending_machines(&self) -> Result<Vec<MachineRecord>> {
        self.query(&format!(
            "SELECT {MACHINE_COLUMNS} FROM machines WHERE status = 'pending' ORDER BY created_at"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(record_from)
        .collect()
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

    /// Sets the marker of an active record, once an admin confirmed which
    /// guest it is and Lemnos tagged that guest with it.
    pub async fn set_machine_marker(
        &self,
        environment: ProxmoxEnvironmentId,
        machine: &str,
        marker: &str,
    ) -> Result<()> {
        self.query(
            "UPDATE machines SET marker = ?, updated_at = ?
             WHERE environment_id = ? AND machine = ? AND status = 'active'",
        )
        .bind(marker.to_owned())
        .bind(id(now()))
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
