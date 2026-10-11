//! OpenID Connect: discovery, the authorization code flow with PKCE, and ID
//! token verification.

use std::{
    sync::{PoisonError, RwLock},
    time::{Duration, Instant},
};

use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
};
use serde::Deserialize;
use url::Url;

use super::{Identity, LoginStart, OAuthCallback, PendingFlow, PendingLogin, Secret, SsoError};

/// How often the provider's metadata (and with it, its signing keys) is
/// fetched again, so key rotation at the provider doesn't need a restart.
const REDISCOVER_AFTER: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, Deserialize)]
pub struct OidcConfig {
    /// `/.well-known/openid-configuration` is fetched from below this URL.
    pub issuer_url: Url,
    pub client_id: String,
    /// Leave out for a public client; PKCE protects the flow either way.
    #[serde(default)]
    pub client_secret: Option<Secret>,
    /// Where the provider sends the browser back to. Must be registered at
    /// the provider exactly as written here.
    pub redirect_url: Url,
    /// Requested on top of `openid`, which is always sent.
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
}

fn default_scopes() -> Vec<String> {
    vec!["email".into(), "profile".into()]
}

type DiscoveredClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub(super) struct OidcProvider {
    config: OidcConfig,
    http: reqwest::Client,
    client: RwLock<(Instant, DiscoveredClient)>,
}

impl OidcProvider {
    pub(super) async fn discover(
        config: OidcConfig,
        http: reqwest::Client,
    ) -> Result<Self, SsoError> {
        let client = build_client(&config, &http).await?;
        Ok(Self {
            config,
            http,
            client: RwLock::new((Instant::now(), client)),
        })
    }

    /// The client, rediscovered if it is getting old. If rediscovery fails
    /// the old one is kept: its keys are most likely still valid.
    async fn client(&self) -> DiscoveredClient {
        let (discovered_at, client) = self
            .client
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if discovered_at.elapsed() < REDISCOVER_AFTER {
            return client;
        }
        match build_client(&self.config, &self.http).await {
            Ok(fresh) => {
                *self.client.write().unwrap_or_else(PoisonError::into_inner) =
                    (Instant::now(), fresh.clone());
                fresh
            }
            Err(_) => client,
        }
    }

    pub(super) async fn begin(&self, provider_id: &str) -> Result<LoginStart, SsoError> {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let client = self.client().await;
        let mut request = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(pkce_challenge);
        for scope in &self.config.scopes {
            request = request.add_scope(Scope::new(scope.clone()));
        }
        let (redirect_to, state, nonce) = request.url();
        Ok(LoginStart {
            redirect_to,
            pending: PendingLogin::new(
                provider_id,
                PendingFlow::Oidc {
                    state: state.into_secret(),
                    nonce: nonce.secret().clone(),
                    pkce_verifier: pkce_verifier.into_secret(),
                },
            ),
        })
    }

    pub(super) async fn finish(
        &self,
        provider_id: &str,
        state: &str,
        nonce: String,
        pkce_verifier: String,
        callback: &OAuthCallback,
    ) -> Result<Identity, SsoError> {
        let code = callback.verify(state)?;
        let client = self.client().await;
        let token = client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .map_err(SsoError::boxed(SsoError::TokenExchange))?
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
            .request_async(&self.http)
            .await
            .map_err(SsoError::boxed(SsoError::TokenExchange))?;

        let id_token = token
            .id_token()
            .ok_or_else(|| SsoError::IdToken("the provider returned no ID token".into()))?;
        let verifier = client.id_token_verifier();
        let claims = id_token
            .claims(&verifier, &Nonce::new(nonce))
            .map_err(SsoError::boxed(SsoError::IdToken))?;

        // Ties the access token to this ID token, so one can't be swapped in
        // from another user's sign-in.
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                token.access_token(),
                id_token
                    .signing_alg()
                    .map_err(SsoError::boxed(SsoError::IdToken))?,
                id_token
                    .signing_key(&verifier)
                    .map_err(SsoError::boxed(SsoError::IdToken))?,
            )
            .map_err(SsoError::boxed(SsoError::IdToken))?;
            if actual != *expected {
                return Err(SsoError::IdToken(
                    "the access token does not belong to the ID token".into(),
                ));
            }
        }

        Ok(Identity {
            provider: provider_id.to_owned(),
            subject: claims.subject().to_string(),
            email: claims.email().map(|email| email.to_string()),
            email_verified: claims.email_verified(),
            name: claims
                .name()
                .and_then(|name| name.get(None))
                .map(|name| name.to_string()),
            username: claims
                .preferred_username()
                .map(|username| username.to_string()),
        })
    }
}

async fn build_client(
    config: &OidcConfig,
    http: &reqwest::Client,
) -> Result<DiscoveredClient, SsoError> {
    let metadata =
        CoreProviderMetadata::discover_async(IssuerUrl::from_url(config.issuer_url.clone()), http)
            .await
            .map_err(SsoError::boxed(SsoError::Discovery))?;
    Ok(CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(config.client_id.clone()),
        config
            .client_secret
            .as_ref()
            .map(|secret| ClientSecret::new(secret.expose().to_owned())),
    )
    .set_redirect_uri(RedirectUrl::from_url(config.redirect_url.clone())))
}
