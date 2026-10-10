//! Lemnos' database: SQLite, PostgreSQL or MariaDB/MySQL, picked at runtime
//! by the connection URL.
//!
//! ```text
//! sqlite:lemnos.db?mode=rwc                       a file next to Lemnos
//! postgres://user:password@host/lemnos
//! mysql://user:password@host/lemnos               MariaDB or MySQL
//! ```
//!
//! For SQLite, write `sqlite:` without `//` in front of the path, or a
//! Windows path like `C:/data/lemnos.db` is misread. `mode=rwc` creates the
//! file if it doesn't exist yet.
//!
//! [`Database`] implements [`lemnos_auth::Store`], so it drops in wherever
//! the in-memory store is used. Switching databases is a change of URL; data
//! is not moved from one to the other.
//!
//! To work the same on all three, the schema sticks to what they share:
//! every column is a 64-bit integer or text. Times are stored as seconds
//! since the Unix epoch, and binary values as hex.

use sqlx::{
    AnyPool, AssertSqlSafe,
    any::{AnyArguments, AnyPoolOptions, install_default_drivers},
    query::Query,
};

mod auth_store;
mod schema;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("`{0}` is not a database URL Lemnos supports; use sqlite://, postgres:// or mysql://")]
    UnsupportedUrl(String),
    #[error("could not connect to the database")]
    Connect(#[source] sqlx::Error),
    #[error("could not set up the database tables")]
    Schema(#[source] sqlx::Error),
}

/// The SQL dialects that differ in ways Lemnos has to care about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialect {
    Sqlite,
    Postgres,
    MySql,
}

impl Dialect {
    fn from_url(url: &str) -> Option<Self> {
        match url.split_once(':')?.0 {
            "sqlite" => Some(Self::Sqlite),
            "postgres" | "postgresql" => Some(Self::Postgres),
            "mysql" | "mariadb" => Some(Self::MySql),
            _ => None,
        }
    }
}

/// A pool of connections to Lemnos' database. Cheap to clone; every clone
/// shares the same connections.
#[derive(Clone)]
pub struct Database {
    pool: AnyPool,
    dialect: Dialect,
}

impl Database {
    /// Connects and brings the tables up to date.
    pub async fn connect(url: &str) -> Result<Self, DbError> {
        let dialect = Dialect::from_url(url).ok_or_else(|| DbError::UnsupportedUrl(redact(url)))?;
        install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await
            .map_err(DbError::Connect)?;
        let database = Self { pool, dialect };
        if dialect == Dialect::Sqlite {
            // Lets readers and a writer work at the same time.
            database
                .query("PRAGMA journal_mode = WAL")
                .execute(&database.pool)
                .await
                .map_err(DbError::Schema)?;
        }
        schema::migrate(&database).await.map_err(DbError::Schema)?;
        Ok(database)
    }

    /// Prepares `sql`, which is written with `?` placeholders. PostgreSQL
    /// wants `$1, $2, ...` instead, so they are renumbered for it.
    ///
    /// Only ever called with SQL written in this crate, never with user
    /// input; values go in through `.bind()`.
    pub(crate) fn query(&self, sql: &str) -> Query<'static, sqlx::Any, AnyArguments> {
        sqlx::query(AssertSqlSafe(placeholders(self.dialect, sql)))
    }
}

fn placeholders(dialect: Dialect, sql: &str) -> String {
    if dialect != Dialect::Postgres {
        return sql.to_owned();
    }
    let mut numbered = String::with_capacity(sql.len() + 8);
    let mut count = 0;
    for character in sql.chars() {
        if character == '?' {
            count += 1;
            numbered.push('$');
            numbered.push_str(&count.to_string());
        } else {
            numbered.push(character);
        }
    }
    numbered
}

/// The URL without its password, for error messages.
fn redact(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme_end), Some(at)) if at > scheme_end => {
            format!("{}://…{}", &url[..scheme_end], &url[at..])
        }
        _ => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_numbered_for_postgres_only() {
        let sql = "SELECT a FROM t WHERE b = ? AND c = ?";
        assert_eq!(placeholders(Dialect::Sqlite, sql), sql);
        assert_eq!(placeholders(Dialect::MySql, sql), sql);
        assert_eq!(
            placeholders(Dialect::Postgres, sql),
            "SELECT a FROM t WHERE b = $1 AND c = $2"
        );
    }

    #[test]
    fn dialect_follows_the_url_scheme() {
        assert_eq!(
            Dialect::from_url("sqlite://lemnos.db"),
            Some(Dialect::Sqlite)
        );
        assert_eq!(
            Dialect::from_url("postgresql://h/db"),
            Some(Dialect::Postgres)
        );
        assert_eq!(Dialect::from_url("mariadb://h/db"), Some(Dialect::MySql));
        assert_eq!(Dialect::from_url("mongodb://h/db"), None);
        assert_eq!(Dialect::from_url("lemnos.db"), None);
    }

    #[test]
    fn passwords_stay_out_of_error_messages() {
        assert_eq!(
            redact("postgres://lemnos:hunter2@db.example.com/lemnos"),
            "postgres://…@db.example.com/lemnos"
        );
        assert_eq!(redact("sqlite://lemnos.db"), "sqlite://lemnos.db");
    }
}
