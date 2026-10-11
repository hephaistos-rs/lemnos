//! [`Auth`]: accounts, sign-in and sessions, on top of a [`Store`].
//!
//! Every way of signing in ends in the same place, an [`Authenticated`]
//! value, and only that can start a session:
//!
//! ```text
//! login_password ─┐                       ┌─ Login::Complete ───────────┐
//! login_ldap ─────┼─► Login ──────────────┤                             ├─► Authenticated ─► start_session
//!                 │                       └─ Login::SecondFactor ─► verify_totp
//! finish_passkey_login ───────────────────────────────────────────────►─┤
//! authenticate_identity (SSO) ────────────────────────────────────────►─┘
//! ```
//!
//! The other sign-in methods live next to their own code: [`crate::totp`],
//! `crate::ldap` and `crate::passkey` each add methods to [`Auth`].

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::{
    identity::Identity,
    password::{self, PasswordPolicy, WeakPassword},
    store::{SessionId, SessionRecord, Store, StoreError},
    throttle::Throttle,
    unix_now,
    user::{InvalidUsername, NewUser, User, UserId, Username},
};

/// How long a half-finished sign-in (waiting for a code or a passkey) stays
/// valid.
pub(crate) const STEP_MAX_AGE: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// Shown in authenticator apps next to the account name.
    pub issuer: String,
    /// Sessions end this long after sign-in, whether used or not.
    pub session_lifetime: Duration,
    pub password_policy: PasswordPolicy,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            issuer: "Lemnos".into(),
            session_lifetime: Duration::from_secs(24 * 60 * 60),
            password_policy: PasswordPolicy::default(),
        }
    }
}

/// Accounts, sign-in and sessions. Build one at startup and share it.
///
/// `S` is a generic parameter: `Auth` works with any type that implements
/// [`Store`], and the compiler generates a version specialised for the one
/// actually used (`Auth<MemoryStore>` today, a database store later).
pub struct Auth<S> {
    pub(crate) store: S,
    pub(crate) config: AuthConfig,
    pub(crate) throttle: Throttle,
    #[cfg(feature = "ldap")]
    pub(crate) ldap: Option<crate::ldap::LdapDirectory>,
    #[cfg(feature = "passkeys")]
    pub(crate) passkeys: Option<crate::passkey::Passkeys>,
}

/// Proof that someone completed every sign-in step required of them.
///
/// The fields are private and there is no public constructor, so code
/// outside this crate can't make one up: the only way to get an
/// `Authenticated` is to actually sign in. It is also not `Clone`, and
/// [`Auth::start_session`] takes it by value, so one sign-in starts exactly
/// one session.
#[derive(Debug)]
pub struct Authenticated {
    pub(crate) user: User,
    pub(crate) method: Method,
    pub(crate) second_factor: bool,
}

impl Authenticated {
    pub fn user(&self) -> &User {
        &self.user
    }

    pub fn method(&self) -> Method {
        self.method
    }

    /// Whether a TOTP code was checked on top of [`method`](Self::method).
    pub fn used_second_factor(&self) -> bool {
        self.second_factor
    }
}

/// How the user proved who they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    Password,
    Ldap,
    Passkey,
    Sso,
}

/// The result of checking a password. An enum makes the caller deal with
/// both cases: there is no way to reach the [`Authenticated`] inside
/// `Complete` without also writing what happens for `SecondFactor`.
#[derive(Debug)]
pub enum Login {
    Complete(Authenticated),
    /// The password was right, but the user has 2FA on. Ask for a code and
    /// pass it to [`Auth::verify_totp`].
    SecondFactor(SecondFactorChallenge),
}

/// Remembers that a user got their password right while Lemnos waits for
/// their code.
///
/// Whoever holds this has passed the first factor, so keep it server-side or
/// in an encrypted cookie, like [`PendingLogin`](crate::sso::PendingLogin).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecondFactorChallenge {
    pub(crate) user: UserId,
    pub(crate) method: Method,
    pub(crate) issued_at: u64,
}

impl SecondFactorChallenge {
    pub(crate) fn is_expired(&self) -> bool {
        unix_now().saturating_sub(self.issued_at) > STEP_MAX_AGE.as_secs()
    }
}

/// What to do when an outside identity has no Lemnos account yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownIdentity {
    /// Only users an admin has linked beforehand may sign in.
    Reject,
    /// Create an account on first sign-in. Right for your own directory or
    /// identity provider; wrong for public ones like GitHub, where it would
    /// let anyone in.
    CreateAccount,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Deliberately the same for "no such user" and "wrong password".
    #[error("wrong username or password")]
    InvalidCredentials,
    #[error("too many failed attempts; try again in {} seconds", .0.as_secs().max(1))]
    Throttled(Duration),
    #[error("that code is wrong or was already used")]
    InvalidCode,
    #[error("this sign-in step took too long; start again")]
    Expired,
    #[error("no Lemnos account is linked to this identity")]
    NoLinkedAccount,
    #[error("the username `{0}` is already taken")]
    UsernameTaken(Username),
    #[error("two-factor authentication is already set up; disable it first")]
    TotpAlreadyEnabled,
    #[error("two-factor authentication has not been set up")]
    TotpNotSetUp,
    #[error(transparent)]
    WeakPassword(#[from] WeakPassword),
    #[error(transparent)]
    InvalidUsername(#[from] InvalidUsername),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("{0} sign-in is not configured")]
    NotConfigured(&'static str),
    #[cfg(feature = "ldap")]
    #[error(transparent)]
    Ldap(#[from] crate::ldap::LdapError),
    #[cfg(feature = "passkeys")]
    #[error(transparent)]
    Passkey(#[from] crate::passkey::PasskeyError),
    #[error("internal error: {0}")]
    Internal(String),
}

impl<S: Store> Auth<S> {
    pub fn new(store: S, config: AuthConfig) -> Self {
        Self {
            store,
            config,
            throttle: Throttle::default(),
            #[cfg(feature = "ldap")]
            ldap: None,
            #[cfg(feature = "passkeys")]
            passkeys: None,
        }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// The rules [`set_password`](Self::set_password) enforces, for checking
    /// a password before creating the account it is for.
    pub fn password_policy(&self) -> &PasswordPolicy {
        &self.config.password_policy
    }

    /// Creates an account that can't sign in yet; follow up with
    /// [`set_password`](Self::set_password) or link an identity.
    pub async fn create_user(&self, new: NewUser) -> Result<User, AuthError> {
        let username = new.username.clone();
        self.store
            .create_user(new)
            .await
            .map_err(|error| match error {
                StoreError::Conflict => AuthError::UsernameTaken(username),
                other => other.into(),
            })
    }

    /// Sets a new password and signs the user out everywhere, so a password
    /// change after a compromise also gets rid of the intruder's sessions.
    pub async fn set_password(&self, user: UserId, password: &str) -> Result<(), AuthError> {
        self.config.password_policy.check(password)?;
        let hash = password::hash(password.to_owned())
            .await
            .ok_or_else(|| AuthError::Internal("hashing the password failed".into()))?;
        self.store.set_password_hash(user, Some(hash)).await?;
        self.store.delete_sessions_of(user).await?;
        Ok(())
    }

    /// Checks a local account's password.
    pub async fn login_password(&self, username: &str, password: &str) -> Result<Login, AuthError> {
        let Ok(username) = Username::parse(username) else {
            return Err(AuthError::InvalidCredentials);
        };
        let key = login_key(username.as_str());
        self.throttle.check(&key).map_err(AuthError::Throttled)?;

        let user = self.store.user_by_username(&username).await?;
        let hash = match &user {
            Some(user) => self.store.password_hash(user.id).await?,
            None => None,
        };
        // Runs even for unknown users, so the response time doesn't reveal
        // which usernames exist.
        let verified = password::verify(password.to_owned(), hash).await;

        match user {
            Some(user) if verified => {
                self.throttle.success(&key);
                self.after_first_factor(user, Method::Password).await
            }
            _ => {
                self.throttle.failure(&key);
                Err(AuthError::InvalidCredentials)
            }
        }
    }

    /// Decides whether a correct password (or LDAP bind) is enough.
    pub(crate) async fn after_first_factor(
        &self,
        user: User,
        method: Method,
    ) -> Result<Login, AuthError> {
        let has_totp = self
            .store
            .totp(user.id)
            .await?
            .is_some_and(|totp| totp.confirmed);
        Ok(if has_totp {
            Login::SecondFactor(SecondFactorChallenge {
                user: user.id,
                method,
                issued_at: unix_now(),
            })
        } else {
            Login::Complete(Authenticated {
                user,
                method,
                second_factor: false,
            })
        })
    }

    /// Signs in the Lemnos account behind an SSO [`Identity`] (the result of
    /// [`Sso::finish`](crate::sso::Sso::finish)). No TOTP step: second
    /// factors for SSO users are the identity provider's job.
    pub async fn authenticate_identity(
        &self,
        identity: &Identity,
        unknown: UnknownIdentity,
    ) -> Result<Authenticated, AuthError> {
        Ok(Authenticated {
            user: self.resolve_identity(identity, unknown).await?,
            method: Method::Sso,
            second_factor: false,
        })
    }

    /// Lets `user` sign in with `identity` from now on.
    pub async fn link_identity(&self, user: UserId, identity: &Identity) -> Result<(), AuthError> {
        self.store
            .link_identity(user, &identity.provider, &identity.subject)
            .await?;
        Ok(())
    }

    pub(crate) async fn resolve_identity(
        &self,
        identity: &Identity,
        unknown: UnknownIdentity,
    ) -> Result<User, AuthError> {
        if let Some(user) = self
            .store
            .user_by_identity(&identity.provider, &identity.subject)
            .await?
        {
            return Ok(user);
        }
        if unknown == UnknownIdentity::Reject {
            return Err(AuthError::NoLinkedAccount);
        }
        // A new identity always gets a new account. If the name is taken
        // this fails rather than attaching to the existing account: the
        // provider only vouches for its own user, not for ours with the
        // same name.
        let name = identity
            .username
            .as_deref()
            .or(identity.email.as_deref())
            .ok_or(InvalidUsername::Empty)?;
        let user = self
            .create_user(NewUser {
                username: Username::parse(name)?,
                name: identity.name.clone(),
                email: identity.email.clone(),
            })
            .await?;
        self.link_identity(user.id, identity).await?;
        Ok(user)
    }

    /// Starts a session for a completed sign-in. `id` is the hash of the
    /// token the web layer put in the browser's cookie.
    pub async fn start_session(
        &self,
        authenticated: Authenticated,
        id: SessionId,
    ) -> Result<User, AuthError> {
        self.store
            .insert_session(SessionRecord {
                id,
                user: authenticated.user.id,
                expires_at: SystemTime::now() + self.config.session_lifetime,
            })
            .await?;
        Ok(authenticated.user)
    }

    /// The user signed in on session `id`, if it exists and hasn't expired.
    pub async fn session_user(&self, id: &SessionId) -> Result<Option<User>, AuthError> {
        let Some(session) = self.store.session(id).await? else {
            return Ok(None);
        };
        if session.expires_at <= SystemTime::now() {
            self.store.delete_session(id).await?;
            return Ok(None);
        }
        Ok(self.store.user(session.user).await?)
    }

    pub async fn end_session(&self, id: &SessionId) -> Result<(), AuthError> {
        Ok(self.store.delete_session(id).await?)
    }

    /// Signs the user out everywhere.
    pub async fn end_all_sessions(&self, user: UserId) -> Result<(), AuthError> {
        Ok(self.store.delete_sessions_of(user).await?)
    }
}

/// Password and LDAP sign-ins share one failure count per username.
pub(crate) fn login_key(username: &str) -> String {
    format!("login:{username}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;

    const PASSWORD: &str = "correct horse battery";

    async fn auth_with_alice() -> (Auth<MemoryStore>, User) {
        let auth = Auth::new(MemoryStore::new(), AuthConfig::default());
        let alice = auth
            .create_user(NewUser::new("alice".parse().unwrap()))
            .await
            .unwrap();
        auth.set_password(alice.id, PASSWORD).await.unwrap();
        (auth, alice)
    }

    fn identity(subject: &str, username: &str) -> Identity {
        Identity {
            provider: "keycloak".into(),
            subject: subject.into(),
            email: None,
            email_verified: None,
            name: None,
            username: Some(username.into()),
        }
    }

    #[tokio::test]
    async fn password_sign_in_and_session_lifecycle() {
        let (auth, alice) = auth_with_alice().await;
        // Usernames are normalised, so this is still alice.
        let Login::Complete(authenticated) = auth.login_password(" Alice", PASSWORD).await.unwrap()
        else {
            panic!("no second factor is set up");
        };
        assert_eq!(authenticated.method(), Method::Password);

        let session = SessionId([7; 32]);
        auth.start_session(authenticated, session).await.unwrap();
        assert_eq!(auth.session_user(&session).await.unwrap(), Some(alice));
        auth.end_session(&session).await.unwrap();
        assert_eq!(auth.session_user(&session).await.unwrap(), None);
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_user_look_the_same() {
        let (auth, _) = auth_with_alice().await;
        assert!(matches!(
            auth.login_password("alice", "wrong password!").await,
            Err(AuthError::InvalidCredentials)
        ));
        assert!(matches!(
            auth.login_password("nobody", PASSWORD).await,
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn repeated_failures_are_throttled() {
        let (auth, _) = auth_with_alice().await;
        for _ in 0..6 {
            let _ = auth.login_password("alice", "wrong password!").await;
        }
        // Even the right password has to wait now.
        assert!(matches!(
            auth.login_password("alice", PASSWORD).await,
            Err(AuthError::Throttled(_))
        ));
    }

    #[tokio::test]
    async fn expired_sessions_are_rejected() {
        let (auth, alice) = auth_with_alice().await;
        let session = SessionId([1; 32]);
        auth.store
            .insert_session(SessionRecord {
                id: session,
                user: alice.id,
                expires_at: SystemTime::now() - Duration::from_secs(1),
            })
            .await
            .unwrap();
        assert_eq!(auth.session_user(&session).await.unwrap(), None);
    }

    #[tokio::test]
    async fn changing_the_password_ends_sessions_and_enforces_policy() {
        let (auth, alice) = auth_with_alice().await;
        let Login::Complete(authenticated) = auth.login_password("alice", PASSWORD).await.unwrap()
        else {
            panic!("no second factor is set up");
        };
        let session = SessionId([2; 32]);
        auth.start_session(authenticated, session).await.unwrap();

        assert!(matches!(
            auth.set_password(alice.id, "short").await,
            Err(AuthError::WeakPassword(_))
        ));
        auth.set_password(alice.id, "a different password")
            .await
            .unwrap();
        assert_eq!(auth.session_user(&session).await.unwrap(), None);
    }

    #[tokio::test]
    async fn identities_sign_in_only_when_linked_or_allowed() {
        let (auth, alice) = auth_with_alice().await;

        let stranger = identity("sub-1", "bob");
        assert!(matches!(
            auth.authenticate_identity(&stranger, UnknownIdentity::Reject)
                .await,
            Err(AuthError::NoLinkedAccount)
        ));

        let created = auth
            .authenticate_identity(&stranger, UnknownIdentity::CreateAccount)
            .await
            .unwrap();
        assert_eq!(created.user().username.as_str(), "bob");
        // The second time it is the same account, not a new one.
        let again = auth
            .authenticate_identity(&stranger, UnknownIdentity::Reject)
            .await
            .unwrap();
        assert_eq!(again.user().id, created.user().id);

        // Someone at the provider who happens to be called "alice" does not
        // get the local alice's account.
        let impostor = identity("sub-2", "alice");
        assert!(matches!(
            auth.authenticate_identity(&impostor, UnknownIdentity::CreateAccount)
                .await,
            Err(AuthError::UsernameTaken(_))
        ));

        // Unless an admin links them on purpose.
        auth.link_identity(alice.id, &impostor).await.unwrap();
        let linked = auth
            .authenticate_identity(&impostor, UnknownIdentity::Reject)
            .await
            .unwrap();
        assert_eq!(linked.user().id, alice.id);
    }
}
