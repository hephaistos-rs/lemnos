//! Single sign-on: sending users to an external identity provider and turning
//! what comes back into an [`Identity`].
//!
//! Three kinds of provider are supported:
//!
//! - [OpenID Connect](oidc) for anything spec-compliant (Entra ID, Google,
//!   Keycloak, Authentik, Zitadel, Okta, ...).
//! - [Generic OAuth2](oauth) for providers without OIDC (GitHub, Discord,
//!   ...): the user is looked up through a userinfo URL instead.
//! - [SAML 2.0](saml), behind the `saml` feature. The underlying crate is
//!   pre-1.0 and unaudited, so it is opt-in.
//!
//! A sign-in is two calls with a browser round trip in between:
//!
//! 1. [`Sso::begin`] returns the URL to send the browser to, plus a
//!    [`PendingLogin`] the caller keeps until the browser comes back.
//! 2. [`Sso::finish`] takes that [`PendingLogin`] and the provider's
//!    [`Callback`], verifies everything, and returns the [`Identity`].
//!
//! This module stores nothing itself; where the [`PendingLogin`] lives and
//! which Lemnos account an [`Identity`] maps to are up to the caller.

use std::{error::Error as StdError, fmt, time::Duration};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::unix_now;
pub use crate::{Identity, Secret};

pub mod oauth;
pub mod oidc;
#[cfg(feature = "saml")]
pub mod saml;

pub use oauth::{ClaimMapping, ClientAuth, OAuthConfig};
pub use oidc::OidcConfig;
#[cfg(feature = "saml")]
pub use saml::{IdpMetadata, SamlCallback, SamlConfig};

/// How long a [`PendingLogin`] stays valid. Long enough to type a password
/// and do 2FA at the provider, short enough that a leaked one is soon useless.
pub const PENDING_MAX_AGE: Duration = Duration::from_secs(10 * 60);

type BoxError = Box<dyn StdError + Send + Sync>;

/// One configured identity provider.
#[derive(Clone, Debug, Deserialize)]
pub struct ProviderConfig {
    /// Stable identifier, used in URLs (e.g. `/auth/sso/<id>/callback`).
    /// Lowercase letters, digits and `-` only.
    pub id: String,
    /// Shown on the sign-in button.
    pub display_name: String,
    #[serde(flatten)]
    pub kind: ProviderKind,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[expect(clippy::large_enum_variant, reason = "read once at startup")]
pub enum ProviderKind {
    Oidc(OidcConfig),
    #[serde(rename = "oauth2")]
    OAuth2(OAuthConfig),
    #[cfg(feature = "saml")]
    Saml(SamlConfig),
}

/// Where to send the browser, and what to remember until it comes back.
#[derive(Debug)]
pub struct LoginStart {
    pub redirect_to: Url,
    pub pending: PendingLogin,
}

/// What Lemnos must remember between [`Sso::begin`] and [`Sso::finish`]: the
/// CSRF state, PKCE verifier and nonce, or the SAML request tracker.
///
/// It holds secrets, so keep it server-side or in an encrypted cookie; the
/// browser must not be able to read or change it. Use it once, then drop it.
#[derive(Clone, Serialize, Deserialize)]
pub struct PendingLogin {
    provider: String,
    /// Seconds since the Unix epoch.
    issued_at: u64,
    flow: PendingFlow,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "flow", rename_all = "kebab-case")]
enum PendingFlow {
    Oidc {
        state: String,
        nonce: String,
        pkce_verifier: String,
    },
    #[serde(rename = "oauth2")]
    OAuth2 {
        state: String,
        pkce_verifier: String,
    },
    #[cfg(feature = "saml")]
    Saml {
        /// [`::saml::LoginTrackerPayload`], sealed with the provider's tracker key.
        tracker: String,
    },
}

impl PendingLogin {
    fn new(provider: &str, flow: PendingFlow) -> Self {
        Self {
            provider: provider.to_owned(),
            issued_at: unix_now(),
            flow,
        }
    }

    /// The provider this sign-in was started with.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    fn is_expired(&self) -> bool {
        unix_now().saturating_sub(self.issued_at) > PENDING_MAX_AGE.as_secs()
    }
}

impl fmt::Debug for PendingLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingLogin")
            .field("provider", &self.provider)
            .field("issued_at", &self.issued_at)
            .finish_non_exhaustive()
    }
}

/// What the provider sent back with the browser.
#[derive(Clone, Debug)]
pub enum Callback {
    /// OIDC and OAuth2: the query string of the redirect.
    OAuth(OAuthCallback),
    /// SAML: the form the IdP POSTs to the assertion consumer service.
    #[cfg(feature = "saml")]
    Saml(SamlCallback),
}

/// Query parameters on an OIDC/OAuth2 redirect back to Lemnos. Deserialize
/// the callback's query string straight into this.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct OAuthCallback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

impl OAuthCallback {
    /// Checks `state` first, so a forged callback can't even report an error,
    /// then returns the authorization code.
    fn verify(&self, expected_state: &str) -> Result<&str, SsoError> {
        let state = self
            .state
            .as_deref()
            .ok_or(SsoError::MissingParameter("state"))?;
        if !constant_time_eq(state.as_bytes(), expected_state.as_bytes()) {
            return Err(SsoError::StateMismatch);
        }
        if let Some(error) = &self.error {
            return Err(SsoError::Provider {
                error: error.clone(),
                description: self.error_description.clone(),
            });
        }
        self.code
            .as_deref()
            .ok_or(SsoError::MissingParameter("code"))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SsoError {
    #[error("unknown SSO provider `{0}`")]
    UnknownProvider(String),
    #[error("invalid SSO configuration: {0}")]
    Config(String),
    #[error("this sign-in has expired or was started with a different provider")]
    InvalidPending,
    #[error("the callback's state does not match this sign-in")]
    StateMismatch,
    #[error("the callback is missing `{0}`")]
    MissingParameter(&'static str),
    #[error("the provider refused the sign-in: {error}{}", description.as_deref().map(|d| format!(" ({d})")).unwrap_or_default())]
    Provider {
        error: String,
        description: Option<String>,
    },
    #[error("OIDC discovery failed")]
    Discovery(#[source] BoxError),
    #[error("exchanging the authorization code failed")]
    TokenExchange(#[source] BoxError),
    #[error("the ID token is invalid")]
    IdToken(#[source] BoxError),
    #[error("fetching user info failed")]
    UserInfo(#[source] BoxError),
    #[error("the provider's user info has no `{0}` field to identify the user by")]
    MissingSubject(String),
    #[cfg(feature = "saml")]
    #[error(
        "the IdP sent a transient NameID, which changes on every sign-in; configure it to send a persistent or email NameID"
    )]
    TransientSubject,
    #[cfg(feature = "saml")]
    #[error("SAML: {0}")]
    Saml(#[from] ::saml::Error),
}

impl SsoError {
    fn boxed<E: StdError + Send + Sync + 'static>(
        variant: fn(BoxError) -> Self,
    ) -> impl FnOnce(E) -> Self {
        move |error| variant(Box::new(error))
    }
}

/// Short, URL-safe name for the sign-on buttons.
#[derive(Clone, Copy, Debug)]
pub struct ProviderInfo<'a> {
    pub id: &'a str,
    pub display_name: &'a str,
}

#[cfg_attr(
    feature = "saml",
    expect(clippy::large_enum_variant, reason = "built once at startup")
)]
enum Provider {
    Oidc(oidc::OidcProvider),
    OAuth2(oauth::OAuthProvider),
    #[cfg(feature = "saml")]
    Saml(saml::SamlProvider),
}

struct Entry {
    id: String,
    display_name: String,
    provider: Provider,
}

/// All configured identity providers. Build once at startup and share it.
pub struct Sso {
    entries: Vec<Entry>,
}

impl Sso {
    /// Sets up every provider. Talks to the network: OIDC providers are
    /// discovered and SAML metadata may be downloaded, so a provider that is
    /// down at startup fails the whole call.
    pub async fn from_config(configs: Vec<ProviderConfig>) -> Result<Self, SsoError> {
        let http = http_client()?;
        let mut entries: Vec<Entry> = Vec::with_capacity(configs.len());
        for config in configs {
            validate_id(&config.id)?;
            if entries.iter().any(|entry| entry.id == config.id) {
                return Err(SsoError::Config(format!(
                    "provider id `{}` is used twice",
                    config.id
                )));
            }
            let provider = match config.kind {
                ProviderKind::Oidc(oidc) => {
                    Provider::Oidc(oidc::OidcProvider::discover(oidc, http.clone()).await?)
                }
                ProviderKind::OAuth2(oauth) => {
                    Provider::OAuth2(oauth::OAuthProvider::new(oauth, http.clone()))
                }
                #[cfg(feature = "saml")]
                ProviderKind::Saml(saml) => {
                    Provider::Saml(saml::SamlProvider::new(saml, &http).await?)
                }
            };
            entries.push(Entry {
                id: config.id,
                display_name: config.display_name,
                provider,
            });
        }
        Ok(Self { entries })
    }

    /// The configured providers, in configuration order.
    pub fn providers(&self) -> impl Iterator<Item = ProviderInfo<'_>> {
        self.entries.iter().map(|entry| ProviderInfo {
            id: &entry.id,
            display_name: &entry.display_name,
        })
    }

    /// Starts a sign-in with `provider_id`.
    pub async fn begin(&self, provider_id: &str) -> Result<LoginStart, SsoError> {
        match &self.entry(provider_id)?.provider {
            Provider::Oidc(oidc) => oidc.begin(provider_id).await,
            Provider::OAuth2(oauth) => oauth.begin(provider_id),
            #[cfg(feature = "saml")]
            Provider::Saml(saml) => saml.begin(provider_id),
        }
    }

    /// Completes a sign-in. `provider_id` is the provider the callback came
    /// in for (e.g. from its URL); it must match the one the sign-in started
    /// with, so one provider's response can't be replayed against another.
    pub async fn finish(
        &self,
        provider_id: &str,
        pending: PendingLogin,
        callback: Callback,
    ) -> Result<Identity, SsoError> {
        let entry = self.entry(provider_id)?;
        if pending.provider != provider_id || pending.is_expired() {
            return Err(SsoError::InvalidPending);
        }
        match (&entry.provider, pending.flow, callback) {
            (
                Provider::Oidc(oidc),
                PendingFlow::Oidc {
                    state,
                    nonce,
                    pkce_verifier,
                },
                Callback::OAuth(callback),
            ) => {
                oidc.finish(provider_id, &state, nonce, pkce_verifier, &callback)
                    .await
            }
            (
                Provider::OAuth2(oauth),
                PendingFlow::OAuth2 {
                    state,
                    pkce_verifier,
                },
                Callback::OAuth(callback),
            ) => {
                oauth
                    .finish(provider_id, &state, pkce_verifier, &callback)
                    .await
            }
            #[cfg(feature = "saml")]
            (Provider::Saml(saml), PendingFlow::Saml { tracker }, Callback::Saml(callback)) => {
                saml.finish(provider_id, &tracker, &callback)
            }
            _ => Err(SsoError::InvalidPending),
        }
    }

    /// This service provider's SAML metadata, to register Lemnos at the IdP.
    #[cfg(feature = "saml")]
    pub fn saml_metadata(&self, provider_id: &str) -> Result<String, SsoError> {
        match &self.entry(provider_id)?.provider {
            Provider::Saml(saml) => saml.metadata(),
            _ => Err(SsoError::UnknownProvider(provider_id.to_owned())),
        }
    }

    fn entry(&self, provider_id: &str) -> Result<&Entry, SsoError> {
        self.entries
            .iter()
            .find(|entry| entry.id == provider_id)
            .ok_or_else(|| SsoError::UnknownProvider(provider_id.to_owned()))
    }
}

/// One HTTP client for all provider back-channel calls.
fn http_client() -> Result<reqwest::Client, SsoError> {
    reqwest::Client::builder()
        // A redirect from a token or discovery endpoint could point us at an
        // internal address; providers don't need them, so refuse them.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        // Some APIs (GitHub's) reject requests without a User-Agent.
        .user_agent(concat!("lemnos/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| SsoError::Config(format!("building the HTTP client failed: {error}")))
}

fn validate_id(id: &str) -> Result<(), SsoError> {
    let valid = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(SsoError::Config(format!(
            "provider id `{id}` may only contain lowercase letters, digits and `-`"
        )))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn callback(state: Option<&str>, code: Option<&str>, error: Option<&str>) -> OAuthCallback {
        OAuthCallback {
            code: code.map(Into::into),
            state: state.map(Into::into),
            error: error.map(Into::into),
            error_description: None,
        }
    }

    #[test]
    fn callback_requires_matching_state() {
        assert_eq!(
            callback(Some("s"), Some("c"), None).verify("s").unwrap(),
            "c"
        );
        assert!(matches!(
            callback(Some("x"), Some("c"), None).verify("s"),
            Err(SsoError::StateMismatch)
        ));
        assert!(matches!(
            callback(None, Some("c"), None).verify("s"),
            Err(SsoError::MissingParameter("state"))
        ));
    }

    #[test]
    fn callback_error_needs_valid_state_to_be_reported() {
        assert!(matches!(
            callback(Some("x"), None, Some("access_denied")).verify("s"),
            Err(SsoError::StateMismatch)
        ));
        assert!(matches!(
            callback(Some("s"), None, Some("access_denied")).verify("s"),
            Err(SsoError::Provider { .. })
        ));
    }

    #[test]
    fn pending_login_expires() {
        let mut pending = PendingLogin::new(
            "github",
            PendingFlow::OAuth2 {
                state: "s".into(),
                pkce_verifier: "v".into(),
            },
        );
        assert!(!pending.is_expired());
        pending.issued_at -= PENDING_MAX_AGE.as_secs() + 1;
        assert!(pending.is_expired());
    }

    #[test]
    fn pending_login_round_trips_and_hides_secrets() {
        let pending = PendingLogin::new(
            "github",
            PendingFlow::OAuth2 {
                state: "s".into(),
                pkce_verifier: "very-secret".into(),
            },
        );
        let json = serde_json::to_string(&pending).unwrap();
        let back: PendingLogin = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider(), "github");
        assert!(!format!("{back:?}").contains("very-secret"));
    }

    #[test]
    fn provider_ids_are_url_safe() {
        assert!(validate_id("entra-id").is_ok());
        assert!(validate_id("").is_err());
        assert!(validate_id("Entra").is_err());
        assert!(validate_id("a/b").is_err());
    }

    #[test]
    fn config_deserializes() {
        let configs: Vec<ProviderConfig> = serde_json::from_value(serde_json::json!([
            {
                "id": "keycloak",
                "display_name": "Keycloak",
                "type": "oidc",
                "issuer_url": "https://sso.example.com/realms/lemnos",
                "client_id": "lemnos",
                "client_secret": "hunter2",
                "redirect_url": "https://lemnos.example.com/auth/sso/keycloak/callback"
            },
            {
                "id": "github",
                "display_name": "GitHub",
                "type": "oauth2",
                "client_id": "abc",
                "auth_url": "https://github.com/login/oauth/authorize",
                "token_url": "https://github.com/login/oauth/access_token",
                "userinfo_url": "https://api.github.com/user",
                "redirect_url": "https://lemnos.example.com/auth/sso/github/callback",
                "scopes": ["read:user", "user:email"],
                "claims": { "subject": "id", "username": "login" }
            }
        ]))
        .unwrap();
        let ProviderKind::Oidc(oidc) = &configs[0].kind else {
            panic!("expected OIDC")
        };
        assert_eq!(oidc.scopes, ["email", "profile"]);
        assert!(!format!("{oidc:?}").contains("hunter2"));
        let ProviderKind::OAuth2(oauth) = &configs[1].kind else {
            panic!("expected OAuth2")
        };
        assert_eq!(oauth.claims.subject, "id");
        assert_eq!(oauth.claims.email, "email");
    }

    #[tokio::test]
    async fn rejects_duplicate_ids_and_mismatched_providers() {
        let github = |id: &str| ProviderConfig {
            id: id.into(),
            display_name: "GitHub".into(),
            kind: ProviderKind::OAuth2(OAuthConfig {
                client_id: "abc".into(),
                client_secret: None,
                auth_url: "https://github.com/login/oauth/authorize".parse().unwrap(),
                token_url: "https://github.com/login/oauth/access_token"
                    .parse()
                    .unwrap(),
                userinfo_url: "https://api.github.com/user".parse().unwrap(),
                redirect_url: "https://lemnos.example.com/cb".parse().unwrap(),
                scopes: vec![],
                client_auth: ClientAuth::default(),
                claims: ClaimMapping::default(),
            }),
        };
        assert!(matches!(
            Sso::from_config(vec![github("gh"), github("gh")]).await,
            Err(SsoError::Config(_))
        ));

        let sso = Sso::from_config(vec![github("gh"), github("gh2")])
            .await
            .unwrap();
        let start = sso.begin("gh").await.unwrap();
        let query: Vec<_> = start
            .redirect_to
            .query_pairs()
            .map(|(k, _)| k.into_owned())
            .collect();
        assert!(query.iter().any(|k| k == "state"));
        assert!(query.iter().any(|k| k == "code_challenge"));

        // A sign-in started with `gh` can't be finished as `gh2`.
        let result = sso
            .finish(
                "gh2",
                start.pending,
                Callback::OAuth(OAuthCallback::default()),
            )
            .await;
        assert!(matches!(result, Err(SsoError::InvalidPending)));
    }
}
