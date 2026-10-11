//! Types shared by everything that signs users in through an outside system
//! (SSO providers and LDAP directories).

use std::fmt;

use serde::Deserialize;

/// Who an outside system says the user is.
///
/// Only `provider` + `subject` together identify a user. Do not link
/// accounts by `email` unless `email_verified` is `Some(true)`: most
/// providers let users type in any address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The configured ID of the SSO provider or LDAP directory.
    pub provider: String,
    /// The provider's stable, unique ID for the user.
    pub subject: String,
    pub email: Option<String>,
    /// `None` when the provider does not say.
    pub email_verified: Option<bool>,
    pub name: Option<String>,
    pub username: Option<String>,
}

/// A configuration value that must not end up in logs.
///
/// It has no `Display` and its `Debug` prints a placeholder, so the only way
/// to get at the value is to call [`expose`](Self::expose) on purpose.
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}
