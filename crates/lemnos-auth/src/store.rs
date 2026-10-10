//! Where accounts, credentials and sessions are kept.
//!
//! [`Store`] is a trait: a list of operations that [`Auth`](crate::Auth)
//! needs, without saying how they are done. [`MemoryStore`] implements it
//! with plain maps, which is enough for tests and for trying things out. A
//! database-backed store only has to implement the same trait; nothing else
//! in this crate changes.

use std::{
    collections::HashMap,
    error::Error as StdError,
    fmt,
    future::Future,
    sync::{Mutex, MutexGuard, PoisonError},
    time::SystemTime,
};

use crate::user::{NewUser, User, UserId, Username};

/// Identifies a session: the hash of the token in the browser's cookie.
///
/// Only the hash is stored, so someone who can read the store still can't
/// take over anyone's session.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(pub [u8; 32]);

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionId(..)")
    }
}

#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub id: SessionId,
    pub user: UserId,
    pub expires_at: SystemTime,
}

/// A user's TOTP (authenticator app) setup.
#[derive(Clone)]
pub struct TotpRecord {
    /// The shared secret. It has to be stored readable, since every check
    /// recomputes the expected code from it; protect the store accordingly.
    pub secret: Vec<u8>,
    /// `false` until the user has proven their app works by entering a code.
    /// Unconfirmed records never count as 2FA being on.
    pub confirmed: bool,
    /// The newest time step a code was accepted for; see
    /// [`Store::advance_totp_step`].
    pub last_step: Option<u64>,
}

impl fmt::Debug for TotpRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TotpRecord")
            .field("confirmed", &self.confirmed)
            .field("last_step", &self.last_step)
            .finish_non_exhaustive()
    }
}

/// One registered passkey. `data` is opaque to the store.
#[derive(Clone, Debug)]
pub struct PasskeyRecord {
    pub credential_id: Vec<u8>,
    /// The user's own name for it, e.g. "YubiKey" or "Laptop".
    pub label: String,
    /// The public key and counters, serialized by the passkey module.
    pub data: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("it already exists")]
    Conflict,
    #[error("there is no such user")]
    NoSuchUser,
    #[error("the storage backend failed")]
    Backend(#[source] Box<dyn StdError + Send + Sync>),
}

type StoreResult<T> = Result<T, StoreError>;

/// Everything [`Auth`](crate::Auth) needs to remember.
///
/// The methods return `impl Future + Send` instead of being written as
/// `async fn`, which promises callers the futures can move between threads.
/// Implementations can still just write `async fn`.
pub trait Store: Send + Sync + 'static {
    /// Fails with [`StoreError::Conflict`] if the username is taken.
    fn create_user(&self, new: NewUser) -> impl Future<Output = StoreResult<User>> + Send;
    fn user(&self, id: UserId) -> impl Future<Output = StoreResult<Option<User>>> + Send;
    fn user_by_username(
        &self,
        username: &Username,
    ) -> impl Future<Output = StoreResult<Option<User>>> + Send;

    /// `None` for accounts that can't sign in with a password.
    fn password_hash(
        &self,
        user: UserId,
    ) -> impl Future<Output = StoreResult<Option<String>>> + Send;
    fn set_password_hash(
        &self,
        user: UserId,
        hash: Option<String>,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    /// The user linked to an outside identity (SSO or LDAP), if any.
    fn user_by_identity(
        &self,
        provider: &str,
        subject: &str,
    ) -> impl Future<Output = StoreResult<Option<User>>> + Send;
    /// Fails with [`StoreError::Conflict`] if the identity is already linked.
    fn link_identity(
        &self,
        user: UserId,
        provider: &str,
        subject: &str,
    ) -> impl Future<Output = StoreResult<()>> + Send;

    fn insert_session(
        &self,
        session: SessionRecord,
    ) -> impl Future<Output = StoreResult<()>> + Send;
    fn session(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = StoreResult<Option<SessionRecord>>> + Send;
    fn delete_session(&self, id: &SessionId) -> impl Future<Output = StoreResult<()>> + Send;
    fn delete_sessions_of(&self, user: UserId) -> impl Future<Output = StoreResult<()>> + Send;

    fn totp(&self, user: UserId) -> impl Future<Output = StoreResult<Option<TotpRecord>>> + Send;
    fn set_totp(
        &self,
        user: UserId,
        totp: Option<TotpRecord>,
    ) -> impl Future<Output = StoreResult<()>> + Send;
    /// Records `step` as used if it is newer than the last one, and says
    /// whether it was. Must be atomic (one transaction): it is what stops a
    /// TOTP code from being accepted twice.
    fn advance_totp_step(
        &self,
        user: UserId,
        step: u64,
    ) -> impl Future<Output = StoreResult<bool>> + Send;

    fn passkeys(
        &self,
        user: UserId,
    ) -> impl Future<Output = StoreResult<Vec<PasskeyRecord>>> + Send;
    /// Adds the passkey, or replaces the one with the same credential ID.
    fn save_passkey(
        &self,
        user: UserId,
        passkey: PasskeyRecord,
    ) -> impl Future<Output = StoreResult<()>> + Send;
    fn delete_passkey(
        &self,
        user: UserId,
        credential_id: &[u8],
    ) -> impl Future<Output = StoreResult<()>> + Send;
}

/// A [`Store`] that lives in memory and is gone on restart.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    last_id: u64,
    users: HashMap<UserId, Account>,
    identities: HashMap<(String, String), UserId>,
    sessions: HashMap<SessionId, SessionRecord>,
}

struct Account {
    user: User,
    password_hash: Option<String>,
    totp: Option<TotpRecord>,
    passkeys: Vec<PasskeyRecord>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock only means another thread panicked while holding
        // it; the maps themselves are still usable.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Inner {
    fn account(&mut self, user: UserId) -> StoreResult<&mut Account> {
        self.users.get_mut(&user).ok_or(StoreError::NoSuchUser)
    }
}

impl Store for MemoryStore {
    async fn create_user(&self, new: NewUser) -> StoreResult<User> {
        let mut inner = self.lock();
        if inner
            .users
            .values()
            .any(|account| account.user.username == new.username)
        {
            return Err(StoreError::Conflict);
        }
        inner.last_id += 1;
        let user = User {
            id: UserId(inner.last_id),
            name: new.name.unwrap_or_else(|| new.username.to_string()),
            username: new.username,
            email: new.email,
        };
        inner.users.insert(
            user.id,
            Account {
                user: user.clone(),
                password_hash: None,
                totp: None,
                passkeys: Vec::new(),
            },
        );
        Ok(user)
    }

    async fn user(&self, id: UserId) -> StoreResult<Option<User>> {
        Ok(self
            .lock()
            .users
            .get(&id)
            .map(|account| account.user.clone()))
    }

    async fn user_by_username(&self, username: &Username) -> StoreResult<Option<User>> {
        Ok(self
            .lock()
            .users
            .values()
            .find(|account| account.user.username == *username)
            .map(|account| account.user.clone()))
    }

    async fn password_hash(&self, user: UserId) -> StoreResult<Option<String>> {
        Ok(self.lock().account(user)?.password_hash.clone())
    }

    async fn set_password_hash(&self, user: UserId, hash: Option<String>) -> StoreResult<()> {
        self.lock().account(user)?.password_hash = hash;
        Ok(())
    }

    async fn user_by_identity(&self, provider: &str, subject: &str) -> StoreResult<Option<User>> {
        let inner = self.lock();
        Ok(inner
            .identities
            .get(&(provider.to_owned(), subject.to_owned()))
            .and_then(|id| inner.users.get(id))
            .map(|account| account.user.clone()))
    }

    async fn link_identity(&self, user: UserId, provider: &str, subject: &str) -> StoreResult<()> {
        let mut inner = self.lock();
        inner.account(user)?;
        let key = (provider.to_owned(), subject.to_owned());
        if inner.identities.contains_key(&key) {
            return Err(StoreError::Conflict);
        }
        inner.identities.insert(key, user);
        Ok(())
    }

    async fn insert_session(&self, session: SessionRecord) -> StoreResult<()> {
        self.lock().sessions.insert(session.id, session);
        Ok(())
    }

    async fn session(&self, id: &SessionId) -> StoreResult<Option<SessionRecord>> {
        Ok(self.lock().sessions.get(id).cloned())
    }

    async fn delete_session(&self, id: &SessionId) -> StoreResult<()> {
        self.lock().sessions.remove(id);
        Ok(())
    }

    async fn delete_sessions_of(&self, user: UserId) -> StoreResult<()> {
        self.lock()
            .sessions
            .retain(|_, session| session.user != user);
        Ok(())
    }

    async fn totp(&self, user: UserId) -> StoreResult<Option<TotpRecord>> {
        Ok(self.lock().account(user)?.totp.clone())
    }

    async fn set_totp(&self, user: UserId, totp: Option<TotpRecord>) -> StoreResult<()> {
        self.lock().account(user)?.totp = totp;
        Ok(())
    }

    async fn advance_totp_step(&self, user: UserId, step: u64) -> StoreResult<bool> {
        let mut inner = self.lock();
        let Some(totp) = &mut inner.account(user)?.totp else {
            return Ok(false);
        };
        if totp.last_step.is_some_and(|last| step <= last) {
            return Ok(false);
        }
        totp.last_step = Some(step);
        Ok(true)
    }

    async fn passkeys(&self, user: UserId) -> StoreResult<Vec<PasskeyRecord>> {
        Ok(self.lock().account(user)?.passkeys.clone())
    }

    async fn save_passkey(&self, user: UserId, passkey: PasskeyRecord) -> StoreResult<()> {
        let mut inner = self.lock();
        let passkeys = &mut inner.account(user)?.passkeys;
        match passkeys
            .iter_mut()
            .find(|existing| existing.credential_id == passkey.credential_id)
        {
            Some(existing) => *existing = passkey,
            None => passkeys.push(passkey),
        }
        Ok(())
    }

    async fn delete_passkey(&self, user: UserId, credential_id: &[u8]) -> StoreResult<()> {
        self.lock()
            .account(user)?
            .passkeys
            .retain(|passkey| passkey.credential_id != credential_id);
        Ok(())
    }
}
