//! The same checks against every kind of store, so they all behave alike.
//!
//! The in-memory and SQLite ones always run. PostgreSQL and MariaDB need the
//! containers from `compose.dev.yml`:
//!
//! ```text
//! docker compose -f compose.dev.yml up -d --wait
//! cargo test -p lemnos-db -- --ignored
//! ```

use std::time::{Duration, SystemTime};

use lemnos_auth::{
    Auth, AuthConfig, Login, MemoryStore, NewUser, SessionId, Store, StoreError, UserId, Username,
    store::{PasskeyRecord, SessionRecord, TotpRecord},
};
use lemnos_db::Database;

const POSTGRES: &str = "postgres://lemnos:lemnos-dev@localhost:54320/lemnos";
const MARIADB: &str = "mysql://lemnos:lemnos-dev@localhost:33060/lemnos";
const NEEDS_STACK: &str = "run `docker compose -f compose.dev.yml up -d --wait` first";

/// A value no earlier test run has used. The PostgreSQL and MariaDB
/// containers keep their rows between runs.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{:016x}", rand::random::<u64>())
}

fn username(prefix: &str) -> Username {
    unique(prefix).parse().unwrap()
}

/// A fresh SQLite file in the temp directory; the URL and the path to
/// delete afterwards.
fn sqlite_file() -> (String, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("{}.db", unique("lemnos-test")));
    // `sqlite:` without `//`: after the slashes, `C:` would be read as a host.
    let url = format!("sqlite:{}?mode=rwc", path.display()).replace('\\', "/");
    (url, path)
}

fn remove_sqlite_file(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

async fn exercise<S: Store>(store: &S) {
    // --- users ---
    let alice_name = username("alice");
    let alice = store
        .create_user(NewUser {
            username: alice_name.clone(),
            name: Some("Alice Liddell".into()),
            email: Some("alice@example.com".into()),
        })
        .await
        .unwrap();
    assert_eq!(alice.name, "Alice Liddell");
    assert_eq!(store.user(alice.id).await.unwrap(), Some(alice.clone()));
    assert_eq!(
        store.user_by_username(&alice_name).await.unwrap(),
        Some(alice.clone())
    );
    assert!(matches!(
        store.create_user(NewUser::new(alice_name.clone())).await,
        Err(StoreError::Conflict)
    ));

    // Without a display name, the username stands in.
    let bob = store
        .create_user(NewUser::new(username("bob")))
        .await
        .unwrap();
    assert_eq!(bob.name, bob.username.as_str());
    assert_eq!(bob.email, None);
    assert_ne!(bob.id, alice.id);

    let nobody = UserId(u64::from(u32::MAX) + 12345);
    assert_eq!(store.user(nobody).await.unwrap(), None);
    assert_eq!(
        store.user_by_username(&username("ghost")).await.unwrap(),
        None
    );

    // --- passwords ---
    assert_eq!(store.password_hash(alice.id).await.unwrap(), None);
    store
        .set_password_hash(alice.id, Some("$argon2id$fake".into()))
        .await
        .unwrap();
    assert_eq!(
        store.password_hash(alice.id).await.unwrap().as_deref(),
        Some("$argon2id$fake")
    );
    // Setting the same value again is not an error.
    store
        .set_password_hash(alice.id, Some("$argon2id$fake".into()))
        .await
        .unwrap();
    store.set_password_hash(alice.id, None).await.unwrap();
    assert_eq!(store.password_hash(alice.id).await.unwrap(), None);
    assert!(matches!(
        store.password_hash(nobody).await,
        Err(StoreError::NoSuchUser)
    ));
    assert!(matches!(
        store.set_password_hash(nobody, None).await,
        Err(StoreError::NoSuchUser)
    ));

    // --- identities ---
    let subject = unique("Subject");
    assert_eq!(store.user_by_identity("dex", &subject).await.unwrap(), None);
    store
        .link_identity(alice.id, "dex", &subject)
        .await
        .unwrap();
    assert_eq!(
        store.user_by_identity("dex", &subject).await.unwrap(),
        Some(alice.clone())
    );
    // Same subject at another provider, or in other letter case, is
    // somebody else.
    assert_eq!(
        store.user_by_identity("saml", &subject).await.unwrap(),
        None
    );
    assert_eq!(
        store
            .user_by_identity("dex", &subject.to_lowercase())
            .await
            .unwrap(),
        None
    );
    assert!(matches!(
        store.link_identity(bob.id, "dex", &subject).await,
        Err(StoreError::Conflict)
    ));
    assert!(matches!(
        store.link_identity(nobody, "dex", &unique("s")).await,
        Err(StoreError::NoSuchUser)
    ));

    // --- sessions ---
    let session = SessionId(rand::random());
    let other_session = SessionId(rand::random());
    let bobs_session = SessionId(rand::random());
    // Whole seconds, since that is all the databases keep.
    let expires_at = SystemTime::UNIX_EPOCH + Duration::from_secs(4_000_000_000);
    assert!(store.session(&session).await.unwrap().is_none());
    for (id, user) in [
        (session, alice.id),
        (other_session, alice.id),
        (bobs_session, bob.id),
    ] {
        store
            .insert_session(SessionRecord {
                id,
                user,
                expires_at,
            })
            .await
            .unwrap();
    }
    let found = store.session(&session).await.unwrap().unwrap();
    assert_eq!((found.user, found.expires_at), (alice.id, expires_at));
    store.delete_session(&session).await.unwrap();
    assert!(store.session(&session).await.unwrap().is_none());
    assert!(store.session(&other_session).await.unwrap().is_some());
    store.delete_sessions_of(alice.id).await.unwrap();
    assert!(store.session(&other_session).await.unwrap().is_none());
    assert!(store.session(&bobs_session).await.unwrap().is_some());

    // --- TOTP ---
    assert!(store.totp(alice.id).await.unwrap().is_none());
    // No setup, so there is no step to advance.
    assert!(!store.advance_totp_step(alice.id, 10).await.unwrap());
    let secret: Vec<u8> = (0..20).collect();
    store
        .set_totp(
            alice.id,
            Some(TotpRecord {
                secret: secret.clone(),
                confirmed: false,
                last_step: None,
            }),
        )
        .await
        .unwrap();
    let totp = store.totp(alice.id).await.unwrap().unwrap();
    assert_eq!(
        (totp.secret, totp.confirmed, totp.last_step),
        (secret, false, None)
    );
    // Each step is accepted once, and never an older one.
    assert!(store.advance_totp_step(alice.id, 100).await.unwrap());
    assert!(!store.advance_totp_step(alice.id, 100).await.unwrap());
    assert!(!store.advance_totp_step(alice.id, 99).await.unwrap());
    assert!(store.advance_totp_step(alice.id, 101).await.unwrap());
    assert_eq!(
        store.totp(alice.id).await.unwrap().unwrap().last_step,
        Some(101)
    );
    store.set_totp(alice.id, None).await.unwrap();
    assert!(store.totp(alice.id).await.unwrap().is_none());
    assert!(matches!(
        store.totp(nobody).await,
        Err(StoreError::NoSuchUser)
    ));

    // --- passkeys ---
    assert!(store.passkeys(alice.id).await.unwrap().is_empty());
    let passkey = |id: u8, label: &str, data: &str| PasskeyRecord {
        credential_id: vec![id, 0, 255, 16],
        label: label.into(),
        data: data.into(),
    };
    store
        .save_passkey(alice.id, passkey(1, "Laptop", "{\"n\":1}"))
        .await
        .unwrap();
    store
        .save_passkey(alice.id, passkey(2, "Phone", "{\"n\":2}"))
        .await
        .unwrap();
    // Saving an existing credential ID replaces it.
    store
        .save_passkey(alice.id, passkey(1, "Laptop", "{\"n\":3}"))
        .await
        .unwrap();
    let mut passkeys = store.passkeys(alice.id).await.unwrap();
    passkeys.sort_by(|a, b| a.credential_id.cmp(&b.credential_id));
    assert_eq!(passkeys.len(), 2);
    assert_eq!(passkeys[0].credential_id, vec![1, 0, 255, 16]);
    assert_eq!(passkeys[0].data, "{\"n\":3}");
    assert_eq!(passkeys[1].label, "Phone");
    assert!(store.passkeys(bob.id).await.unwrap().is_empty());
    store
        .delete_passkey(alice.id, &[1, 0, 255, 16])
        .await
        .unwrap();
    assert_eq!(store.passkeys(alice.id).await.unwrap().len(), 1);
    assert!(matches!(
        store.save_passkey(nobody, passkey(9, "x", "{}")).await,
        Err(StoreError::NoSuchUser)
    ));
}

#[tokio::test]
async fn memory() {
    exercise(&MemoryStore::new()).await;
}

#[tokio::test]
async fn sqlite() {
    let (url, path) = sqlite_file();
    let database = Database::connect(&url).await.unwrap();
    exercise(&database).await;
    drop(database);
    remove_sqlite_file(&path);
}

#[tokio::test]
#[ignore = "needs the compose.dev.yml stack"]
async fn postgres() {
    exercise(&Database::connect(POSTGRES).await.expect(NEEDS_STACK)).await;
}

#[tokio::test]
#[ignore = "needs the compose.dev.yml stack"]
async fn mariadb() {
    exercise(&Database::connect(MARIADB).await.expect(NEEDS_STACK)).await;
}

/// The point of a database: a restart loses nothing.
#[tokio::test]
async fn sqlite_keeps_accounts_across_restarts() {
    let (url, path) = sqlite_file();
    let name = username("carol");
    {
        let auth = Auth::new(
            Database::connect(&url).await.unwrap(),
            AuthConfig::default(),
        );
        let carol = auth.create_user(NewUser::new(name.clone())).await.unwrap();
        auth.set_password(carol.id, "correct horse battery")
            .await
            .unwrap();
    }

    // A second connect finds the tables already there and leaves them be.
    let auth = Auth::new(
        Database::connect(&url).await.unwrap(),
        AuthConfig::default(),
    );
    let login = auth
        .login_password(name.as_str(), "correct horse battery")
        .await
        .unwrap();
    let Login::Complete(authenticated) = login else {
        panic!("no second factor is set up");
    };
    let session = SessionId(rand::random());
    auth.start_session(authenticated, session).await.unwrap();
    assert_eq!(
        auth.session_user(&session).await.unwrap().unwrap().username,
        name
    );
    assert!(
        auth.login_password(name.as_str(), "wrong password!")
            .await
            .is_err()
    );

    // Expired sessions can be swept out; live ones stay.
    let stale = SessionId(rand::random());
    let carol = auth.store().user_by_username(&name).await.unwrap().unwrap();
    auth.store()
        .insert_session(SessionRecord {
            id: stale,
            user: carol.id,
            expires_at: SystemTime::now() - Duration::from_secs(60),
        })
        .await
        .unwrap();
    assert_eq!(
        auth.store()
            .delete_expired_sessions(SystemTime::now())
            .await
            .unwrap(),
        1
    );
    assert!(auth.session_user(&session).await.unwrap().is_some());

    drop(auth);
    remove_sqlite_file(&path);
}

#[tokio::test]
async fn unsupported_urls_are_refused() {
    assert!(matches!(
        Database::connect("mongodb://localhost/lemnos").await,
        Err(lemnos_db::DbError::UnsupportedUrl(_))
    ));
}
