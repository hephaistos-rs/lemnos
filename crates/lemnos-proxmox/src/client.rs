//! Authentication against the Proxmox VE HTTP API.
//!
//! [`connect`] takes an [`Endpoint`] and [`Credentials`] and hands back a
//! [`Client`]. It makes one authenticated request before returning, so bad
//! credentials fail right away instead of on your first real call:
//!
//! ```no_run
//! # async fn demo() -> Result<(), lemnos_proxmox::client::Error> {
//! use lemnos_proxmox::client::{Credentials, Endpoint, connect};
//!
//! let endpoint = Endpoint::new("https://pve.example.com:8006", "root", "pam");
//! let credentials = Credentials::ApiToken {
//!     id: "lemnos".into(),
//!     secret: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
//! };
//! let client = connect(&endpoint, &credentials).await?;
//! # Ok(())
//! # }
//! ```
//!
//! The client is cheap to clone. Pass it to the generated sub-clients in
//! [`proxmox_api`], e.g. `proxmox_api::nodes::NodesClient::new(client)`.

use std::fmt;
use std::time::Duration;

use proxmox_api::version::VersionClient;
use tokio::time::Instant;

/// An authenticated Proxmox VE API client.
pub use proxmox_api::ReqwestClient as Client;

/// Where to find a Proxmox VE node or cluster, and which user to log in as.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Base URL with no path, e.g. `https://pve.example.com:8006`.
    pub url: String,
    /// Username without the realm, e.g. `root`.
    pub user: String,
    /// The user's realm, e.g. `pam` or `pve`.
    pub realm: String,
    /// Skip TLS certificate verification. Proxmox ships with a self-signed
    /// cert, so you'll often need this, but it's off by default: without
    /// verification, anyone on the network path can read the secret.
    pub accept_invalid_certs: bool,
}

impl Endpoint {
    pub fn new(url: impl Into<String>, user: impl Into<String>, realm: impl Into<String>) -> Self {
        Self {
            url: url.into().trim_end_matches('/').to_string(),
            user: user.into(),
            realm: realm.into(),
            accept_invalid_certs: false,
        }
    }

    pub fn accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }
}

/// How we authenticate.
#[derive(Clone)]
pub enum Credentials {
    /// An API token belonging to the endpoint's user. It doesn't expire by
    /// itself and never needs refreshing, so prefer this for services.
    ApiToken {
        /// The token name: the part after `!` in `root@pam!lemnos`.
        id: String,
        secret: String,
    },
    /// The user's password. It gets exchanged for a ticket that's valid for
    /// two hours, so run [`keep_alive`] next to it to stay logged in.
    Password(String),
}

// Manual impl so secrets don't leak into logs via `{:?}`.
impl fmt::Debug for Credentials {
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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not set up the HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Proxmox rejected the request: {0}")]
    Api(#[from] proxmox_api::ReqwestError),
}

/// Logs in to `endpoint` and verifies the credentials actually work.
pub async fn connect(endpoint: &Endpoint, credentials: &Credentials) -> Result<Client, Error> {
    // Build our own HTTP client: `proxmox_api`'s default one disables
    // certificate checks.
    let http = reqwest::Client::builder()
        .danger_accept_invalid_certs(endpoint.accept_invalid_certs)
        .build()?;
    let client = Client::new(&endpoint.url, &endpoint.user, &endpoint.realm, Some(http));

    let client = match credentials {
        Credentials::ApiToken { id, secret } => client.with_api_token(id, secret),
        Credentials::Password(password) => client.with_login(password).await?,
    };

    // API tokens aren't validated until a request uses them, so make one now.
    VersionClient::new(&client).get().await?;

    Ok(client)
}

/// How long a ticket stays valid after it was issued.
const TICKET_LIFETIME: Duration = Duration::from_secs(2 * 60 * 60);
/// How often [`keep_alive`] looks at the ticket. The client itself only
/// renews one that is over an hour old, so most looks cost nothing.
const REFRESH_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Keeps a password login from expiring, for as long as this future runs.
///
/// It never finishes while things go well, so run it in the background:
///
/// ```no_run
/// # async fn demo(client: lemnos_proxmox::client::Client) {
/// let session = client.clone();
/// tokio::spawn(async move {
///     let error = lemnos_proxmox::client::keep_alive(&session).await;
///     eprintln!("Proxmox session lost: {error}");
/// });
/// # }
/// ```
///
/// It returns only once the ticket could not be renewed for so long that it
/// has expired, with the last error. After that, [`connect`] again. A client
/// that uses an API token has no ticket, so for it this does nothing.
pub async fn keep_alive(client: &Client) -> Error {
    let mut renewed = Instant::now();

    loop {
        tokio::time::sleep(REFRESH_INTERVAL).await;

        match client.refresh_auth_ticket(false).await {
            Ok(()) => renewed = Instant::now(),
            // A single failure is usually the network. The old ticket still
            // works, so keep trying until it can no longer be valid.
            Err(error) if renewed.elapsed() >= TICKET_LIFETIME => return error.into(),
            Err(_) => {}
        }
    }
}
