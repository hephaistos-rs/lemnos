// Which sign-in methods this Lemnos offers. Read once at startup from the
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
