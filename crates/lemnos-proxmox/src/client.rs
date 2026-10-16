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
/// The error every call can return.
pub use proxmox_api::ReqwestError;
/// The HTTP status inside a [`ReqwestError::UnknownFailure`].
pub use reqwest::StatusCode;

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
    /// How long to wait for a connection to Proxmox before giving up.
    pub connect_timeout: Duration,
    /// How long one request may take from start to finished answer.
    pub request_timeout: Duration,
}

/// A reachable Proxmox answers within milliseconds, so after this the host
/// is down or the address is wrong.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Every slow operation (copying, starting, removing) returns a task ID right
/// away and is followed with [`crate::tasks::wait`], one short request per
/// poll. So no single request should get near this; it only stops a hung
/// Proxmox from holding a page open forever.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

impl Endpoint {
    pub fn new(url: impl Into<String>, user: impl Into<String>, realm: impl Into<String>) -> Self {
        Self {
            url: url.into().trim_end_matches('/').to_string(),
            user: user.into(),
            realm: realm.into(),
            accept_invalid_certs: false,
            connect_timeout: CONNECT_TIMEOUT,
            request_timeout: REQUEST_TIMEOUT,
        }
    }

    pub fn accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
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
        /// [`crate::token::split`] gets it out of the full form.
        id: String,
        /// The token secret.
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
    #[error("the request to Proxmox failed: {0}")]
    Api(#[from] proxmox_api::ReqwestError),
}

/// What kind of trouble an [`Error`] is, as far as the person who set up
/// the environment can do something about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// No answer: wrong address, host down, timed out, or a certificate
    /// that does not verify.
    Unreachable,
    /// Proxmox answered, and turned the login down (HTTP 401): a token ID in
    /// the wrong form, a wrong secret or password, or an expired token.
    LoginRejected,
    /// Signed in, but the user or token may not do this (HTTP 403).
    NotAllowed,
    Other,
}

impl Error {
    pub fn failure(&self) -> Failure {
        match self {
            Self::Http(_) => Failure::Other,
            Self::Api(error) => failure_of(error),
        }
    }
}

/// [`Error::failure`] for an error straight from a call.
pub fn failure_of(error: &ReqwestError) -> Failure {
    match error {
        ReqwestError::Reqwest(error) if error.is_connect() || error.is_timeout() => {
            Failure::Unreachable
        }
        ReqwestError::UnknownFailure(StatusCode::UNAUTHORIZED, _) => Failure::LoginRejected,
        ReqwestError::UnknownFailure(StatusCode::FORBIDDEN, _) => Failure::NotAllowed,
        _ => Failure::Other,
    }
}

/// Logs in to `endpoint` and verifies the credentials actually work.
pub async fn connect(endpoint: &Endpoint, credentials: &Credentials) -> Result<Client, Error> {
    // Build our own HTTP client: `proxmox_api`'s default one disables
    // certificate checks.
    let http = reqwest::Client::builder()
        .danger_accept_invalid_certs(endpoint.accept_invalid_certs)
        .connect_timeout(endpoint.connect_timeout)
        .timeout(endpoint.request_timeout)
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

/// Runs a call whose generated return type is `()`, such as setting a
/// guest's config.
///
/// Proxmox answers those with `{"data":null}`. `proxmox_api` reads the `null`
/// as "no data" and, with no `errors` either, reports a call that worked as
/// `UnknownFailure(200 OK, ..)`. This turns exactly that case back into
/// success. A real failure comes with another status, so it still errors.
pub(crate) async fn no_data(
    call: impl Future<Output = Result<(), ReqwestError>>,
) -> Result<(), ReqwestError> {
    accept_null(call.await)
}

fn accept_null(result: Result<(), ReqwestError>) -> Result<(), ReqwestError> {
    match result {
        Err(ReqwestError::UnknownFailure(StatusCode::OK, _)) => Ok(()),
        other => other,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_null_answer_with_200_is_success() {
        let null = Err(ReqwestError::UnknownFailure(
            StatusCode::OK,
            Some(r#"{"data":null}"#.into()),
        ));
        assert!(accept_null(null).is_ok());
        assert!(accept_null(Ok(())).is_ok());
    }

    #[test]
    fn other_failures_still_fail() {
        let server = Err(ReqwestError::UnknownFailure(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some("VM 100 is locked (clone)".into()),
        ));
        assert!(matches!(
            accept_null(server),
            Err(ReqwestError::UnknownFailure(status, _)) if status == StatusCode::INTERNAL_SERVER_ERROR
        ));

        let errors = Err(ReqwestError::EncounteredErrors(
            serde_json::json!({"tags": "invalid"}),
        ));
        assert!(matches!(
            accept_null(errors),
            Err(ReqwestError::EncounteredErrors(_))
        ));
    }

    #[test]
    fn a_rejected_login_is_told_apart() {
        let status =
            |code| ReqwestError::UnknownFailure(code, Some("authentication failure".into()));
        assert_eq!(
            failure_of(&status(StatusCode::UNAUTHORIZED)),
            Failure::LoginRejected
        );
        assert_eq!(
            failure_of(&status(StatusCode::FORBIDDEN)),
            Failure::NotAllowed
        );
        assert_eq!(
            failure_of(&status(StatusCode::INTERNAL_SERVER_ERROR)),
            Failure::Other
        );
    }

    #[tokio::test]
    async fn no_data_awaits_the_call() {
        let call = async { Err(ReqwestError::UnknownFailure(StatusCode::OK, None)) };
        assert!(no_data(call).await.is_ok());
    }
}
