//! Password rules and hashing (Argon2id, through the `password-auth` crate).

use std::sync::LazyLock;

/// Which passwords are acceptable. Length is what matters; rules like "one
/// digit, one symbol" mostly produce `Password1!` and are left out on purpose.
#[derive(Clone, Debug)]
pub struct PasswordPolicy {
    pub min_length: usize,
}

impl PasswordPolicy {
    /// Hashing cost grows with input size, so an upper bound keeps someone
    /// from tying up the server with megabyte-long "passwords".
    pub const MAX_LENGTH: usize = 128;

    pub fn check(&self, password: &str) -> Result<(), WeakPassword> {
        let length = password.chars().count();
        if length < self.min_length {
            Err(WeakPassword::TooShort {
                min: self.min_length,
            })
        } else if length > Self::MAX_LENGTH {
            Err(WeakPassword::TooLong)
        } else {
            Ok(())
        }
    }
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self { min_length: 12 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WeakPassword {
    #[error("the password must be at least {min} characters")]
    TooShort { min: usize },
    #[error(
        "the password can be at most {} characters",
        PasswordPolicy::MAX_LENGTH
    )]
    TooLong,
}

/// Checked against when there is no real hash, so a sign-in for an unknown
/// user takes as long as one for a real user with a wrong password.
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| password_auth::generate_hash("no account has this password"));

/// Hashing takes tens of milliseconds by design. `spawn_blocking` moves it
/// off the async worker threads so other requests aren't held up meanwhile.
/// `None` means the hashing task itself failed.
pub(crate) async fn hash(password: String) -> Option<String> {
    tokio::task::spawn_blocking(move || password_auth::generate_hash(password))
        .await
        .ok()
}

/// Whether `password` matches `hash`. Always does a full hash computation,
/// also when there is no hash to compare with.
pub(crate) async fn verify(password: String, hash: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || {
        if password.len() > PasswordPolicy::MAX_LENGTH * 4 {
            return false;
        }
        match hash {
            Some(hash) => password_auth::verify_password(password, &hash).is_ok(),
            None => {
                let _ = password_auth::verify_password(password, &DUMMY_HASH);
                false
            }
        }
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_checks_length() {
        let policy = PasswordPolicy::default();
        assert_eq!(
            policy.check("short"),
            Err(WeakPassword::TooShort { min: 12 })
        );
        assert_eq!(policy.check(&"x".repeat(129)), Err(WeakPassword::TooLong));
        assert!(policy.check("correct horse battery").is_ok());
    }

    #[tokio::test]
    async fn hashes_verify() {
        let hash = hash("correct horse battery".into()).await.unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify("correct horse battery".into(), Some(hash.clone())).await);
        assert!(!verify("wrong".into(), Some(hash)).await);
        assert!(!verify("anything".into(), None).await);
    }
}
