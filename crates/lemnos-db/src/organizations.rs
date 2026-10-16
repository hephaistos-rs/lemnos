//! Who may see what: organisations, the people in them, the environments
//! each one was given, and the admins of the whole installation.
//!
//! An organisation is given an environment together with a tag. Its people
//! see the machines in that environment that carry the tag; an empty tag
//! means every machine there. This crate only keeps those facts; applying
//! them is up to whoever lists the machines.

use lemnos_auth::UserId;
use sqlx::Row;

use crate::{Database, ProxmoxEnvironmentId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OrganizationId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Organization {
    pub id: OrganizationId,
    /// What people call it. No two organisations share a name.
    pub name: String,
}

/// What a person may do in an organisation. Each role includes the ones
/// after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Also decides who else is in the organisation.
    Owner,
    /// Uses the organisation's machines.
    Member,
    /// Sees them, and changes nothing.
    Viewer,
}

impl Role {
    pub const ALL: [Self; 3] = [Self::Owner, Self::Member, Self::Viewer];

    /// The word stored in the database and sent in forms.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
            Self::Viewer => "viewer",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.as_str() == word)
    }

    /// Whether the role may change a machine, such as powering it off.
    pub fn can_operate(self) -> bool {
        matches!(self, Self::Owner | Self::Member)
    }

    /// Whether the role may add, change and remove members.
    pub fn can_manage_members(self) -> bool {
        self == Self::Owner
    }
}

/// One of a person's organisations, and their role in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Membership {
    pub organization: Organization,
    pub role: Role,
}

/// One of an organisation's people.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub user: UserId,
    pub username: String,
    pub name: String,
    pub role: Role,
}

/// An environment an organisation may use, and the tag that marks its
/// machines there. An empty tag means all of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub environment: ProxmoxEnvironmentId,
    pub tag: String,
}

/// An account, as the list of all of them shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub user: UserId,
    pub username: String,
    pub name: String,
    pub admin: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum OrganizationStoreError {
    #[error("another organisation already has that name")]
    NameTaken,
    #[error("the database holds an unknown role")]
    UnknownRole,
    #[error("the database holds a machine without an owner")]
    UnknownOwner,
    #[error("the database holds a machine in an unknown state")]
    UnknownStatus,
    /// [`Database::reserve_machine`](crate::Database::reserve_machine)
    /// found no room left.
    #[error("there is no room for another machine")]
    QuotaReached,
    #[error("the database failed")]
    Backend(#[source] sqlx::Error),
}

type Result<T> = std::result::Result<T, OrganizationStoreError>;

pub(crate) fn store_error(error: sqlx::Error) -> OrganizationStoreError {
    match error.as_database_error() {
        Some(database_error) if database_error.is_unique_violation() => {
            OrganizationStoreError::NameTaken
        }
        _ => OrganizationStoreError::Backend(error),
    }
}

fn id(organization: OrganizationId) -> i64 {
    organization.0 as i64
}

fn user_id(user: UserId) -> i64 {
    user.0 as i64
}

fn role_from(word: &str) -> Result<Role> {
    Role::parse(word).ok_or(OrganizationStoreError::UnknownRole)
}

impl Database {
    pub async fn create_organization(&self, name: &str) -> Result<Organization> {
        // A random ID, for the same reason users get one.
        let organization = Organization {
            id: OrganizationId(rand::random_range(1..=i64::MAX as u64)),
            name: name.to_owned(),
        };
        self.query("INSERT INTO organizations (id, name) VALUES (?, ?)")
            .bind(id(organization.id))
            .bind(organization.name.clone())
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(organization)
    }

    /// Every organisation, ordered by name.
    pub async fn organizations(&self) -> Result<Vec<Organization>> {
        self.query("SELECT id, name FROM organizations ORDER BY name")
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .iter()
            .map(|row| {
                Ok(Organization {
                    id: OrganizationId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
                    name: row.try_get(1).map_err(store_error)?,
                })
            })
            .collect()
    }

    pub async fn organization(&self, organization: OrganizationId) -> Result<Option<Organization>> {
        Ok(self
            .organizations()
            .await?
            .into_iter()
            .find(|known| known.id == organization))
    }

    pub async fn organization_by_name(&self, name: &str) -> Result<Option<Organization>> {
        Ok(self
            .organizations()
            .await?
            .into_iter()
            .find(|known| known.name == name))
    }

    /// Forgets the organisation, who was in it, its projects and what it was
    /// given, and who owns the machines it created. Accounts, environments
    /// and the machines themselves are left alone.
    pub async fn delete_organization(&self, organization: OrganizationId) -> Result<()> {
        // Spelled out instead of left to `ON DELETE CASCADE`: SQLite only
        // follows that when asked to on every connection.
        for statement in [
            "DELETE FROM machines WHERE organization_id = ?",
            "DELETE FROM project_members WHERE project_id IN
               (SELECT id FROM projects WHERE organization_id = ?)",
            "DELETE FROM projects WHERE organization_id = ?",
            "DELETE FROM memberships WHERE organization_id = ?",
            "DELETE FROM organization_environments WHERE organization_id = ?",
            "DELETE FROM organizations WHERE id = ?",
        ] {
            self.query(statement)
                .bind(id(organization))
                .execute(&self.pool)
                .await
                .map_err(store_error)?;
        }
        Ok(())
    }

    /// Puts the user in the organisation with this role, or changes the
    /// role they have there.
    pub async fn set_membership(
        &self,
        user: UserId,
        organization: OrganizationId,
        role: Role,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await.map_err(store_error)?;
        self.query("DELETE FROM memberships WHERE user_id = ? AND organization_id = ?")
            .bind(user_id(user))
            .bind(id(organization))
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
        self.query("INSERT INTO memberships (user_id, organization_id, role) VALUES (?, ?, ?)")
            .bind(user_id(user))
            .bind(id(organization))
            .bind(role.as_str().to_owned())
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
        transaction.commit().await.map_err(store_error)
    }

    /// Takes the user out of the organisation and out of its projects.
    pub async fn remove_membership(
        &self,
        user: UserId,
        organization: OrganizationId,
    ) -> Result<()> {
        for statement in [
            "DELETE FROM project_members WHERE user_id = ? AND project_id IN
               (SELECT id FROM projects WHERE organization_id = ?)",
            "DELETE FROM memberships WHERE user_id = ? AND organization_id = ?",
        ] {
            self.query(statement)
                .bind(user_id(user))
                .bind(id(organization))
                .execute(&self.pool)
                .await
                .map_err(store_error)?;
        }
        Ok(())
    }

    /// The organisations the user is in, ordered by name.
    pub async fn memberships_of(&self, user: UserId) -> Result<Vec<Membership>> {
        self.query(
            "SELECT organizations.id, organizations.name, memberships.role
             FROM memberships
             JOIN organizations ON organizations.id = memberships.organization_id
             WHERE memberships.user_id = ?
             ORDER BY organizations.name",
        )
        .bind(user_id(user))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            let role: String = row.try_get(2).map_err(store_error)?;
            Ok(Membership {
                organization: Organization {
                    id: OrganizationId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
                    name: row.try_get(1).map_err(store_error)?,
                },
                role: role_from(&role)?,
            })
        })
        .collect()
    }

    /// The people in the organisation, ordered by username.
    pub async fn members(&self, organization: OrganizationId) -> Result<Vec<Member>> {
        self.query(
            "SELECT users.id, users.username, users.name, memberships.role
             FROM memberships
             JOIN users ON users.id = memberships.user_id
             WHERE memberships.organization_id = ?
             ORDER BY users.username",
        )
        .bind(id(organization))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            let role: String = row.try_get(3).map_err(store_error)?;
            Ok(Member {
                user: UserId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
                username: row.try_get(1).map_err(store_error)?,
                name: row.try_get(2).map_err(store_error)?,
                role: role_from(&role)?,
            })
        })
        .collect()
    }

    /// Lets the organisation use the environment, or changes the tag that
    /// marks its machines there.
    pub async fn grant_environment(
        &self,
        organization: OrganizationId,
        environment: ProxmoxEnvironmentId,
        tag: &str,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await.map_err(store_error)?;
        self.query(
            "DELETE FROM organization_environments
             WHERE organization_id = ? AND environment_id = ?",
        )
        .bind(id(organization))
        .bind(environment.0 as i64)
        .execute(&mut *transaction)
        .await
        .map_err(store_error)?;
        self.query(
            "INSERT INTO organization_environments (organization_id, environment_id, tag)
             VALUES (?, ?, ?)",
        )
        .bind(id(organization))
        .bind(environment.0 as i64)
        .bind(tag.to_owned())
        .execute(&mut *transaction)
        .await
        .map_err(store_error)?;
        transaction.commit().await.map_err(store_error)
    }

    pub async fn revoke_environment(
        &self,
        organization: OrganizationId,
        environment: ProxmoxEnvironmentId,
    ) -> Result<()> {
        self.query(
            "DELETE FROM organization_environments
             WHERE organization_id = ? AND environment_id = ?",
        )
        .bind(id(organization))
        .bind(environment.0 as i64)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// What the organisation was given. Environments that have since been
    /// removed are left out.
    pub async fn grants(&self, organization: OrganizationId) -> Result<Vec<Grant>> {
        self.query(
            "SELECT organization_environments.environment_id, organization_environments.tag
             FROM organization_environments
             JOIN proxmox_environments
               ON proxmox_environments.id = organization_environments.environment_id
             WHERE organization_environments.organization_id = ?
             ORDER BY proxmox_environments.name",
        )
        .bind(id(organization))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(Grant {
                environment: ProxmoxEnvironmentId(
                    row.try_get::<i64, _>(0).map_err(store_error)? as u64
                ),
                tag: row.try_get(1).map_err(store_error)?,
            })
        })
        .collect()
    }

    pub async fn is_admin(&self, user: UserId) -> Result<bool> {
        Ok(self
            .query("SELECT user_id FROM admins WHERE user_id = ?")
            .bind(user_id(user))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .is_some())
    }

    /// Makes the user an admin of the whole installation, or an ordinary
    /// user again.
    pub async fn set_admin(&self, user: UserId, admin: bool) -> Result<()> {
        if admin == self.is_admin(user).await? {
            return Ok(());
        }
        self.query(if admin {
            "INSERT INTO admins (user_id) VALUES (?)"
        } else {
            "DELETE FROM admins WHERE user_id = ?"
        })
        .bind(user_id(user))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// Every account, ordered by username.
    pub async fn accounts(&self) -> Result<Vec<Account>> {
        self.query(
            "SELECT users.id, users.username, users.name, admins.user_id
             FROM users
             LEFT JOIN admins ON admins.user_id = users.id
             ORDER BY users.username",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            Ok(Account {
                user: UserId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
                username: row.try_get(1).map_err(store_error)?,
                name: row.try_get(2).map_err(store_error)?,
                admin: row
                    .try_get::<Option<i64>, _>(3)
                    .map_err(store_error)?
                    .is_some(),
            })
        })
        .collect()
    }
}
