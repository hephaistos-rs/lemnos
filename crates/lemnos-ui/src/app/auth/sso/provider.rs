// The three stops of an SSO sign-in, for the provider named in the URL:
//
//   GET  /auth/sso/{provider}/start     send the browser to the provider
//   GET  /auth/sso/{provider}/callback  OIDC/OAuth2: the provider sends it back
//   POST /auth/sso/{provider}/acs       SAML: the provider posts its answer

use lemnos_auth::sso::{Callback, OAuthCallback, PendingLogin, SamlCallback, SsoError};
use topcoat::{
    Result,
    context::Cx,
    cookie::SameSite,
    router::{
        content::Form,
        error::{SeeOther, not_found, see_other},
        href, module_param, path_param, query_params, route,
    },
};

use crate::app::auth::{error_code, sign_in, start_session, stash, state, take};

module_param!(provider);

/// Cookie remembering the sign-in that is under way.
const PENDING: &str = "sso-pending";

// SAML answers arrive as a POST from the provider's site. Browsers only
// attach cookies to such cross-site POSTs when they are `SameSite=None`.
const PENDING_SAME_SITE: SameSite = SameSite::None;

#[route(GET "./start")]
pub async fn start(cx: &Cx) -> Result<SeeOther> {
    let provider = path_param::<Provider>(cx);
    let start = match state(cx).sso.begin(provider).await {
        Err(SsoError::UnknownProvider(_)) => return Err(not_found().into()),
        other => other?,
    };
    stash(cx, PENDING, PENDING_SAME_SITE, start.pending)?;
    Ok(see_other(start.redirect_to.as_str()))
}

#[query_params(error = bad_request)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[route(GET "./callback")]
pub async fn oauth_callback(cx: &Cx) -> Result<SeeOther> {
    let query = query_params::<CallbackQuery>(cx)?;
    let callback = OAuthCallback {
        code: query.code.clone(),
        state: query.state.clone(),
        error: query.error.clone(),
        error_description: query.error_description.clone(),
    };
    finish(cx, Callback::OAuth(callback)).await
}

// Exempt from the router's cross-origin check (see `app.rs`): this request
// comes from the provider's site by design, and the signed response inside
// it is what gets verified.
#[route(POST "./acs")]
pub async fn saml_acs(cx: &Cx, Form(response): Form<SamlCallback>) -> Result<SeeOther> {
    finish(cx, Callback::Saml(response)).await
}

async fn finish(cx: &Cx, callback: Callback) -> Result<SeeOther> {
    let provider = path_param::<Provider>(cx);
    let state = state(cx);
    let Some(pending) = take::<PendingLogin>(cx, PENDING, PENDING_SAME_SITE) else {
        return Ok(see_other(sign_in::failed(cx, "expired")));
    };
    let identity = match state.sso.finish(provider, pending, callback).await {
        Ok(identity) => identity,
        Err(error) => {
            eprintln!("SSO sign-in with `{provider}` failed: {error:?}");
            return Ok(see_other(sign_in::failed(cx, "sso")));
        }
    };
    Ok(see_other(
        match state
            .auth
            .authenticate_identity(&identity, state.unknown_identity(provider))
            .await
        {
            Ok(authenticated) => {
                start_session(cx, authenticated).await?;
                href!(crate::app::home).resolve(cx).to_string()
            }
            Err(error) => sign_in::failed(cx, error_code(&error)),
        },
    ))
}
