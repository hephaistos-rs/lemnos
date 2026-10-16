//! Storing Proxmox environments, checked against every supported database.
//!
//! SQLite always runs. PostgreSQL and MariaDB need the containers from
//! `compose.dev.yml`:
//!
//! ```text
//! docker compose -f compose.dev.yml up -d --wait
//! cargo test -p lemnos-db -- --ignored
//! ```

use lemnos_db::{
    Database, NewProxmoxEnvironment, ProxmoxEnvironmentEdit, ProxmoxEnvironmentId, ProxmoxLogin,
    ProxmoxStoreError,
};

const POSTGRES: &str = "postgres://lemnos:lemnos-dev@localhost:54320/lemnos";
const MARIADB: &str = "mysql://lemnos:lemnos-dev@localhost:33060/lemnos";
const NEEDS_STACK: &str = "run `docker compose -f compose.dev.yml up -d --wait` first";

/// A value no earlier test run has used. The PostgreSQL and MariaDB
/// containers keep their rows between runs.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{:016x}", rand::random::<u64>())
}

fn with_token(name: &str) -> NewProxmoxEnvironment {
    NewProxmoxEnvironment {
        name: name.to_owned(),
        url: "https://pve.example.com:8006".into(),
        user: "root".into(),
        realm: "pam".into(),
        login: ProxmoxLogin::ApiToken {
            id: "lemnos".into(),
            secret: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
        },
        accept_invalid_certs: true,
    }
}

async fn exercise(database: &Database) {
    // --- create and read back ---
    let lab_name = unique("lab");
    let lab = database
        .create_proxmox_environment(with_token(&lab_name))
        .await
        .unwrap();
    assert_eq!(lab.name, lab_name);
    assert!(lab.accept_invalid_certs);
    assert_eq!(
        database.proxmox_environment(lab.id).await.unwrap(),
        Some(lab.clone())
    );

    // A password login comes back as one, not as a token without a name.
    let office = database
        .create_proxmox_environment(NewProxmoxEnvironment {
            login: ProxmoxLogin::Password("hunter2".into()),
            accept_invalid_certs: false,
            ..with_token(&unique("office"))
        })
        .await
        .unwrap();
    let stored = database.proxmox_environment(office.id).await.unwrap();
    assert_eq!(stored, Some(office.clone()));
    assert_eq!(office.login, ProxmoxLogin::Password("hunter2".into()));

    // --- names are unique ---
    assert!(matches!(
        database
            .create_proxmox_environment(with_token(&lab_name))
            .await,
        Err(ProxmoxStoreError::NameTaken)
    ));

    // --- listing is ordered by name ---
    let listed: Vec<_> = database
        .proxmox_environments()
        .await
        .unwrap()
        .into_iter()
        .filter(|environment| [lab.id, office.id].contains(&environment.id))
        .collect();
    assert_eq!(listed, [lab.clone(), office.clone()]);

    // --- update ---
    let mut changed = lab.clone();
    changed.url = "https://10.0.0.5:8006".into();
    changed.login = ProxmoxLogin::Password("correct horse".into());
    changed.accept_invalid_certs = false;
    database
        .update_proxmox_environment(changed.clone())
        .await
        .unwrap();
    assert_eq!(
        database.proxmox_environment(lab.id).await.unwrap(),
        Some(changed.clone())
    );

    // Saving it again without changes is fine.
    database
        .update_proxmox_environment(changed.clone())
        .await
        .unwrap();

    let mut renamed = changed.clone();
    renamed.name = office.name.clone();
    assert!(matches!(
        database.update_proxmox_environment(renamed).await,
        Err(ProxmoxStoreError::NameTaken)
    ));

    let mut missing = changed;
    missing.id = ProxmoxEnvironmentId(u64::from(u32::MAX) + 12345);
    assert!(matches!(
        database.update_proxmox_environment(missing.clone()).await,
        Err(ProxmoxStoreError::NotFound)
    ));
    assert_eq!(
        database.proxmox_environment(missing.id).await.unwrap(),
        None
    );

    // --- edit, keeping the secret ---
    let kept = database
        .create_proxmox_environment(with_token(&unique("kept")))
        .await
        .unwrap();
    let edit = ProxmoxEnvironmentEdit {
        name: unique("kept-renamed"),
        // Same address; checking certificates again is stricter, so fine.
        url: "https://pve.example.com:8006".into(),
        user: "root".into(),
        realm: "pam".into(),
        token_id: Some("lemnos".into()),
        secret: None,
        accept_invalid_certs: false,
    };
    let edited = database
        .edit_proxmox_environment(kept.id, edit.clone())
        .await
        .unwrap();
    assert_eq!(edited.name, edit.name);
    assert_eq!(edited.url, edit.url);
    assert!(!edited.accept_invalid_certs);
    // Left empty: the stored secret stays.
    assert_eq!(edited.login, kept.login);
    assert_eq!(
        database.proxmox_environment(kept.id).await.unwrap(),
        Some(edited.clone())
    );

    // A different token, user, way of signing in or address needs the
    // secret again, and so does no longer checking the certificate.
    for changed in [
        ProxmoxEnvironmentEdit {
            url: "https://elsewhere.example.com:8006".into(),
            ..edit.clone()
        },
        ProxmoxEnvironmentEdit {
            accept_invalid_certs: true,
            ..edit.clone()
        },
        ProxmoxEnvironmentEdit {
            token_id: Some("other".into()),
            ..edit.clone()
        },
        ProxmoxEnvironmentEdit {
            user: "admin".into(),
            ..edit.clone()
        },
        ProxmoxEnvironmentEdit {
            token_id: None,
            ..edit.clone()
        },
    ] {
        assert!(matches!(
            database.edit_proxmox_environment(kept.id, changed).await,
            Err(ProxmoxStoreError::SecretNeeded)
        ));
    }
    assert_eq!(
        database.proxmox_environment(kept.id).await.unwrap(),
        Some(edited)
    );

    // Given a secret, it replaces the old one.
    let replaced = database
        .edit_proxmox_environment(
            kept.id,
            ProxmoxEnvironmentEdit {
                token_id: Some("other".into()),
                secret: Some("ffffffff-0000-1111-2222-333333333333".into()),
                ..edit.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        replaced.login,
        ProxmoxLogin::ApiToken {
            id: "other".into(),
            secret: "ffffffff-0000-1111-2222-333333333333".into(),
        }
    );

    assert!(matches!(
        database
            .edit_proxmox_environment(ProxmoxEnvironmentId(u64::from(u32::MAX) + 12345), edit)
            .await,
        Err(ProxmoxStoreError::NotFound)
    ));
    database.delete_proxmox_environment(kept.id).await.unwrap();

    // --- delete ---
    database.delete_proxmox_environment(lab.id).await.unwrap();
    assert_eq!(database.proxmox_environment(lab.id).await.unwrap(), None);
    // Deleting what is already gone is not an error.
    database.delete_proxmox_environment(lab.id).await.unwrap();
    database
        .delete_proxmox_environment(office.id)
        .await
        .unwrap();
}

#[test]
fn secrets_stay_out_of_debug_output() {
    let shown = format!("{:?}", with_token("lab"));
    assert!(shown.contains("lemnos"));
    assert!(!shown.contains("aaaaaaaa"));

    let shown = format!("{:?}", ProxmoxLogin::Password("hunter2".into()));
    assert!(!shown.contains("hunter2"));
}

#[tokio::test]
async fn sqlite() {
    let path = std::env::temp_dir().join(format!("{}.db", unique("lemnos-test")));
    // `sqlite:` without `//`: after the slashes, `C:` would be read as a host.
    let url = format!("sqlite:{}?mode=rwc", path.display()).replace('\\', "/");

    let database = Database::connect(&url).await.unwrap();
    exercise(&database).await;
    drop(database);

    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[tokio::test]
#[ignore = "needs the dev stack"]
async fn postgres() {
    exercise(&Database::connect(POSTGRES).await.expect(NEEDS_STACK)).await;
}

#[tokio::test]
#[ignore = "needs the dev stack"]
async fn mariadb() {
    exercise(&Database::connect(MARIADB).await.expect(NEEDS_STACK)).await;
}
