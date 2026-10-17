//! Storing projects, machine owners, quotas and the personal environment,
//! checked against every supported database.
//!
//! SQLite always runs. PostgreSQL and MariaDB need the containers from
//! `compose.dev.yml`:
//!
//! ```text
//! docker compose -f compose.dev.yml up -d --wait
//! cargo test -p lemnos-db -- --ignored
//! ```

use lemnos_auth::{NewUser, Store, Username};
use lemnos_db::{
    Database, MachineRecord, NewProxmoxEnvironment, OrganizationStoreError, Owner, ProxmoxLogin,
    Role,
};

const POSTGRES: &str = "postgres://lemnos:lemnos-dev@localhost:54320/lemnos";
const MARIADB: &str = "mysql://lemnos:lemnos-dev@localhost:33060/lemnos";
const NEEDS_STACK: &str = "run `docker compose -f compose.dev.yml up -d --wait` first";

/// A value no earlier test run has used. The PostgreSQL and MariaDB
/// containers keep their rows between runs.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{:016x}", rand::random::<u64>())
}

fn environment(name: &str) -> NewProxmoxEnvironment {
    NewProxmoxEnvironment {
        name: name.to_owned(),
        url: "https://pve.example.com:8006".into(),
        user: "root".into(),
        realm: "pam".into(),
        login: ProxmoxLogin::Password("hunter2".into()),
        accept_invalid_certs: false,
    }
}

/// The records with their timestamps cleared, to compare with what went in.
fn untimed(records: Vec<MachineRecord>) -> Vec<MachineRecord> {
    records
        .into_iter()
        .map(|record| {
            assert!(record.created_at > 0 && record.updated_at >= record.created_at);
            MachineRecord {
                created_at: 0,
                updated_at: 0,
                ..record
            }
        })
        .collect()
}

async fn exercise(database: &Database) {
    let alice = database
        .create_user(NewUser::new(Username::parse(&unique("alice")).unwrap()))
        .await
        .unwrap();
    let forge = database
        .create_organization(&unique("forge"))
        .await
        .unwrap();
    database
        .set_membership(alice.id, forge.id, Role::Member)
        .await
        .unwrap();
    let lab = database
        .create_proxmox_environment(environment(&unique("lab")))
        .await
        .unwrap();

    // --- projects ---
    let bellows = database.create_project(forge.id, "bellows").await.unwrap();
    assert!(matches!(
        database.create_project(forge.id, "bellows").await,
        Err(OrganizationStoreError::NameTaken)
    ));
    assert_eq!(
        database.projects(forge.id).await.unwrap(),
        std::slice::from_ref(&bellows)
    );
    database
        .add_project_member(bellows.id, alice.id)
        .await
        .unwrap();
    database
        .add_project_member(bellows.id, alice.id)
        .await
        .unwrap();
    assert_eq!(
        database.projects_of(alice.id).await.unwrap(),
        std::slice::from_ref(&bellows)
    );
    assert_eq!(database.project_members(bellows.id).await.unwrap().len(), 1);

    // --- quotas ---
    assert_eq!(database.machine_quota(forge.id).await.unwrap(), 0);
    database.set_machine_quota(forge.id, 3).await.unwrap();
    assert_eq!(database.machine_quota(forge.id).await.unwrap(), 3);

    // --- machines ---
    let personal = MachineRecord::active(
        lab.id,
        "100",
        "scratch",
        Owner::Person(alice.id),
        Some(alice.id),
        Some(lemnos_core::marker::generate()),
    );
    let shared = MachineRecord::active(
        lab.id,
        "101",
        "build",
        Owner::Project {
            organization: forge.id,
            project: bellows.id,
        },
        Some(alice.id),
        // As records from before markers have it.
        None,
    );
    database.record_machine(&personal).await.unwrap();
    database.record_machine(&shared).await.unwrap();
    assert_eq!(
        untimed(database.personal_machines(alice.id).await.unwrap()),
        std::slice::from_ref(&personal)
    );
    assert_eq!(
        untimed(database.organization_machines(forge.id).await.unwrap()),
        std::slice::from_ref(&shared)
    );
    assert_eq!(
        database.environment_machines(lab.id).await.unwrap().len(),
        2
    );

    // --- markers ---
    // Stored and read back as given, including none.
    assert!(
        personal
            .marker
            .as_deref()
            .is_some_and(lemnos_core::marker::is_marker)
    );
    assert_eq!(
        database.personal_machines(alice.id).await.unwrap()[0].marker,
        personal.marker
    );
    assert_eq!(
        database.organization_machines(forge.id).await.unwrap()[0].marker,
        None
    );
    // Confirmed by an admin: it gets one.
    let confirmed = lemnos_core::marker::generate();
    database
        .set_machine_marker(lab.id, "101", &confirmed)
        .await
        .unwrap();
    assert_eq!(
        database.organization_machines(forge.id).await.unwrap()[0]
            .marker
            .as_deref(),
        Some(confirmed.as_str())
    );

    // A deleted project leaves its machines to the organisation.
    database.delete_project(bellows.id).await.unwrap();
    assert_eq!(
        database.organization_machines(forge.id).await.unwrap()[0].owner,
        Owner::Organization(forge.id)
    );
    assert!(database.projects_of(alice.id).await.unwrap().is_empty());

    database.forget_machine(lab.id, "100").await.unwrap();
    assert!(
        database
            .personal_machines(alice.id)
            .await
            .unwrap()
            .is_empty()
    );

    // Leaving the organisation leaves its projects too.
    let anvil = database.create_project(forge.id, "anvil").await.unwrap();
    database
        .add_project_member(anvil.id, alice.id)
        .await
        .unwrap();
    database
        .remove_membership(alice.id, forge.id)
        .await
        .unwrap();
    assert!(database.projects_of(alice.id).await.unwrap().is_empty());

    // --- personal environment ---
    database
        .set_personal_environment(Some(lab.id))
        .await
        .unwrap();
    assert_eq!(database.personal_environment().await.unwrap(), Some(lab.id));

    // Removing things takes what hangs off them along.
    database.delete_organization(forge.id).await.unwrap();
    assert!(
        database
            .environment_machines(lab.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(database.project(anvil.id).await.unwrap().is_none());
    database.delete_proxmox_environment(lab.id).await.unwrap();
    assert_eq!(database.personal_environment().await.unwrap(), None);
    database.set_personal_environment(None).await.unwrap();
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
