//! [`Database`] as the place `lemnos-auth` keeps accounts, credentials and
//! sessions: one SQL statement (or a short transaction) per [`Store`] method.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lemnos_auth::{
    NewUser, SessionId, Store, StoreError, User, UserId, Username,
    store::{PasskeyRecord, SessionRecord, TotpRecord},
};
use sqlx::{Row, any::AnyRow};

use crate::Database;

type StoreResult<T> = Result<T, StoreError>;

const USER_COLUMNS: &str = "id, username, name, email";

/// Database errors become [`StoreError::Backend`], except a broken
/// uniqueness rule, which is the caller's "already exists".
fn store_error(error: sqlx::Error) -> StoreError {
    match error.as_database_error() {
        Some(database_error) if database_error.is_unique_violation() => StoreError::Conflict,
        _ => StoreError::Backend(Box::new(error)),
    }
}

/// For values read back that the database should never have held.
fn corrupt(what: &str) -> StoreError {
    StoreError::Backend(format!("the database holds an invalid {what}").into())
}

fn user_from(row: &AnyRow) -> StoreResult<User> {
    let username: String = row.try_get(1).map_err(store_error)?;
    Ok(User {
        id: UserId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
        username: Username::parse(&username).map_err(|_| corrupt("username"))?,
        name: row.try_get(2).map_err(store_error)?,
        email: row.try_get(3).map_err(store_error)?,
    })
}

fn id(user: UserId) -> i64 {
    user.0 as i64
}

fn to_seconds(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn from_seconds(seconds: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds.max(0) as u64)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|start| u8::from_str_radix(hex.get(start..start + 2)?, 16).ok())
        .collect()
}

impl Database {
    /// Fails with [`StoreError::NoSuchUser`] unless the user exists. Used
    /// before updates, because databases disagree on whether an update that
    /// changes nothing counts as having matched a row.
    async fn require_user(&self, user: UserId) -> StoreResult<()> {
        self.query("SELECT id FROM users WHERE id = ?")
            .bind(id(user))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(|_| ())
            .ok_or(StoreError::NoSuchUser)
    }

    /// Removes sessions that ended before `now`. They are never accepted
    /// either way; this only keeps the table from growing forever.
    pub async fn delete_expired_sessions(&self, now: SystemTime) -> StoreResult<u64> {
        Ok(self
            .query("DELETE FROM sessions WHERE expires_at <= ?")
            .bind(to_seconds(now))
            .execute(&self.pool)
            .await
            .map_err(store_error)?
            .rows_affected())
    }
}

impl Store for Database {
    async fn create_user(&self, new: NewUser) -> StoreResult<User> {
        // A random ID needs no database-specific "give me the next number"
        // feature. Half the range is plenty, and keeps it positive in SQL.
        let user = User {
            id: UserId(rand::random_range(1..=i64::MAX as u64)),
            name: new.name.unwrap_or_else(|| new.username.to_string()),
            username: new.username,
            email: new.email,
        };
        self.query(
            "INSERT INTO users (id, username, name, email, totp_confirmed) VALUES (?, ?, ?, ?, 0)",
        )
        .bind(id(user.id))
        .bind(user.username.as_str().to_owned())
        .bind(user.name.clone())
        .bind(user.email.clone())
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(user)
    }

    async fn user(&self, user: UserId) -> StoreResult<Option<User>> {
        self.query(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?"))
            .bind(id(user))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .as_ref()
            .map(user_from)
            .transpose()
    }

    async fn user_by_username(&self, username: &Username) -> StoreResult<Option<User>> {
        self.query(&format!(
            "SELECT {USER_COLUMNS} FROM users WHERE username = ?"
        ))
        .bind(username.as_str().to_owned())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .as_ref()
        .map(user_from)
        .transpose()
    }

    async fn password_hash(&self, user: UserId) -> StoreResult<Option<String>> {
        self.query("SELECT password_hash FROM users WHERE id = ?")
            .bind(id(user))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .ok_or(StoreError::NoSuchUser)?
            .try_get(0)
            .map_err(store_error)
    }

    async fn set_password_hash(&self, user: UserId, hash: Option<String>) -> StoreResult<()> {
        self.require_user(user).await?;
        self.query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(hash)
            .bind(id(user))
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    async fn user_by_identity(&self, provider: &str, subject: &str) -> StoreResult<Option<User>> {
        self.query(
            "SELECT users.id, users.username, users.name, users.email
             FROM identities JOIN users ON users.id = identities.user_id
             WHERE identities.provider = ? AND identities.subject = ?",
        )
        .bind(provider.to_owned())
        .bind(subject.to_owned())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .as_ref()
        .map(user_from)
        .transpose()
    }

    async fn link_identity(&self, user: UserId, provider: &str, subject: &str) -> StoreResult<()> {
        self.require_user(user).await?;
        self.query("INSERT INTO identities (provider, subject, user_id) VALUES (?, ?, ?)")
            .bind(provider.to_owned())
            .bind(subject.to_owned())
            .bind(id(user))
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    async fn insert_session(&self, session: SessionRecord) -> StoreResult<()> {
        let session_id = to_hex(&session.id.0);
        let mut transaction = self.pool.begin().await.map_err(store_error)?;
        self.query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id.clone())
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
        self.query("INSERT INTO sessions (id, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(session_id)
            .bind(id(session.user))
            .bind(to_seconds(session.expires_at))
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
        transaction.commit().await.map_err(store_error)
    }

    async fn session(&self, session: &SessionId) -> StoreResult<Option<SessionRecord>> {
        let Some(row) = self
            .query("SELECT user_id, expires_at FROM sessions WHERE id = ?")
            .bind(to_hex(&session.0))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
        else {
            return Ok(None);
        };
        Ok(Some(SessionRecord {
            id: *session,
            user: UserId(row.try_get::<i64, _>(0).map_err(store_error)? as u64),
            expires_at: from_seconds(row.try_get(1).map_err(store_error)?),
        }))
    }

    async fn delete_session(&self, session: &SessionId) -> StoreResult<()> {
        self.query("DELETE FROM sessions WHERE id = ?")
            .bind(to_hex(&session.0))
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    async fn delete_sessions_of(&self, user: UserId) -> StoreResult<()> {
        self.query("DELETE FROM sessions WHERE user_id = ?")
            .bind(id(user))
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }

    async fn totp(&self, user: UserId) -> StoreResult<Option<TotpRecord>> {
        let row = self
            .query("SELECT totp_secret, totp_confirmed, totp_last_step FROM users WHERE id = ?")
            .bind(id(user))
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .ok_or(StoreError::NoSuchUser)?;
        let Some(secret) = row.try_get::<Option<String>, _>(0).map_err(store_error)? else {
            return Ok(None);
        };
        Ok(Some(TotpRecord {
            secret: from_hex(&secret).ok_or_else(|| corrupt("TOTP secret"))?,
            confirmed: row.try_get::<i64, _>(1).map_err(store_error)? != 0,
            last_step: row
                .try_get::<Option<i64>, _>(2)
                .map_err(store_error)?
                .map(|step| step as u64),
        }))
    }

    async fn set_totp(&self, user: UserId, totp: Option<TotpRecord>) -> StoreResult<()> {
        self.require_user(user).await?;
        let (secret, confirmed, last_step) = match totp {
            Some(totp) => (
                Some(to_hex(&totp.secret)),
                i64::from(totp.confirmed),
                totp.last_step.map(|step| step as i64),
            ),
            None => (None, 0, None),
        };
        self.query(
            "UPDATE users SET totp_secret = ?, totp_confirmed = ?, totp_last_step = ? WHERE id = ?",
        )
        .bind(secret)
        .bind(confirmed)
        .bind(last_step)
        .bind(id(user))
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn advance_totp_step(&self, user: UserId, step: u64) -> StoreResult<bool> {
        // Check and update in one statement, so two sign-ins racing with
        // the same code can't both win.
        let step = step as i64;
        let updated = self
            .query(
                "UPDATE users SET totp_last_step = ?
                 WHERE id = ? AND totp_secret IS NOT NULL
                   AND (totp_last_step IS NULL OR totp_last_step < ?)",
            )
            .bind(step)
            .bind(id(user))
            .bind(step)
            .execute(&self.pool)
            .await
            .map_err(store_error)?
            .rows_affected();
        Ok(updated == 1)
    }

    async fn passkeys(&self, user: UserId) -> StoreResult<Vec<PasskeyRecord>> {
        self.require_user(user).await?;
        self.query(
            "SELECT credential_id, label, data FROM passkeys
             WHERE user_id = ? ORDER BY created_at, credential_id",
        )
        .bind(id(user))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|row| {
            let credential_id: String = row.try_get(0).map_err(store_error)?;
            Ok(PasskeyRecord {
                credential_id: from_hex(&credential_id).ok_or_else(|| corrupt("passkey ID"))?,
                label: row.try_get(1).map_err(store_error)?,
                data: row.try_get(2).map_err(store_error)?,
            })
        })
        .collect()
    }

    async fn save_passkey(&self, user: UserId, passkey: PasskeyRecord) -> StoreResult<()> {
        self.require_user(user).await?;
        let credential_id = to_hex(&passkey.credential_id);
        let mut transaction = self.pool.begin().await.map_err(store_error)?;
        let exists = self
            .query("SELECT user_id FROM passkeys WHERE user_id = ? AND credential_id = ?")
            .bind(id(user))
            .bind(credential_id.clone())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(store_error)?
            .is_some();
        if exists {
            self.query(
                "UPDATE passkeys SET label = ?, data = ? WHERE user_id = ? AND credential_id = ?",
            )
            .bind(passkey.label)
            .bind(passkey.data)
            .bind(id(user))
            .bind(credential_id)
        } else {
            self.query(
                "INSERT INTO passkeys (label, data, user_id, credential_id, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(passkey.label)
            .bind(passkey.data)
            .bind(id(user))
            .bind(credential_id)
            .bind(to_seconds(SystemTime::now()))
        }
        .execute(&mut *transaction)
        .await
        .map_err(store_error)?;
        transaction.commit().await.map_err(store_error)
    }

    async fn delete_passkey(&self, user: UserId, credential_id: &[u8]) -> StoreResult<()> {
        self.require_user(user).await?;
        self.query("DELETE FROM passkeys WHERE user_id = ? AND credential_id = ?")
            .bind(id(user))
            .bind(to_hex(credential_id))
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
        Ok(())
    }
}
