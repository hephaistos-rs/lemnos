// Which sign-in methods this Lemnos offers, which Proxmox VE environments
// it manages, who its admins are and which organisations exist from the
// start. Read once at startup from the
// TOML file named by the `LEMNOS_CONFIG` environment variable, or else from
// `lemnos.toml` in the directory Lemnos is started from. With neither, only
// local accounts work. See `lemnos.dev.toml` for an example.

use std::{env, error::Error, fs, path::PathBuf};

const DEFAULT_PATH: &str = "lemnos.toml";

use lemnos_auth::{Secret, ldap::LdapConfig, passkey::PasskeyConfig, sso::ProviderConfig};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Where accounts and sessions are kept: `sqlite:`, `postgres://` or
    /// `mysql://` (MariaDB too). See `lemnos-db` for the URL forms.
    pub database_url: Secret,
    /// Whether visitors can create their own local account.
    pub allow_sign_up: bool,
    /// IDs of the SSO providers (or the LDAP directory) whose users get an
    /// account on their first sign-in. Never list a provider anyone can
    /// register at, such as GitHub: that would let everyone in.
    pub auto_provision: Vec<String>,
    pub sso: Vec<ProviderConfig>,
    pub ldap: Option<LdapConfig>,
    pub passkeys: Option<PasskeyConfig>,
    /// Proxmox VE environments to manage, one `[[proxmox]]` table each.
    pub proxmox: Vec<ProxmoxConfig>,
    /// Admins of the installation, one `[[admin]]` table each.
    pub admin: Vec<AdminConfig>,
    /// Organisations that exist from the start, one `[[organization]]`
    /// table each.
    pub organization: Vec<OrganizationConfig>,
}

/// An admin of the whole installation.
///
/// Checked at every start: an account that does not exist yet is created
/// with `password`, and one that does is made an admin and keeps the
/// password it has. Without any `[[admin]]`, the first account ever created
/// becomes one.
///
/// One more admin can be named without a file, through the environment
/// variables `LEMNOS_ADMIN_USERNAME` and `LEMNOS_ADMIN_PASSWORD`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminConfig {
    pub username: String,
    /// The first password, at least 12 characters. Only used to create the
    /// account; change it after signing in.
    pub password: Option<Secret>,
}

impl AdminConfig {
    pub fn from_environment() -> Option<Self> {
        let username = env::var("LEMNOS_ADMIN_USERNAME").ok()?;
        Some(Self {
            username,
            password: env::var("LEMNOS_ADMIN_PASSWORD").ok().map(Secret::new),
        })
    }
}

/// An organisation: a group of people who share machines. Created at start
/// if it does not exist yet; which environments it may use is set in the
/// console.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationConfig {
    /// No two organisations share a name.
    pub name: String,
    /// IDs of the SSO providers (or `ldap`) whose users join this
    /// organisation as members when they sign in.
    #[serde(default)]
    pub providers: Vec<String>,
}

/// One Proxmox VE environment. Set either the two `token_*` keys or
/// `password`, not both.
///
/// Environments listed here are written to the database at every start, so
/// the file wins over changes made to them on the Environments page. Ones added
/// on that page are kept in the database only and are left alone.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxmoxConfig {
    /// What Lemnos calls it. No two environments share a name.
    pub name: String,
    /// Scheme, host and port, e.g. `https://pve.example.com:8006`.
    pub url: String,
    /// User name without the realm, e.g. `root`. Can be left out when
    /// `token_id` is the full `user@realm!name`.
    #[serde(default)]
    pub user: String,
    /// Realm the user belongs to, e.g. `pam` or `pve`. Can be left out like
    /// `user`.
    #[serde(default)]
    pub realm: String,
    /// The API token's ID as Proxmox shows it, `root@pam!lemnos`, or just
    /// the name after the `!`.
    pub token_id: Option<String>,
    pub token_secret: Option<Secret>,
    pub password: Option<Secret>,
    /// Accept a certificate that does not verify, such as the self-signed
    /// one Proxmox installs by default.
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // A file in the directory Lemnos is started from, created on
            // first use. A URL can hold a password, hence `Secret`.
            database_url: Secret::new("sqlite:lemnos.db?mode=rwc"),
            allow_sign_up: false,
            auto_provision: Vec::new(),
            sso: Vec::new(),
            ldap: None,
            passkeys: None,
            proxmox: Vec::new(),
            admin: Vec::new(),
            organization: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn Error>> {
        let path = match env::var_os("LEMNOS_CONFIG") {
            // Asked for by name, so it has to exist.
            Some(path) => PathBuf::from(path),
            // The default file is optional.
            None if fs::exists(DEFAULT_PATH)? => PathBuf::from(DEFAULT_PATH),
            None => {
                eprintln!("no {DEFAULT_PATH} found; only local accounts are available");
                return Ok(Self::default());
            }
        };
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("reading {}: {error}", path.display()))?;
        let config =
            toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        eprintln!("config loaded from {}", path.display());
        Ok(config)
    }
}
