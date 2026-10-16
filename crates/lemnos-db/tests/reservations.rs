//! Reserving room for new machines, and how a reservation ends, checked
//! against every supported database.
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
    Database, Limit, MachineRecord, MachineStatus, NewProxmoxEnvironment, OrganizationId,
    OrganizationStoreError, Owner, ProxmoxEnvironmentId, ProxmoxLogin, Reservation,
};

const POSTGRES: &str = "postgres://lemnos:lemnos-dev@localhost:54320/lemnos";
const MARIADB: &str = "mysql://lemnos:lemnos-dev@localhost:33060/lemnos";
const NEEDS_STACK: &str = "run `docker compose -f compose.dev.yml up -d --wait` first";

/// How often the race is run. Once could pass by luck.
const ROUNDS: usize = 10;

fn unique(prefix: &str) -> String {
    format!("{prefix}-{:016x}", rand::random::<u64>())
}

fn reservation(environment: ProxmoxEnvironmentId, owner: Owner) -> Reservation {
    let marker = lemnos_core::marker::generate();
    Reservation {
        environment,
        name: unique("box"),
        owner,
        created_by: None,
        tags: vec!["team-a".into(), marker.clone()],
        marker,
    }
}

async fn environment(database: &Database) -> ProxmoxEnvironmentId {
    database
        .create_proxmox_environment(NewProxmoxEnvironment {
            name: unique("lab"),
            url: "https://pve.example.com:8006".into(),
            user: "root".into(),
            realm: "pam".into(),
            login: ProxmoxLogin::Password("hunter2".into()),
            accept_invalid_certs: false,
        })
        .await
        .unwrap()
        .id
}

/// Two reservations at once for an organisation with one slot left:
/// exactly one gets it.
async fn race(database: &Database) {
    let lab = environment(database).await;
    for _ in 0..ROUNDS {
        let forge = database
            .create_organization(&unique("forge"))
            .await
            .unwrap();
        database.set_machine_quota(forge.id, 2).await.unwrap();
        let owner = Owner::Organization(forge.id);
        // One slot taken already, by a machine that exists.
        database
            .reserve_machine(&reservation(lab, owner), Limit::OrganizationQuota)
            .await
            .unwrap();

        let (first, second) = (database.clone(), database.clone());
        let (a, b) = (reservation(lab, owner), reservation(lab, owner));
        let (a, b) = tokio::join!(
            tokio::spawn(async move { first.reserve_machine(&a, Limit::OrganizationQuota).await }),
            tokio::spawn(async move { second.reserve_machine(&b, Limit::OrganizationQuota).await }),
        );
        let outcomes = [a.unwrap(), b.unwrap()];
        let won = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        let full = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Err(OrganizationStoreError::QuotaReached)))
            .count();
        assert_eq!((won, full), (1, 1), "{outcomes:?}");
        assert_eq!(counted(database, forge.id).await, 2);

        database.delete_organization(forge.id).await.unwrap();
    }

    // The same for a person, with the personal limit of one.
    for _ in 0..ROUNDS {
        let user = database
            .create_user(NewUser::new(Username::parse(&unique("jo")).unwrap()))
            .await
            .unwrap();
        let owner = Owner::Person(user.id);
        let (first, second) = (database.clone(), database.clone());
        let (a, b) = (reservation(lab, owner), reservation(lab, owner));
        let (a, b) = tokio::join!(
            tokio::spawn(async move { first.reserve_machine(&a, Limit::AtMost(1)).await }),
            tokio::spawn(async move { second.reserve_machine(&b, Limit::AtMost(1)).await }),
        );
        let won = [a.unwrap(), b.unwrap()]
            .iter()
            .filter(|outcome| outcome.is_ok())
            .count();
        assert_eq!(won, 1);
        assert_eq!(database.personal_machines(user.id).await.unwrap().len(), 1);
    }
    database.delete_proxmox_environment(lab).await.unwrap();
}

async fn counted(database: &Database, organization: OrganizationId) -> usize {
    database
        .organization_machines(organization)
        .await
        .unwrap()
        .iter()
        .filter(|record| record.status != MachineStatus::Failed)
        .count()
}

/// pending -> active, pending -> failed, and that failing frees the slot.
async fn transitions(database: &Database) {
    let lab = environment(database).await;
    let forge = database
        .create_organization(&unique("forge"))
        .await
        .unwrap();
    database.set_machine_quota(forge.id, 1).await.unwrap();
    let owner = Owner::Organization(forge.id);

    // --- reserved: pending, under its marker, with nothing chosen yet ---
    let wanted = reservation(lab, owner);
    let pending = database
        .reserve_machine(&wanted, Limit::OrganizationQuota)
        .await
        .unwrap();
    assert_eq!(pending.status, MachineStatus::Pending);
    assert_eq!(pending.machine, format!("pending-{}", wanted.marker));
    assert_eq!(pending.marker.as_deref(), Some(wanted.marker.as_str()));
    assert_eq!(pending.tags, wanted.tags);
    assert_eq!(
        (pending.vmid.as_deref(), pending.task.as_deref()),
        (None, None)
    );
    assert!(pending.created_at > 0);
    assert_eq!(
        database
            .machine_record(lab, &pending.machine)
            .await
            .unwrap(),
        Some(pending.clone())
    );
    assert_eq!(
        database
            .pending_machines()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.machine == pending.machine)
            .count(),
        1
    );
    // Pending counts: the only slot is taken.
    assert!(matches!(
        database
            .reserve_machine(&reservation(lab, owner), Limit::OrganizationQuota)
            .await,
        Err(OrganizationStoreError::QuotaReached)
    ));
    // Unlimited (an admin) still gets one, and it counts too.
    let extra = database
        .reserve_machine(&reservation(lab, owner), Limit::Unlimited)
        .await
        .unwrap();
    database
        .fail_machine(lab, &extra.machine, "no", "no")
        .await
        .unwrap();

    // --- an ID and a task are noted as the copy starts ---
    database
        .machine_started(lab, &pending.machine, "105", None)
        .await
        .unwrap();
    let upid = "UPID:pve:0000A1B2:0012C3D4:6720F00D:qmclone:8200:root@pam!lemnos:";
    database
        .machine_started(lab, &pending.machine, "106", Some(upid))
        .await
        .unwrap();
    let started = database
        .machine_record(lab, &pending.machine)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(started.vmid.as_deref(), Some("106"));
    assert_eq!(started.task.as_deref(), Some(upid));
    assert_eq!(started.status, MachineStatus::Pending);

    // --- pending -> active: it goes by its VM ID from now on ---
    // A stale record of a guest deleted behind Lemnos' back held the ID.
    database
        .record_machine(&MachineRecord::active(
            lab,
            "106",
            "old",
            Owner::Organization(forge.id),
            None,
            None,
        ))
        .await
        .unwrap();
    let active = database
        .activate_machine(lab, &pending.machine)
        .await
        .unwrap()
        .expect("it was pending");
    assert_eq!(active.status, MachineStatus::Active);
    assert_eq!(active.machine, "106");
    assert_eq!(
        database
            .machine_record(lab, &pending.machine)
            .await
            .unwrap(),
        None
    );
    let stored = database.machine_record(lab, "106").await.unwrap().unwrap();
    assert_eq!(stored.status, MachineStatus::Active);
    assert_eq!(stored.name, wanted.name);
    assert_eq!(stored.marker.as_deref(), Some(wanted.marker.as_str()));
    // Done once only.
    assert_eq!(
        database
            .activate_machine(lab, &pending.machine)
            .await
            .unwrap(),
        None
    );
    // An active machine can't be failed.
    assert!(
        !database
            .fail_machine(lab, "106", "late", "late")
            .await
            .unwrap()
    );
    assert_eq!(counted(database, forge.id).await, 1);

    // --- pending -> failed frees the slot ---
    database.forget_machine(lab, "106").await.unwrap();
    let doomed = database
        .reserve_machine(&reservation(lab, owner), Limit::OrganizationQuota)
        .await
        .unwrap();
    assert!(
        database
            .fail_machine(
                lab,
                &doomed.machine,
                "The copy failed.",
                "HTTP 500: disk full"
            )
            .await
            .unwrap()
    );
    let failed = database
        .machine_record(lab, &doomed.machine)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, MachineStatus::Failed);
    assert_eq!(failed.failure.as_deref(), Some("The copy failed."));
    assert_eq!(
        failed.failure_detail.as_deref(),
        Some("HTTP 500: disk full")
    );
    // Failing again, or activating, does nothing.
    assert!(
        !database
            .fail_machine(lab, &doomed.machine, "x", "x")
            .await
            .unwrap()
    );
    assert_eq!(
        database
            .activate_machine(lab, &doomed.machine)
            .await
            .unwrap(),
        None
    );
    // The failed one is still there for its owner to read, but the slot is free.
    let next = database
        .reserve_machine(&reservation(lab, owner), Limit::OrganizationQuota)
        .await
        .unwrap();
    assert_eq!(next.status, MachineStatus::Pending);
    // And its owner can remove the failed entry.
    database.forget_machine(lab, &doomed.machine).await.unwrap();
    assert_eq!(
        database.machine_record(lab, &doomed.machine).await.unwrap(),
        None
    );

    database.delete_organization(forge.id).await.unwrap();
    database.delete_proxmox_environment(lab).await.unwrap();
}

async fn exercise(database: &Database) {
    transitions(database).await;
    race(database).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite() {
    let path = std::env::temp_dir().join(format!("{}.db", unique("lemnos-test")));
    let url = format!("sqlite:{}?mode=rwc", path.display()).replace('\\', "/");

    let database = Database::connect(&url).await.unwrap();
    exercise(&database).await;
    drop(database);

    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the dev stack"]
async fn postgres() {
    exercise(&Database::connect(POSTGRES).await.expect(NEEDS_STACK)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the dev stack"]
async fn mariadb() {
    exercise(&Database::connect(MARIADB).await.expect(NEEDS_STACK)).await;
}
