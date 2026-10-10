//! The tables, and how they are brought up to date.
//!
//! [`MIGRATIONS`] is a list of steps. A fresh database runs all of them; an
//! existing one runs only the steps added since it was last started. To
//! change the schema, append a new step. Never edit one that has shipped:
//! databases that already ran it would not get the change.

use sqlx::Row;

use crate::{Database, Dialect};

/// Step N (counting from 1) takes the schema from version N-1 to N.
const MIGRATIONS: &[fn(Dialect) -> Vec<String>] = &[accounts_and_sessions];

fn accounts_and_sessions(dialect: Dialect) -> Vec<String> {
    // MariaDB/MySQL compare text ignoring case unless told otherwise. IDs
    // from identity providers are case-sensitive, so make it compare bytes
    // like the other two do.
    let table_options = match dialect {
        Dialect::MySql => " DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin",
        Dialect::Sqlite | Dialect::Postgres => "",
    };
    [
        "CREATE TABLE users (
            id BIGINT NOT NULL PRIMARY KEY,
            username VARCHAR(64) NOT NULL UNIQUE,
            name VARCHAR(255) NOT NULL,
            email VARCHAR(255),
            password_hash VARCHAR(255),
            totp_secret VARCHAR(255),
            totp_confirmed BIGINT NOT NULL,
            totp_last_step BIGINT
        )",
        "CREATE TABLE identities (
            provider VARCHAR(64) NOT NULL,
            subject VARCHAR(255) NOT NULL,
            user_id BIGINT NOT NULL,
            PRIMARY KEY (provider, subject),
            FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
        )",
        "CREATE TABLE sessions (
            id VARCHAR(64) NOT NULL PRIMARY KEY,
            user_id BIGINT NOT NULL,
            expires_at BIGINT NOT NULL,
            FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
        )",
        "CREATE TABLE passkeys (
            user_id BIGINT NOT NULL,
            credential_id TEXT NOT NULL,
            label VARCHAR(255) NOT NULL,
            data TEXT NOT NULL,
            created_at BIGINT NOT NULL,
            FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
        )",
        "CREATE INDEX sessions_user_id ON sessions (user_id)",
        "CREATE INDEX passkeys_user_id ON passkeys (user_id)",
    ]
    .into_iter()
    .map(|statement| {
        if statement.starts_with("CREATE TABLE") {
            format!("{statement}{table_options}")
        } else {
            statement.to_owned()
        }
    })
    .collect()
}

pub(crate) async fn migrate(database: &Database) -> Result<(), sqlx::Error> {
    database
        .query("CREATE TABLE IF NOT EXISTS lemnos_schema (version BIGINT NOT NULL)")
        .execute(&database.pool)
        .await?;
    let current: i64 = database
        .query("SELECT COALESCE(MAX(version), 0) FROM lemnos_schema")
        .fetch_one(&database.pool)
        .await?
        .try_get(0)?;

    for (index, step) in MIGRATIONS.iter().enumerate() {
        let version = index as i64 + 1;
        if version <= current {
            continue;
        }
        // One transaction per step, so SQLite and PostgreSQL never end up
        // with half a step applied. (MariaDB can't undo table changes.)
        let mut transaction = database.pool.begin().await?;
        for statement in step(database.dialect) {
            database
                .query(&statement)
                .execute(&mut *transaction)
                .await?;
        }
        database
            .query("INSERT INTO lemnos_schema (version) VALUES (?)")
            .bind(version)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
    }
    Ok(())
}
