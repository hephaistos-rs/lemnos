//! The Proxmox VE environments Lemnos manages, and how it signs in to each.
//!
//! This crate only keeps the details; connecting with them is up to
//! `lemnos-proxmox`. The secret is stored as it was given, so that it can be
//! sent to Proxmox again: whoever can read the database can read it.

use std::fmt;

use sqlx::{Row, any::AnyRow};

use crate::Database;

const COLUMNS: &str = "id, name, url, username, realm, token_id, secret, accept_invalid_certs";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProxmoxEnvironmentId(pub u64);

/// How Lemnos proves who it is to an environment.
#[derive(Clone, PartialEq, Eq)]
pub enum ProxmoxLogin {
    ApiToken {
        /// The token's name, the part after `!` in `root@pam!lemnos`.
        id: String,
        secret: String,
    },
    Password(String),
}

// Written by hand so a secret can't end up in a log through `{:?}`.
impl fmt::Debug for ProxmoxLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiToken { id, .. } => f
                .debug_struct("ApiToken")
                .field("id", id)
                .finish_non_exhaustive(),
            Self::Password(_) => f.write_str("Password(..)"),
        }
    }
}

/// An environment that isn't stored yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewProxmoxEnvironment {
    /// What Lemnos calls it. No two environments share a name.
    pub name: String,
    /// Scheme, host and port, e.g. `https://pve.example.com:8006`.
    pub url: String,
    /// User name without the realm, e.g. `root`.
    pub user: String,
    /// Realm the user belongs to, e.g. `pam` or `pve`.
    pub realm: String,
    pub login: ProxmoxLogin,
    /// Accept a certificate that does not verify, such as the self-signed
    /// one Proxmox installs by default.
    pub accept_invalid_certs: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxmoxEnvironment {
    pub id: ProxmoxEnvironmentId,
    pub name: String,
    pub url: String,
    pub user: String,
    pub realm: String,
    pub login: ProxmoxLogin,
    pub accept_invalid_certs: bool,
}

/// What an admin changes about a stored environment.
#[derive(Clone, PartialEq, Eq)]
pub struct ProxmoxEnvironmentEdit {
    pub name: String,
    pub url: String,
    pub user: String,
    pub realm: String,
    /// The token's name, or `None` to sign in with a password.
    pub token_id: Option<String>,
    /// `None` keeps the stored secret. That is only allowed while it would
    /// go to the same place for the same login: the same address, user,
    /// realm and token, and certificates checked at least as strictly.
    /// Otherwise whoever edits could have the secret sent to a server of
    /// their own, though Lemnos never shows it to them.
    pub secret: Option<String>,
    pub accept_invalid_certs: bool,
}

// Written by hand for the same reason as `ProxmoxLogin`'s.
impl fmt::Debug for ProxmoxEnvironmentEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxmoxEnvironmentEdit")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("user", &self.user)
            .field("realm", &self.realm)
            .field("token_id", &self.token_id)
            .field("secret", &self.secret.as_ref().map(|_| ".."))
            .field("accept_invalid_certs", &self.accept_invalid_certs)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProxmoxStoreError {
    #[error("another environment already has that name")]
    NameTaken,
    #[error("there is no such environment")]
    NotFound,
    /// The secret was left out while the address, user, realm or token
    /// changed, or certificate checks were turned off.
    #[error("the login or address changed, so it needs its secret again")]
    SecretNeeded,
    #[error("the database failed")]
    Backend(#[source] sqlx::Error),
}

type Result<T> = std::result::Result<T, ProxmoxStoreError>;

fn store_error(error: sqlx::Error) -> ProxmoxStoreError {
    match error.as_database_error() {
        Some(database_error) if database_error.is_unique_violation() => {
            ProxmoxStoreError::NameTaken
        }
        _ => ProxmoxStoreError::Backend(error),
    }
}

fn id(environment: ProxmoxEnvironmentId) -> i64 {
    environment.0 as i64
}

/// The login as its two columns: the token's name, if any, and the secret.
fn login_columns(login: ProxmoxLogin) -> (Option<String>, String) {
    match login {
        ProxmoxLogin::ApiToken { id, secret } => (Some(id), secret),
        ProxmoxLogin::Password(password) => (None, password),
    }
}

fn environment_from(row: &AnyRow) -> Result<ProxmoxEnvironment> {
    let token_id: Option<String> = row.try_get(5).map_err(store_error)?;
    let secret: String = row.try_get(6).map_err(store_error)?;
    Ok(ProxmoxEnvironment {
        id: ProxmoxEnvironmentId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
        name: row.try_get(1).map_err(store_error)?,
        url: row.try_get(2).map_err(store_error)?,
        user: row.try_get(3).map_err(store_error)?,
        realm: row.try_get(4).map_err(store_error)?,
        login: match token_id {
            Some(id) => ProxmoxLogin::ApiToken { id, secret },
            None => ProxmoxLogin::Password(secret),
        },
        accept_invalid_certs: row.try_get::<i64, _>(7).map_err(store_error)? != 0,
    })
}

impl Database {
    pub async fn create_proxmox_environment(
        &self,
        new: NewProxmoxEnvironment,
    ) -> Result<ProxmoxEnvironment> {
        // A random ID, for the same reason users get one.
        let environment = ProxmoxEnvironment {
            id: ProxmoxEnvironmentId(rand::random_range(1..=i64::MAX as u64)),
            name: new.name,
            url: new.url,
            user: new.user,
            realm: new.realm,
            login: new.login,
            accept_invalid_certs: new.accept_invalid_certs,
        };
        let (token_id, secret) = login_columns(environment.login.clone());
        self.query(&format!(
            "INSERT INTO proxmox_environments ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        ))
        .bind(id(environment.id))
        .bind(environment.name.clone())
        .bind(environment.url.clone())
        .bind(environment.user.clone())
        .bind(environment.realm.clone())
        .bind(token_id)
        .bind(secret)
        .bind(i64::from(environment.accept_invalid_certs))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(environment)
    }

    /// Every environment, ordered by name.
    pub async fn proxmox_environments(&self) -> Result<Vec<ProxmoxEnvironment>> {
        self.query(&format!(
            "SELECT {COLUMNS} FROM proxmox_environments ORDER BY name"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(environment_from)
        .collect()
    }

    pub async fn proxmox_environment(
        &self,
        environment: ProxmoxEnvironmentId,
    ) -> Result<Option<ProxmoxEnvironment>> {
        self.query(&format!(
            "SELECT {COLUMNS} FROM proxmox_environments WHERE id = ?"
        ))
        .bind(id(environment))
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .as_ref()
        .map(environment_from)
        .transpose()
    }

    /// Replaces everything stored about the environment with this ID.
    pub async fn update_proxmox_environment(&self, environment: ProxmoxEnvironment) -> Result<()> {
        // Checked first, because databases disagree on whether an update
        // that changes nothing counts as having matched a row.
        if self.proxmox_environment(environment.id).await?.is_none() {
            return Err(ProxmoxStoreError::NotFound);
        }
        let (token_id, secret) = login_columns(environment.login);
        self.query(
            "UPDATE proxmox_environments
             SET name = ?, url = ?, username = ?, realm = ?, token_id = ?, secret = ?,
                 accept_invalid_certs = ?
             WHERE id = ?",
        )
        .bind(environment.name)
        .bind(environment.url)
        .bind(environment.user)
        .bind(environment.realm)
        .bind(token_id)
        .bind(secret)
        .bind(i64::from(environment.accept_invalid_certs))
        .bind(id(environment.id))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// Applies an admin's changes, keeping the stored secret when none is
    /// given. Returns the environment as it is now stored.
    pub async fn edit_proxmox_environment(
        &self,
        environment: ProxmoxEnvironmentId,
        edit: ProxmoxEnvironmentEdit,
    ) -> Result<ProxmoxEnvironment> {
        let current = self
            .proxmox_environment(environment)
            .await?
            .ok_or(ProxmoxStoreError::NotFound)?;
        let secret = match edit.secret {
            Some(secret) => secret,
            None => {
                let current_token = match &current.login {
                    ProxmoxLogin::ApiToken { id, .. } => Some(id),
                    ProxmoxLogin::Password(_) => None,
                };
                let same_login = current.url == edit.url
                    && current.user == edit.user
                    && current.realm == edit.realm
                    && current_token == edit.token_id.as_ref()
                    && (current.accept_invalid_certs || !edit.accept_invalid_certs);
                if !same_login {
                    return Err(ProxmoxStoreError::SecretNeeded);
                }
                match current.login {
                    ProxmoxLogin::ApiToken { secret, .. } | ProxmoxLogin::Password(secret) => {
                        secret
                    }
                }
            }
        };
        let updated = ProxmoxEnvironment {
            id: environment,
            name: edit.name,
            url: edit.url,
            user: edit.user,
            realm: edit.realm,
            login: match edit.token_id {
                Some(id) => ProxmoxLogin::ApiToken { id, secret },
                None => ProxmoxLogin::Password(secret),
            },
            accept_invalid_certs: edit.accept_invalid_certs,
        };
        self.update_proxmox_environment(updated.clone()).await?;
        Ok(updated)
    }

    /// Forgets the environment. Nothing changes on the Proxmox side.
    pub async fn delete_proxmox_environment(
        &self,
        environment: ProxmoxEnvironmentId,
    ) -> Result<()> {
        // What organisations were given of it goes first; see
        // `delete_organization` for why this is spelled out.
        for statement in [
            "DELETE FROM machines WHERE environment_id = ?",
            "DELETE FROM organization_environments WHERE environment_id = ?",
            "DELETE FROM proxmox_environments WHERE id = ?",
        ] {
            self.query(statement)
                .bind(id(environment))
                .execute(&self.pool)
                .await
                .map_err(store_error)?;
        }
        Ok(())
    }
}
