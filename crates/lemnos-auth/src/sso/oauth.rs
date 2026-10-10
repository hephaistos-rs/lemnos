//! Generic OAuth2, for providers that don't speak OpenID Connect (GitHub,
//! Discord, ...). OAuth2 only grants an access token, so the user is looked up
//! by calling the provider's userinfo URL with it and picking fields out of
//! the JSON according to a [`ClaimMapping`].
//!
//! Prefer [OIDC](super::oidc) whenever a provider supports it: it verifies a
//! signed ID token instead of trusting whatever the userinfo URL returns.

use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet,
    EndpointSet, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
    basic::BasicClient,
};
use reqwest::header::ACCEPT;
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use super::{Identity, LoginStart, OAuthCallback, PendingFlow, PendingLogin, Secret, SsoError};

#[derive(Clone, Debug, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<Secret>,
    pub auth_url: Url,
    pub token_url: Url,
    /// Called with the access token to find out who signed in.
    pub userinfo_url: Url,
    /// Where the provider sends the browser back to. Must be registered at
    /// the provider exactly as written here.
    pub redirect_url: Url,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub client_auth: ClientAuth,
    #[serde(default)]
    pub claims: ClaimMapping,
}

/// How the client secret is sent to the token endpoint.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientAuth {
    /// HTTP Basic auth, the OAuth2 default.
    #[default]
    Basic,
    /// `client_id` and `client_secret` form fields, for providers that need it.
    RequestBody,
}

/// Which userinfo fields hold what. Nested fields use dots (`data.id`); a
/// missing field is skipped, except `subject`.
///
/// For GitHub: `subject = "id"`, `username = "login"`.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ClaimMapping {
    /// The provider's stable user ID. Pick an ID that never changes, not a
    /// username or email address the user can edit.
    pub subject: String,
    pub email: String,
    pub email_verified: String,
    pub name: String,
    pub username: String,
}

impl Default for ClaimMapping {
    /// The OIDC standard claim names, which many OAuth2 userinfo URLs reuse.
    fn default() -> Self {
        Self {
            subject: "sub".into(),
            email: "email".into(),
            email_verified: "email_verified".into(),
            name: "name".into(),
            username: "preferred_username".into(),
        }
    }
}

impl ClaimMapping {
    fn identity(&self, provider_id: &str, userinfo: &Value) -> Result<Identity, SsoError> {
        let text = |path: &str| lookup(userinfo, path).and_then(as_text);
        Ok(Identity {
            provider: provider_id.to_owned(),
            subject: text(&self.subject)
                .ok_or_else(|| SsoError::MissingSubject(self.subject.clone()))?,
            email: text(&self.email),
            email_verified: lookup(userinfo, &self.email_verified).and_then(Value::as_bool),
            name: text(&self.name),
            username: text(&self.username),
        })
    }
}

fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |value, key| value.get(key))
}

/// Strings as-is and numbers as text, since some providers (GitHub) use
/// numeric IDs. Empty strings count as missing.
fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

type ConfiguredClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

pub(super) struct OAuthProvider {
    config: OAuthConfig,
    http: reqwest::Client,
    client: ConfiguredClient,
}

impl OAuthProvider {
    pub(super) fn new(config: OAuthConfig, http: reqwest::Client) -> Self {
        let mut client = BasicClient::new(ClientId::new(config.client_id.clone()))
            .set_auth_uri(AuthUrl::from_url(config.auth_url.clone()))
            .set_token_uri(TokenUrl::from_url(config.token_url.clone()))
            .set_redirect_uri(RedirectUrl::from_url(config.redirect_url.clone()))
            .set_auth_type(match config.client_auth {
                ClientAuth::Basic => AuthType::BasicAuth,
                ClientAuth::RequestBody => AuthType::RequestBody,
            });
        if let Some(secret) = &config.client_secret {
            client = client.set_client_secret(ClientSecret::new(secret.expose().to_owned()));
        }
        Self {
            config,
            http,
            client,
        }
    }

    pub(super) fn begin(&self, provider_id: &str) -> Result<LoginStart, SsoError> {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let mut request = self
            .client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge);
        for scope in &self.config.scopes {
            request = request.add_scope(Scope::new(scope.clone()));
        }
        let (redirect_to, state) = request.url();
        Ok(LoginStart {
            redirect_to,
            pending: PendingLogin::new(
                provider_id,
                PendingFlow::OAuth2 {
                    state: state.into_secret(),
                    pkce_verifier: pkce_verifier.into_secret(),
                },
            ),
        })
    }

    pub(super) async fn finish(
        &self,
        provider_id: &str,
        state: &str,
        pkce_verifier: String,
        callback: &OAuthCallback,
    ) -> Result<Identity, SsoError> {
        let code = callback.verify(state)?;
        let token = self
            .client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
            .request_async(&self.http)
            .await
            .map_err(SsoError::boxed(SsoError::TokenExchange))?;

        let userinfo: Value = self
            .http
            .get(self.config.userinfo_url.clone())
            .bearer_auth(token.access_token().secret())
            .header(ACCEPT, "application/json")
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(SsoError::boxed(SsoError::UserInfo))?
            .json()
            .await
            .map_err(SsoError::boxed(SsoError::UserInfo))?;

        self.config.claims.identity(provider_id, &userinfo)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn maps_github_style_userinfo() {
        let mapping = ClaimMapping {
            subject: "id".into(),
            username: "login".into(),
            ..ClaimMapping::default()
        };
        let identity = mapping
            .identity(
                "github",
                &json!({ "id": 583231, "login": "octocat", "name": "The Octocat", "email": null }),
            )
            .unwrap();
        assert_eq!(identity.subject, "583231");
        assert_eq!(identity.username.as_deref(), Some("octocat"));
        assert_eq!(identity.name.as_deref(), Some("The Octocat"));
        assert_eq!(identity.email, None);
        assert_eq!(identity.email_verified, None);
    }

    #[test]
    fn follows_nested_paths() {
        let mapping = ClaimMapping {
            subject: "data.id".into(),
            ..ClaimMapping::default()
        };
        let identity = mapping
            .identity(
                "x",
                &json!({ "data": { "id": "42" }, "email_verified": true }),
            )
            .unwrap();
        assert_eq!(identity.subject, "42");
        assert_eq!(identity.email_verified, Some(true));
    }

    #[test]
    fn requires_a_subject() {
        let result = ClaimMapping::default().identity("x", &json!({ "sub": "" }));
        assert!(matches!(result, Err(SsoError::MissingSubject(field)) if field == "sub"));
    }
}
