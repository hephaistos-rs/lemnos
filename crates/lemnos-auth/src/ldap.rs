//! Signing in against an LDAP directory or Active Directory (`ldap` feature).
//!
//! Unlike SSO, the user types their directory password into Lemnos, which
//! checks it with the directory. The usual "search, then bind" is used:
//!
//! 1. Connect and bind as a service account.
//! 2. Search for the one entry matching the username.
//! 3. Bind again as that entry with the user's password. If the directory
//!    accepts it, the password is right.
//!
//! The entry becomes an [`Identity`], which maps to a Lemnos account the same
//! way an SSO identity does.

use std::time::Duration;

use ldap3::{LdapConnAsync, LdapConnSettings, Scope, SearchEntry, ldap_escape};
use serde::Deserialize;

use crate::{
    auth::{Auth, AuthError, Login, Method, UnknownIdentity, login_key},
    identity::{Identity, Secret},
    store::Store,
};

const TIMEOUT: Duration = Duration::from_secs(15);
/// LDAP result code for a wrong DN or password.
const INVALID_CREDENTIALS: u32 = 49;

#[derive(Clone, Debug, Deserialize)]
pub struct LdapConfig {
    /// Names this directory in linked identities. Changing it later orphans
    /// every account linked through it.
    #[serde(default = "default_id")]
    pub id: String,
    /// `ldaps://host:636`, or `ldap://host:389` together with `starttls`.
    pub url: String,
    /// Upgrade a plain `ldap://` connection to TLS before sending anything.
    #[serde(default)]
    pub starttls: bool,
    /// Allow unencrypted `ldap://`. Passwords then cross the network in the
    /// clear; only for a test directory on localhost.
    #[serde(default)]
    pub danger_allow_plaintext: bool,
    /// The service account used to search for users.
    pub bind_dn: String,
    pub bind_password: Secret,
    /// Where to search for users, e.g. `ou=people,dc=example,dc=com`.
    pub user_base_dn: String,
    /// `{username}` is replaced by the (escaped) name the user typed. For
    /// Active Directory: `(&(objectClass=user)(sAMAccountName={username}))`.
    #[serde(default = "default_user_filter")]
    pub user_filter: String,
    #[serde(default)]
    pub attributes: LdapAttributes,
}

fn default_id() -> String {
    "ldap".into()
}

fn default_user_filter() -> String {
    "(&(objectClass=person)(uid={username}))".into()
}

/// Which attributes of a user's entry hold what.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct LdapAttributes {
    /// A value that never changes, even if the user is renamed or moved:
    /// `entryUUID` on OpenLDAP and most others, `objectGUID` on Active
    /// Directory. Not the DN or the username.
    pub id: String,
    pub username: String,
    pub email: String,
    pub name: String,
}

impl Default for LdapAttributes {
    fn default() -> Self {
        Self {
            id: "entryUUID".into(),
            username: "uid".into(),
            email: "mail".into(),
            name: "displayName".into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LdapError {
    #[error("invalid LDAP configuration: {0}")]
    Config(String),
    #[error("the LDAP directory did not answer in time")]
    Timeout,
    #[error("several LDAP entries match this username; tighten `user_filter`")]
    AmbiguousUser,
    #[error("the LDAP entry has no `{0}` attribute to identify the user by")]
    MissingId(String),
    #[error("the LDAP directory reported an error")]
    Directory(#[from] ldap3::LdapError),
}

pub(crate) struct LdapDirectory {
    config: LdapConfig,
}

impl LdapDirectory {
    fn new(config: LdapConfig) -> Result<Self, LdapError> {
        let encrypted = config.url.starts_with("ldaps://") || config.starttls;
        if !encrypted && !config.danger_allow_plaintext {
            return Err(LdapError::Config(
                "use an ldaps:// URL or set `starttls`; plain LDAP would send passwords unencrypted"
                    .into(),
            ));
        }
        if !config.user_filter.contains("{username}") {
            return Err(LdapError::Config(
                "`user_filter` must contain `{username}`".into(),
            ));
        }
        Ok(Self { config })
    }

    /// `Ok(None)` when the username or password is wrong.
    async fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<Identity>, LdapError> {
        // Many directories treat a bind with an empty password as an
        // anonymous bind and report success. Without this check, anyone
        // could sign in as anyone by leaving the password blank.
        if username.trim().is_empty() || password.is_empty() {
            return Ok(None);
        }
        tokio::time::timeout(TIMEOUT, self.search_and_bind(username.trim(), password))
            .await
            .map_err(|_| LdapError::Timeout)?
    }

    async fn search_and_bind(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<Identity>, LdapError> {
        let config = &self.config;
        let settings = LdapConnSettings::new()
            .set_conn_timeout(TIMEOUT)
            .set_starttls(config.starttls);
        let (connection, mut ldap) = LdapConnAsync::with_settings(settings, &config.url).await?;
        // The connection does its I/O on a background task; `ldap` is the
        // handle for sending requests over it.
        ldap3::drive!(connection);

        ldap.simple_bind(&config.bind_dn, config.bind_password.expose())
            .await?
            .success()?;

        // Escaping keeps a "username" like `*)(uid=*` from rewriting the filter.
        let filter = config
            .user_filter
            .replace("{username}", &ldap_escape(username));
        let attributes = &config.attributes;
        let (mut entries, _) = ldap
            .search(
                &config.user_base_dn,
                Scope::Subtree,
                &filter,
                vec![
                    attributes.id.as_str(),
                    attributes.username.as_str(),
                    attributes.email.as_str(),
                    attributes.name.as_str(),
                ],
            )
            .await?
            .success()?;
        let entry = match entries.len() {
            0 => return Ok(None),
            1 => SearchEntry::construct(entries.remove(0)),
            _ => return Err(LdapError::AmbiguousUser),
        };

        let bind = ldap.simple_bind(&entry.dn, password).await?;
        let _ = ldap.unbind().await;
        match bind.rc {
            0 => {}
            INVALID_CREDENTIALS => return Ok(None),
            _ => {
                bind.success()?;
            }
        }

        let text = |attribute: &str| {
            entry
                .attrs
                .get(attribute)
                .and_then(|values| values.first())
                .filter(|value| !value.is_empty())
                .cloned()
        };
        // Binary IDs (Active Directory's objectGUID) arrive separately from
        // text ones; hex makes them usable as a subject.
        let subject = text(&attributes.id)
            .or_else(|| {
                entry
                    .bin_attrs
                    .get(&attributes.id)
                    .and_then(|values| values.first())
                    .map(|bytes| bytes.iter().map(|byte| format!("{byte:02x}")).collect())
            })
            .ok_or_else(|| LdapError::MissingId(attributes.id.clone()))?;

        Ok(Some(Identity {
            provider: config.id.clone(),
            subject,
            email: text(&attributes.email),
            // The directory is the organisation's own record of the address.
            email_verified: Some(true),
            name: text(&attributes.name),
            username: text(&attributes.username).or_else(|| Some(username.to_owned())),
        }))
    }
}

impl<S: Store> Auth<S> {
    /// Turns on LDAP sign-in. Does not contact the directory yet.
    pub fn with_ldap(mut self, config: LdapConfig) -> Result<Self, LdapError> {
        self.ldap = Some(LdapDirectory::new(config)?);
        Ok(self)
    }

    /// Checks a username and password with the directory. `unknown` says
    /// what happens the first time a directory user signs in; for your own
    /// directory that is normally [`UnknownIdentity::CreateAccount`].
    pub async fn login_ldap(
        &self,
        username: &str,
        password: &str,
        unknown: UnknownIdentity,
    ) -> Result<Login, AuthError> {
        let directory = self.ldap.as_ref().ok_or(AuthError::NotConfigured("LDAP"))?;
        let key = login_key(&username.trim().to_lowercase());
        self.throttle.check(&key).map_err(AuthError::Throttled)?;

        let Some(identity) = directory.authenticate(username, password).await? else {
            self.throttle.failure(&key);
            return Err(AuthError::InvalidCredentials);
        };
        self.throttle.success(&key);
        let user = self.resolve_identity(&identity, unknown).await?;
        self.after_first_factor(user, Method::Ldap).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(url: &str) -> LdapConfig {
        serde_json::from_value(serde_json::json!({
            "url": url,
            "bind_dn": "cn=lemnos,dc=example,dc=com",
            "bind_password": "hunter2",
            "user_base_dn": "ou=people,dc=example,dc=com",
        }))
        .unwrap()
    }

    #[test]
    fn plaintext_ldap_is_refused_by_default() {
        assert!(LdapDirectory::new(config("ldaps://ldap.example.com")).is_ok());
        assert!(matches!(
            LdapDirectory::new(config("ldap://ldap.example.com")),
            Err(LdapError::Config(_))
        ));
        let starttls = LdapConfig {
            starttls: true,
            ..config("ldap://ldap.example.com")
        };
        assert!(LdapDirectory::new(starttls).is_ok());
    }

    #[test]
    fn filter_needs_a_username_placeholder() {
        let fixed = LdapConfig {
            user_filter: "(objectClass=person)".into(),
            ..config("ldaps://ldap.example.com")
        };
        assert!(matches!(
            LdapDirectory::new(fixed),
            Err(LdapError::Config(_))
        ));
    }

    #[tokio::test]
    async fn empty_passwords_never_reach_the_directory() {
        // The host doesn't exist, so getting `None` back (instead of a
        // connection error) shows no connection was attempted.
        let directory = LdapDirectory::new(config("ldaps://ldap.invalid")).unwrap();
        assert_eq!(directory.authenticate("alice", "").await.unwrap(), None);
        assert_eq!(
            directory.authenticate("  ", "password").await.unwrap(),
            None
        );
    }

    #[test]
    fn config_hides_the_bind_password() {
        assert!(!format!("{:?}", config("ldaps://ldap.example.com")).contains("hunter2"));
    }
}
