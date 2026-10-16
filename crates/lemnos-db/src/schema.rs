//! The tables, and how they are brought up to date.
//!
//! [`MIGRATIONS`] is a list of steps. A fresh database runs all of them; an
//! existing one runs only the steps added since it was last started. To
//! change the schema, append a new step. Never edit one that has shipped:
//! databases that already ran it would not get the change.

use sqlx::Row;

use crate::{Database, Dialect};

/// Step N (counting from 1) takes the schema from version N-1 to N.
const MIGRATIONS: &[fn(Dialect) -> Vec<String>] = &[
    accounts_and_sessions,
    unique_passkeys,
    proxmox_environments,
    organizations,
    projects_and_machines,
    machine_markers,
];

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

/// A passkey belongs to exactly one account, and the database enforces it.
fn unique_passkeys(dialect: Dialect) -> Vec<String> {
    let mut statements = Vec::new();
    if dialect == Dialect::MySql {
        // MariaDB can't index a TEXT column in full, and a multi-byte
        // VARCHAR this long is over its index size limit. The IDs are hex,
        // so a single-byte character set fits. 2046 = the longest ID
        // WebAuthn allows (1023 bytes), in hex.
        statements.push(
            "ALTER TABLE passkeys MODIFY credential_id
             VARCHAR(2046) CHARACTER SET ascii COLLATE ascii_bin NOT NULL"
                .to_owned(),
        );
    }
    statements
        .push("CREATE UNIQUE INDEX passkeys_credential_id ON passkeys (credential_id)".to_owned());
    statements
}

/// The Proxmox VE environments Lemnos manages. `token_id` is only set when
/// the login is an API token; `secret` is then the token's secret, and the
/// user's password otherwise.
fn proxmox_environments(dialect: Dialect) -> Vec<String> {
    // As for the first tables: names compare by bytes on every database.
    let table_options = match dialect {
        Dialect::MySql => " DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin",
        Dialect::Sqlite | Dialect::Postgres => "",
    };
    vec![format!(
        "CREATE TABLE proxmox_environments (
            id BIGINT NOT NULL PRIMARY KEY,
            name VARCHAR(64) NOT NULL UNIQUE,
            url VARCHAR(255) NOT NULL,
            username VARCHAR(64) NOT NULL,
            realm VARCHAR(32) NOT NULL,
            token_id VARCHAR(64),
            secret TEXT NOT NULL,
            accept_invalid_certs BIGINT NOT NULL
        ){table_options}"
    )]
}

/// Organisations, who is in them, which environments each was given, and
/// the admins of the whole installation.
fn organizations(dialect: Dialect) -> Vec<String> {
    // As for the first tables: names compare by bytes on every database.
    let table_options = match dialect {
        Dialect::MySql => " DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin",
        Dialect::Sqlite | Dialect::Postgres => "",
    };
    [
        "CREATE TABLE organizations (
            id BIGINT NOT NULL PRIMARY KEY,
            name VARCHAR(64) NOT NULL UNIQUE
        )",
        "CREATE TABLE memberships (
            user_id BIGINT NOT NULL,
            organization_id BIGINT NOT NULL,
            role VARCHAR(16) NOT NULL,
            PRIMARY KEY (user_id, organization_id),
            FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE,
            FOREIGN KEY (organization_id) REFERENCES organizations (id) ON DELETE CASCADE
        )",
        // `tag` marks the organisation's machines in the environment; empty
        // means all of them.
        "CREATE TABLE organization_environments (
            organization_id BIGINT NOT NULL,
            environment_id BIGINT NOT NULL,
            tag VARCHAR(64) NOT NULL,
            PRIMARY KEY (organization_id, environment_id),
            FOREIGN KEY (organization_id) REFERENCES organizations (id) ON DELETE CASCADE,
            FOREIGN KEY (environment_id) REFERENCES proxmox_environments (id) ON DELETE CASCADE
        )",
        "CREATE TABLE admins (
            user_id BIGINT NOT NULL PRIMARY KEY,
            FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
        )",
        "CREATE INDEX memberships_organization_id ON memberships (organization_id)",
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

/// Projects inside organisations, who owns the machines Lemnos created, how
/// many an organisation may create, and installation-wide settings.
fn projects_and_machines(dialect: Dialect) -> Vec<String> {
    // As for the first tables: names compare by bytes on every database.
    let table_options = match dialect {
        Dialect::MySql => " DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin",
        Dialect::Sqlite | Dialect::Postgres => "",
    };
    [
        // How many machines the organisation's people may create, its
        // projects' included. Admins are not counted against it.
        "ALTER TABLE organizations ADD COLUMN machine_quota BIGINT NOT NULL DEFAULT 0",
        "CREATE TABLE projects (
            id BIGINT NOT NULL PRIMARY KEY,
            organization_id BIGINT NOT NULL,
            name VARCHAR(64) NOT NULL,
            UNIQUE (organization_id, name),
            FOREIGN KEY (organization_id) REFERENCES organizations (id) ON DELETE CASCADE
        )",
        "CREATE TABLE project_members (
            project_id BIGINT NOT NULL,
            user_id BIGINT NOT NULL,
            PRIMARY KEY (project_id, user_id),
            FOREIGN KEY (project_id) REFERENCES projects (id) ON DELETE CASCADE,
            FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
        )",
        // One row per machine Lemnos created. `machine` is the provider's
        // own ID for it. The owner is the project if `project_id` is set,
        // else the organisation if `organization_id` is, else `user_id`.
        "CREATE TABLE machines (
            environment_id BIGINT NOT NULL,
            machine VARCHAR(64) NOT NULL,
            name VARCHAR(255) NOT NULL,
            user_id BIGINT,
            organization_id BIGINT,
            project_id BIGINT,
            created_by BIGINT,
            created_at BIGINT NOT NULL,
            PRIMARY KEY (environment_id, machine),
            FOREIGN KEY (environment_id) REFERENCES proxmox_environments (id) ON DELETE CASCADE
        )",
        "CREATE TABLE settings (
            name VARCHAR(64) NOT NULL PRIMARY KEY,
            value VARCHAR(255) NOT NULL
        )",
        "CREATE INDEX machines_user_id ON machines (user_id)",
        "CREATE INDEX machines_organization_id ON machines (organization_id)",
        "CREATE INDEX project_members_user_id ON project_members (user_id)",
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

/// The marker tag each machine Lemnos creates carries (see
/// `lemnos_core::marker`). Records from before have none: they are
/// unverified until an admin confirms them.
fn machine_markers(_: Dialect) -> Vec<String> {
    vec!["ALTER TABLE machines ADD COLUMN marker VARCHAR(32)".to_owned()]
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
