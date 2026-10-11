//! TOTP: the six-digit codes from an authenticator app, as a second factor
//! after a password.
//!
//! Setting it up takes two steps, so nobody gets locked out by a
//! half-finished setup: [`Auth::begin_totp_enrollment`] hands out the secret,
//! and 2FA only switches on once [`Auth::confirm_totp`] has seen a valid code.
//!
//! There are no recovery codes yet: a user who loses their phone needs an
//! admin to call [`Auth::disable_totp`].

use totp_rs::{Algorithm, Builder, Secret, Totp};

use crate::{
    auth::{Auth, AuthError, Authenticated, SecondFactorChallenge},
    store::{Store, TotpRecord},
    unix_now,
    user::{User, UserId},
};

/// What to show a user who is setting up their authenticator app.
#[derive(Clone, Debug)]
pub struct TotpEnrollment {
    /// For typing in by hand.
    pub secret_base32: String,
    /// The `otpauth://` link; render it as a QR code for the app to scan.
    pub otpauth_url: String,
}

impl<S: Store> Auth<S> {
    /// Whether `user` has to enter a code after their password.
    pub async fn totp_enabled(&self, user: UserId) -> Result<bool, AuthError> {
        Ok(self
            .store
            .totp(user)
            .await?
            .is_some_and(|totp| totp.confirmed))
    }

    /// Generates a new secret for `user`. Nothing changes for sign-in until
    /// [`confirm_totp`](Self::confirm_totp) succeeds.
    pub async fn begin_totp_enrollment(&self, user: &User) -> Result<TotpEnrollment, AuthError> {
        // Starting over while 2FA is on would quietly switch it off until
        // the new secret is confirmed.
        if self.totp_enabled(user.id).await? {
            return Err(AuthError::TotpAlreadyEnabled);
        }
        let secret = Secret::generate();
        let totp = self.totp(secret.as_bytes(), user.username.as_str())?;
        let enrollment = TotpEnrollment {
            secret_base32: secret.to_base32(),
            otpauth_url: totp.to_url().map_err(internal)?,
        };
        self.store
            .set_totp(
                user.id,
                Some(TotpRecord {
                    secret: secret.as_bytes().to_vec(),
                    confirmed: false,
                    last_step: None,
                }),
            )
            .await?;
        Ok(enrollment)
    }

    /// Switches 2FA on, once `code` proves the user's app has the secret.
    pub async fn confirm_totp(&self, user: UserId, code: &str) -> Result<(), AuthError> {
        let record = self
            .store
            .totp(user)
            .await?
            .ok_or(AuthError::TotpNotSetUp)?;
        if record.confirmed {
            return Err(AuthError::TotpAlreadyEnabled);
        }
        let step = self.check_code(user, &record, code).await?;
        self.store
            .set_totp(
                user,
                Some(TotpRecord {
                    confirmed: true,
                    last_step: Some(step),
                    ..record
                }),
            )
            .await?;
        Ok(())
    }

    /// Switches 2FA off. Ask for the password (or have an admin do it)
    /// before calling this; it does not check who is asking.
    pub async fn disable_totp(&self, user: UserId) -> Result<(), AuthError> {
        Ok(self.store.set_totp(user, None).await?)
    }

    /// Completes a sign-in that returned
    /// [`Login::SecondFactor`](crate::Login::SecondFactor). A wrong code can
    /// be retried with the same challenge until it expires.
    pub async fn verify_totp(
        &self,
        challenge: &SecondFactorChallenge,
        code: &str,
    ) -> Result<Authenticated, AuthError> {
        if challenge.is_expired() {
            return Err(AuthError::Expired);
        }
        let record = self
            .store
            .totp(challenge.user)
            .await?
            .filter(|record| record.confirmed)
            .ok_or(AuthError::TotpNotSetUp)?;
        let step = self.check_code(challenge.user, &record, code).await?;
        // Each code is good for one sign-in, so someone who watched it being
        // typed can't reuse it within the same 30 seconds.
        if !self.store.advance_totp_step(challenge.user, step).await? {
            return Err(AuthError::InvalidCode);
        }
        let user = self
            .store
            .user(challenge.user)
            .await?
            .ok_or(AuthError::InvalidCredentials)?;
        Ok(Authenticated {
            user,
            method: challenge.method,
            second_factor: true,
        })
    }

    /// The time step `code` is valid for. Throttled per user: with only a
    /// million possible codes, unlimited guesses would get through.
    async fn check_code(
        &self,
        user: UserId,
        record: &TotpRecord,
        code: &str,
    ) -> Result<u64, AuthError> {
        let key = format!("totp:{user}");
        self.throttle.check(&key).map_err(AuthError::Throttled)?;
        // Apps show codes as "123 456"; accept them typed that way.
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        match self.totp(&record.secret, "")?.check(&code, unix_now()) {
            Some(step) => {
                self.throttle.success(&key);
                Ok(step)
            }
            None => {
                self.throttle.failure(&key);
                Err(AuthError::InvalidCode)
            }
        }
    }

    /// SHA-1, 6 digits, 30 seconds: the only combination every authenticator
    /// app supports. One step of clock drift either way is tolerated.
    fn totp(&self, secret: &[u8], account: &str) -> Result<Totp, AuthError> {
        Builder::new()
            .with_algorithm(Algorithm::SHA1)
            .with_digits(6)
            .with_step_duration(30)
            .with_skew(1)
            .with_secret(secret.to_vec())
            .with_issuer(Some(self.config.issuer.as_str()))
            .with_account_name(account)
            .build()
            .map_err(internal)
    }
}

fn internal(error: impl std::fmt::Display) -> AuthError {
    AuthError::Internal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuthConfig, Login, MemoryStore, NewUser};

    const PASSWORD: &str = "correct horse battery";

    /// What the user's authenticator app would show right now.
    fn current_code(auth: &Auth<MemoryStore>, enrollment: &TotpEnrollment) -> String {
        let secret = Secret::try_from_base32(&enrollment.secret_base32).unwrap();
        auth.totp(secret.as_bytes(), "")
            .unwrap()
            .generate(unix_now())
            .to_string()
    }

    #[tokio::test]
    async fn enrollment_then_sign_in_with_code() {
        let auth = Auth::new(MemoryStore::new(), AuthConfig::default());
        let alice = auth
            .create_user(NewUser::new("alice".parse().unwrap()))
            .await
            .unwrap();
        auth.set_password(alice.id, PASSWORD).await.unwrap();

        let enrollment = auth.begin_totp_enrollment(&alice).await.unwrap();
        assert!(enrollment.otpauth_url.starts_with("otpauth://totp/"));
        assert!(enrollment.otpauth_url.contains("issuer=Lemnos"));

        // Not confirmed yet, so the password alone is still enough.
        assert!(!auth.totp_enabled(alice.id).await.unwrap());
        assert!(matches!(
            auth.login_password("alice", PASSWORD).await.unwrap(),
            Login::Complete(_)
        ));
        assert!(matches!(
            auth.confirm_totp(alice.id, "000000").await,
            Err(AuthError::InvalidCode)
        ));

        let code = current_code(&auth, &enrollment);
        auth.confirm_totp(alice.id, &code).await.unwrap();
        assert!(auth.totp_enabled(alice.id).await.unwrap());
        assert!(matches!(
            auth.begin_totp_enrollment(&alice).await,
            Err(AuthError::TotpAlreadyEnabled)
        ));

        // Now the password only gets as far as a challenge.
        let Login::SecondFactor(challenge) = auth.login_password("alice", PASSWORD).await.unwrap()
        else {
            panic!("2FA is on");
        };
        // The code used to confirm can't be used again to sign in.
        assert!(matches!(
            auth.verify_totp(&challenge, &code).await,
            Err(AuthError::InvalidCode)
        ));

        auth.disable_totp(alice.id).await.unwrap();
        assert!(matches!(
            auth.login_password("alice", PASSWORD).await.unwrap(),
            Login::Complete(_)
        ));
    }

    #[tokio::test]
    async fn a_code_signs_in_once() {
        let auth = Auth::new(MemoryStore::new(), AuthConfig::default());
        let alice = auth
            .create_user(NewUser::new("alice".parse().unwrap()))
            .await
            .unwrap();
        let enrollment = auth.begin_totp_enrollment(&alice).await.unwrap();
        // Confirm as if it happened a minute ago, so the current code is
        // still unused.
        let secret = Secret::try_from_base32(&enrollment.secret_base32).unwrap();
        auth.store
            .set_totp(
                alice.id,
                Some(TotpRecord {
                    secret: secret.as_bytes().to_vec(),
                    confirmed: true,
                    last_step: None,
                }),
            )
            .await
            .unwrap();

        let challenge = SecondFactorChallenge {
            user: alice.id,
            method: crate::Method::Password,
            issued_at: unix_now(),
        };
        let code = current_code(&auth, &enrollment);
        let spaced = format!("{} {}", &code[..3], &code[3..]);
        let authenticated = auth.verify_totp(&challenge, &spaced).await.unwrap();
        assert!(authenticated.used_second_factor());
        assert!(matches!(
            auth.verify_totp(&challenge, &code).await,
            Err(AuthError::InvalidCode)
        ));

        let stale = SecondFactorChallenge {
            issued_at: unix_now() - 3600,
            ..challenge
        };
        assert!(matches!(
            auth.verify_totp(&stale, &code).await,
            Err(AuthError::Expired)
        ));
    }
}
