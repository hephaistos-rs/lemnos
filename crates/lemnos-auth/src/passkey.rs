//! Passkeys (WebAuthn), behind the `passkeys` feature.
//!
//! A passkey is a key pair made by the user's device (phone, laptop,
//! security key). Lemnos only ever stores the public half, so there is
//! nothing here worth stealing, and the browser ties each passkey to this
//! site's domain, so it can't be phished. Because the device checks a
//! fingerprint or PIN itself, a passkey sign-in counts as complete: no
//! password and no TOTP code.
//!
//! Both registering and signing in are a "ceremony" of two calls with the
//! browser in between:
//!
//! 1. `begin_*` returns a challenge to send to the browser as JSON (it goes
//!    into `navigator.credentials.create()` / `.get()`), plus a state value
//!    to keep on the server side.
//! 2. `finish_*` takes that state and the browser's JSON answer.
//!
//! The state values must not be readable or changeable by the browser; keep
//! them server-side or in an encrypted cookie.
//!
//! Built on a pre-release of `webauthn-rs` (see `Cargo.toml` for why).

use serde::{Deserialize, Serialize};
use url::Url;
use webauthn_rs::{
    Webauthn, WebauthnBuilder,
    prelude::{Passkey, PasskeyAuthentication, PasskeyRegistration, Uuid, WebauthnError},
};

// Re-exported so the web layer can (de)serialize the browser's messages
// without depending on `webauthn-rs` itself.
pub use webauthn_rs::prelude::{
    CreationChallengeResponse, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse,
};

use crate::{
    auth::{Auth, AuthError, Authenticated, Method, STEP_MAX_AGE},
    store::{PasskeyRecord, Store},
    unix_now,
    user::{User, UserId, Username},
};

#[derive(Clone, Debug, Deserialize)]
pub struct PasskeyConfig {
    /// The site's domain, without scheme or port: `lemnos.example.com`.
    /// Passkeys are bound to it, so changing it later invalidates them all.
    pub rp_id: String,
    /// The URL users see in their browser: `https://lemnos.example.com`.
    /// Browsers only allow passkeys on HTTPS (and on `http://localhost`).
    pub origin: Url,
    /// Shown by the browser when it asks to create a passkey.
    #[serde(default = "default_rp_name")]
    pub rp_name: String,
}

fn default_rp_name() -> String {
    "Lemnos".into()
}

#[derive(Debug, thiserror::Error)]
pub enum PasskeyError {
    #[error("this account has no passkeys")]
    NoPasskeys,
    #[error("the passkey could not be verified")]
    Webauthn(#[from] WebauthnError),
    #[error("a stored passkey could not be read")]
    Corrupt(#[from] serde_json::Error),
}

/// Kept between [`Auth::begin_passkey_registration`] and
/// [`Auth::finish_passkey_registration`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasskeyRegistrationState {
    user: UserId,
    issued_at: u64,
    state: PasskeyRegistration,
}

impl PasskeyRegistrationState {
    /// The user this registration was started for.
    pub fn user(&self) -> UserId {
        self.user
    }
}

/// Kept between [`Auth::begin_passkey_login`] and
/// [`Auth::finish_passkey_login`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasskeyLoginState {
    user: UserId,
    issued_at: u64,
    state: PasskeyAuthentication,
}

/// One of a user's passkeys, for listing them in account settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasskeyInfo {
    pub credential_id: Vec<u8>,
    pub label: String,
}

pub(crate) struct Passkeys {
    webauthn: Webauthn,
}

impl<S: Store> Auth<S> {
    /// Turns on passkeys.
    pub fn with_passkeys(mut self, config: PasskeyConfig) -> Result<Self, PasskeyError> {
        let webauthn = WebauthnBuilder::new(&config.rp_id, &config.origin)?
            .rp_name(&config.rp_name)
            .build()?;
        self.passkeys = Some(Passkeys { webauthn });
        Ok(self)
    }

    /// Starts adding a passkey to `user`'s account. Only call this for a
    /// user who is signed in.
    pub async fn begin_passkey_registration(
        &self,
        user: &User,
    ) -> Result<(CreationChallengeResponse, PasskeyRegistrationState), AuthError> {
        // Telling the browser which passkeys exist stops the same device
        // from being registered twice.
        let existing = self
            .stored_passkeys(user.id)
            .await?
            .iter()
            .map(|(_, passkey)| passkey.cred_id().clone())
            .collect();
        let (challenge, state) = self
            .webauthn()?
            .start_passkey_registration(
                user_handle(user.id),
                user.username.as_str(),
                &user.name,
                Some(existing),
            )
            .map_err(PasskeyError::from)?;
        Ok((
            challenge,
            PasskeyRegistrationState {
                user: user.id,
                issued_at: unix_now(),
                state,
            },
        ))
    }

    /// Verifies the browser's answer and saves the new passkey under `label`.
    pub async fn finish_passkey_registration(
        &self,
        state: PasskeyRegistrationState,
        response: &RegisterPublicKeyCredential,
        label: &str,
    ) -> Result<PasskeyInfo, AuthError> {
        if is_expired(state.issued_at) {
            return Err(AuthError::Expired);
        }
        let passkey = self
            .webauthn()?
            .finish_passkey_registration(response, &state.state)
            .map_err(PasskeyError::from)?;
        let label = match label.trim() {
            "" => "Passkey".to_owned(),
            label => label.chars().take(64).collect(),
        };
        let record = to_record(&passkey, label)?;
        let info = PasskeyInfo {
            credential_id: record.credential_id.clone(),
            label: record.label.clone(),
        };
        self.store.save_passkey(state.user, record).await?;
        Ok(info)
    }

    /// Starts a passkey sign-in for the account called `username`.
    ///
    /// Fails if the account doesn't exist or has no passkeys. That does let
    /// a visitor find out whether a username has passkeys; show the same
    /// message for every failure to keep it from being obvious.
    pub async fn begin_passkey_login(
        &self,
        username: &str,
    ) -> Result<(RequestChallengeResponse, PasskeyLoginState), AuthError> {
        let webauthn = self.webauthn()?;
        let user = match Username::parse(username) {
            Ok(username) => self.store.user_by_username(&username).await?,
            Err(_) => None,
        }
        .ok_or(PasskeyError::NoPasskeys)?;
        let passkeys: Vec<Passkey> = self
            .stored_passkeys(user.id)
            .await?
            .into_iter()
            .map(|(_, passkey)| passkey)
            .collect();
        if passkeys.is_empty() {
            return Err(PasskeyError::NoPasskeys.into());
        }
        let (challenge, state) = webauthn
            .start_passkey_authentication(&passkeys)
            .map_err(PasskeyError::from)?;
        Ok((
            challenge,
            PasskeyLoginState {
                user: user.id,
                issued_at: unix_now(),
                state,
            },
        ))
    }

    /// Verifies the browser's answer. On success the sign-in is complete.
    pub async fn finish_passkey_login(
        &self,
        state: PasskeyLoginState,
        response: &PublicKeyCredential,
    ) -> Result<Authenticated, AuthError> {
        if is_expired(state.issued_at) {
            return Err(AuthError::Expired);
        }
        let result = self
            .webauthn()?
            .finish_passkey_authentication(response, &state.state)
            .map_err(PasskeyError::from)?;

        // Devices count their uses, which lets a cloned key be noticed. Save
        // the new count on the passkey that was just used.
        for (record, mut passkey) in self.stored_passkeys(state.user).await? {
            if passkey.update_credential(&result) == Some(true) {
                self.store
                    .save_passkey(state.user, to_record(&passkey, record.label)?)
                    .await?;
            }
        }

        let user = self
            .store
            .user(state.user)
            .await?
            .ok_or(AuthError::InvalidCredentials)?;
        Ok(Authenticated {
            user,
            method: Method::Passkey,
            second_factor: false,
        })
    }

    /// The user's passkeys, for showing in their account settings.
    pub async fn passkeys(&self, user: UserId) -> Result<Vec<PasskeyInfo>, AuthError> {
        Ok(self
            .store
            .passkeys(user)
            .await?
            .into_iter()
            .map(|record| PasskeyInfo {
                credential_id: record.credential_id,
                label: record.label,
            })
            .collect())
    }

    pub async fn remove_passkey(
        &self,
        user: UserId,
        credential_id: &[u8],
    ) -> Result<(), AuthError> {
        Ok(self.store.delete_passkey(user, credential_id).await?)
    }

    fn webauthn(&self) -> Result<&Webauthn, AuthError> {
        self.passkeys
            .as_ref()
            .map(|passkeys| &passkeys.webauthn)
            .ok_or(AuthError::NotConfigured("passkey"))
    }

    async fn stored_passkeys(
        &self,
        user: UserId,
    ) -> Result<Vec<(PasskeyRecord, Passkey)>, AuthError> {
        self.store
            .passkeys(user)
            .await?
            .into_iter()
            .map(|record| {
                let passkey = serde_json::from_str(&record.data).map_err(PasskeyError::from)?;
                Ok((record, passkey))
            })
            .collect()
    }
}

fn to_record(passkey: &Passkey, label: String) -> Result<PasskeyRecord, PasskeyError> {
    Ok(PasskeyRecord {
        credential_id: passkey.cred_id().to_vec(),
        label,
        data: serde_json::to_string(passkey)?,
    })
}

/// WebAuthn wants a UUID per user. Deriving it from the ID means there is
/// nothing extra to store.
fn user_handle(user: UserId) -> Uuid {
    Uuid::from_u128(u128::from(user.0))
}

fn is_expired(issued_at: u64) -> bool {
    unix_now().saturating_sub(issued_at) > STEP_MAX_AGE.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuthConfig, MemoryStore, NewUser};

    fn config() -> PasskeyConfig {
        serde_json::from_value(serde_json::json!({
            "rp_id": "lemnos.example.com",
            "origin": "https://lemnos.example.com",
        }))
        .unwrap()
    }

    async fn auth_with_alice() -> (Auth<MemoryStore>, User) {
        let auth = Auth::new(MemoryStore::new(), AuthConfig::default())
            .with_passkeys(config())
            .unwrap();
        let alice = auth
            .create_user(NewUser::new("alice".parse().unwrap()))
            .await
            .unwrap();
        (auth, alice)
    }

    #[test]
    fn origin_must_belong_to_the_rp_id() {
        let mismatched = PasskeyConfig {
            origin: "https://evil.example.org".parse().unwrap(),
            ..config()
        };
        let result = Auth::new(MemoryStore::new(), AuthConfig::default()).with_passkeys(mismatched);
        assert!(matches!(result, Err(PasskeyError::Webauthn(_))));
    }

    #[tokio::test]
    async fn registration_challenge_names_the_site_and_user() {
        let (auth, alice) = auth_with_alice().await;
        let (challenge, state) = auth.begin_passkey_registration(&alice).await.unwrap();
        let json = serde_json::to_value(&challenge).unwrap();
        assert_eq!(json["publicKey"]["rp"]["id"], "lemnos.example.com");
        assert_eq!(json["publicKey"]["user"]["name"], "alice");

        // The state survives being stored between the two requests.
        let stored = serde_json::to_string(&state).unwrap();
        let restored: PasskeyRegistrationState = serde_json::from_str(&stored).unwrap();
        assert_eq!(restored.user, alice.id);
    }

    #[tokio::test]
    async fn sign_in_needs_a_registered_passkey() {
        let (auth, _) = auth_with_alice().await;
        for username in ["alice", "nobody", "not a username"] {
            assert!(matches!(
                auth.begin_passkey_login(username).await,
                Err(AuthError::Passkey(PasskeyError::NoPasskeys))
            ));
        }
    }

    #[tokio::test]
    async fn passkeys_must_be_configured() {
        let auth = Auth::new(MemoryStore::new(), AuthConfig::default());
        assert!(matches!(
            auth.begin_passkey_login("alice").await,
            Err(AuthError::NotConfigured(_))
        ));
    }
}
