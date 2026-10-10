//! Signing users in to Lemnos itself: accounts, passwords, sessions, SSO.
//!
//! Not to be confused with each backend crate's own `auth` module (e.g.
//! `lemnos-proxmox`), which is how Lemnos signs in to that platform.
//!
//! Kept free of Topcoat on purpose: the UI turns HTTP requests into calls to
//! this crate, so the same logic could later back a CLI or an API.
//!
//! Start at [`Auth`]. Ways to sign in:
//!
//! - a local password ([`Auth::login_password`]), optionally followed by a
//!   [TOTP](totp) code;
//! - an LDAP directory or Active Directory (`ldap` feature);
//! - a passkey (`passkeys` feature);
//! - an outside identity provider ([`sso`]): OIDC, OAuth2, or SAML (`saml`
//!   feature).

use std::time::{SystemTime, UNIX_EPOCH};

mod auth;
mod identity;
mod password;
pub mod store;
mod throttle;
pub mod totp;
mod user;

#[cfg(feature = "ldap")]
pub mod ldap;
#[cfg(feature = "passkeys")]
pub mod passkey;
pub mod sso;

pub use auth::{
    Auth, AuthConfig, AuthError, Authenticated, Login, Method, SecondFactorChallenge,
    UnknownIdentity,
};
pub use identity::{Identity, Secret};
pub use password::{PasswordPolicy, WeakPassword};
pub use store::{MemoryStore, SessionId, Store, StoreError};
pub use totp::TotpEnrollment;
pub use user::{InvalidUsername, NewUser, User, UserId, Username};

/// Seconds since the Unix epoch. Used for timestamps that get serialized.
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
