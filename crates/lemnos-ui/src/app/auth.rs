// /auth/*: signing in to Lemnos. The real logic lives in `lemnos-auth`; these
// pages only turn requests into calls to it.

use std::{error::Error, time::SystemTime};

use lemnos_auth::{Auth, AuthError, Authenticated, SessionId, UnknownIdentity, User, sso::Sso};
use lemnos_db::Database;
use serde::{Serialize, de::DeserializeOwned};
use topcoat::{
    Result,
    context::{Cx, app_context},
    cookie::{Cookies, SameSite, cookie_store, private_cookies, time::Duration},
    session,
};

use crate::config::Config;

pub mod passkey;
pub mod sign_in;
pub mod sign_out;
pub mod sign_up;
pub mod sso;
pub mod two_factor;

/// Everything the auth pages share. Registered on the router as app context.
pub struct AuthState {
    pub auth: Auth<Database>,
    pub sso: Sso,
    pub allow_sign_up: bool,
    pub ldap: bool,
    pub passkeys: bool,
    auto_provision: Vec<String>,
}

impl AuthState {
    pub async fn from_config(config: Config) -> Result<Self, Box<dyn Error>> {
        let database = Database::connect(config.database_url.expose()).await?;
        // Nobody can use these any more; this just tidies up after the last run.
        database.delete_expired_sessions(SystemTime::now()).await?;
        let mut auth = Auth::new(database, Default::default());
        let ldap = config.ldap.is_some();
        if let Some(ldap) = config.ldap {
            auth = auth.with_ldap(ldap)?;
        }
        let passkeys = config.passkeys.is_some();
        if let Some(passkeys) = config.passkeys {
            auth = auth.with_passkeys(passkeys)?;
        }
        Ok(Self {
            auth,
            // Contacts every provider, so one that is down stops startup.
            sso: Sso::from_config(config.sso).await?,
            allow_sign_up: config.allow_sign_up,
            ldap,
            passkeys,
            auto_provision: config.auto_provision,
        })
    }

    /// What to do with a user of `provider` who has no Lemnos account yet.
    pub fn unknown_identity(&self, provider: &str) -> UnknownIdentity {
        if self.auto_provision.iter().any(|id| id == provider) {
            UnknownIdentity::CreateAccount
        } else {
            UnknownIdentity::Reject
        }
    }
}

pub fn state(cx: &Cx) -> &AuthState {
    app_context(cx)
}

/// The user signed in on this request, if any.
pub async fn current_user(cx: &Cx) -> Result<Option<User>> {
    let Some(hash) = session::token_hash(cx).await? else {
        return Ok(None);
    };
    Ok(state(cx).auth.session_user(&SessionId(*hash)).await?)
}

/// Turns a completed sign-in into a session: Topcoat puts a fresh token in
/// the browser's cookie, and `lemnos-auth` records its hash for the user.
pub async fn start_session(cx: &Cx, authenticated: Authenticated) -> Result<User> {
    let session = session::start(cx).await?;
    Ok(state(cx)
        .auth
        .start_session(authenticated, SessionId(*session.token_hash))
        .await?)
}

/// A short code for the sign-in page's `?error=` parameter. Unexpected
/// errors are logged here and shown to the user only as "unavailable".
pub fn error_code(error: &AuthError) -> &'static str {
    match error {
        AuthError::InvalidCredentials => "invalid",
        AuthError::Throttled(_) => "throttled",
        AuthError::InvalidCode => "code",
        AuthError::Expired => "expired",
        AuthError::NoLinkedAccount => "unlinked",
        AuthError::UsernameTaken(_) => "taken",
        AuthError::InvalidUsername(_) => "username",
        AuthError::WeakPassword(_) => "weak",
        other => {
            eprintln!("sign-in failed: {other:?}");
            "unavailable"
        }
    }
}

pub fn error_message(code: &str) -> &'static str {
    match code {
        "invalid" => "Wrong username or password.",
        "throttled" => "Too many failed attempts. Wait a moment and try again.",
        "code" => "That code is wrong or was already used.",
        "expired" => "That took too long. Please start again.",
        "unlinked" => "No Lemnos account is linked to that sign-in.",
        "taken" => "That username is already taken.",
        "username" => "Usernames can only contain letters, digits and . _ - @",
        "weak" => "The password must be between 12 and 128 characters.",
        "sso" => "The identity provider's answer could not be verified.",
        "closed" => "Creating accounts is turned off.",
        _ => "Signing in is not possible right now.",
    }
}

// Values that must survive from one request to the next while a sign-in is
// half done (waiting for the identity provider, a code, or a passkey). They
// go into encrypted cookies: the browser carries them but can't read or
// change them.

fn flow_cookies(cx: &Cx, same_site: SameSite) -> impl Cookies {
    private_cookies(cx)
        .default_path("/auth")
        .default_http_only(true)
        .default_secure(true)
        .default_same_site(same_site)
        .default_max_age(Duration::minutes(10))
}

pub fn stash<T>(cx: &Cx, name: &'static str, same_site: SameSite, value: T) -> Result<()>
where
    T: Serialize + DeserializeOwned,
{
    cookie_store::<T, _>(flow_cookies(cx, same_site), name)
        .set(value)
        .commit()?;
    Ok(())
}

/// Reads a stashed value without removing it.
pub fn peek<T>(cx: &Cx, name: &'static str, same_site: SameSite) -> Option<T>
where
    T: Serialize + DeserializeOwned + Clone,
{
    let store = cookie_store::<T, _>(flow_cookies(cx, same_site), name)
        .parse()
        .ok()??;
    Some(store.get())
}

/// Reads a stashed value and removes it, so it can be used only once.
pub fn take<T>(cx: &Cx, name: &'static str, same_site: SameSite) -> Option<T>
where
    T: Serialize + DeserializeOwned + Clone,
{
    let store = cookie_store::<T, _>(flow_cookies(cx, same_site), name)
        .parse()
        .ok()??;
    let value = store.get();
    store.remove();
    Some(value)
}
