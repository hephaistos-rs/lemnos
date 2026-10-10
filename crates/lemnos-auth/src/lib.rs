//! Signing users in to Lemnos itself: accounts, passwords, sessions, SSO.
//!
//! Not to be confused with each backend crate's own `auth` module (e.g.
//! `lemnos-proxmox`), which is how Lemnos signs in to that platform.
//!
//! Kept free of Topcoat on purpose: the UI turns HTTP requests into calls to
//! this crate, so the same logic could later back a CLI or an API.

pub mod sso;

/// A signed-in Lemnos user. Placeholder until accounts are stored somewhere.
#[derive(Clone, Debug)]
pub struct User {
    pub name: String,
}
