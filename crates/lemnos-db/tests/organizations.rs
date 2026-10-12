//! Storing organisations, members, grants and admins, checked against
//! every supported database.
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
    Database, Grant, NewProxmoxEnvironment, OrganizationId, OrganizationStoreError, ProxmoxLogin,
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

async fn exercise(database: &Database) {
    let alice = database
        .create_user(NewUser::new(Username::parse(&unique("alice")).unwrap()))
        .await
        .unwrap();
    let bob = database
        .create_user(NewUser::new(Username::parse(&unique("bob")).unwrap()))
        .await
        .unwrap();

    // --- organisations ---
    let forge_name = unique("forge");
    let forge = database.create_organization(&forge_name).await.unwrap();
    let anvil = database
        .create_organization(&unique("anvil"))
        .await
        .unwrap();
    assert_eq!(
        database.organization(forge.id).await.unwrap(),
        Some(forge.clone())
    );
    assert_eq!(
        database.organization_by_name(&forge_name).await.unwrap(),
        Some(forge.clone())
    );
    assert!(matches!(
        database.create_organization(&forge_name).await,
        Err(OrganizationStoreError::NameTaken)
    ));

    // --- members ---
    assert!(database.memberships_of(alice.id).await.unwrap().is_empty());
    database
        .set_membership(alice.id, forge.id, Role::Member)
        .await
        .unwrap();
    database
        .set_membership(alice.id, anvil.id, Role::Viewer)
        .await
        .unwrap();
    database
        .set_membership(bob.id, forge.id, Role::Owner)
        .await
        .unwrap();
    // Setting it again changes the role instead of adding a second one.
    database
        .set_membership(alice.id, forge.id, Role::Owner)
        .await
        .unwrap();

    let mut of_alice: Vec<_> = database
        .memberships_of(alice.id)
        .await
        .unwrap()
        .into_iter()
        .map(|membership| (membership.organization.id, membership.role))
        .collect();
    of_alice.sort_by_key(|(organization, _)| organization.0);
    let mut expected = [(forge.id, Role::Owner), (anvil.id, Role::Viewer)];
    expected.sort_by_key(|(organization, _)| organization.0);
    assert_eq!(of_alice, expected);

    let in_forge: Vec<_> = database
        .members(forge.id)
        .await
        .unwrap()
        .into_iter()
        .map(|member| (member.user, member.role))
        .collect();
    assert_eq!(in_forge.len(), 2);
    assert!(in_forge.contains(&(alice.id, Role::Owner)));
    assert!(in_forge.contains(&(bob.id, Role::Owner)));

    database.remove_membership(bob.id, forge.id).await.unwrap();
    assert!(database.memberships_of(bob.id).await.unwrap().is_empty());
    // Removing someone who is not in it is not an error.
    database.remove_membership(bob.id, forge.id).await.unwrap();

    // --- grants ---
    let lab = database
        .create_proxmox_environment(environment(&unique("lab")))
        .await
        .unwrap();
    database
        .grant_environment(forge.id, lab.id, "org-forge")
        .await
        .unwrap();
    assert_eq!(
        database.grants(forge.id).await.unwrap(),
        [Grant {
            environment: lab.id,
            tag: "org-forge".into()
        }]
    );
    // Granting it again changes the tag.
    database
        .grant_environment(forge.id, lab.id, "")
        .await
        .unwrap();
    assert_eq!(
        database.grants(forge.id).await.unwrap(),
        [Grant {
            environment: lab.id,
            tag: String::new()
        }]
    );
    assert!(database.grants(anvil.id).await.unwrap().is_empty());
    // A removed environment takes its grants with it.
    database.delete_proxmox_environment(lab.id).await.unwrap();
    assert!(database.grants(forge.id).await.unwrap().is_empty());

    // --- admins ---
    assert!(!database.is_admin(alice.id).await.unwrap());
    database.set_admin(alice.id, true).await.unwrap();
    database.set_admin(alice.id, true).await.unwrap();
    assert!(database.is_admin(alice.id).await.unwrap());
    let accounts = database.accounts().await.unwrap();
    assert!(
        accounts
            .iter()
            .any(|account| account.user == alice.id && account.admin)
    );
    assert!(
        accounts
            .iter()
            .any(|account| account.user == bob.id && !account.admin)
    );
    database.set_admin(alice.id, false).await.unwrap();
    assert!(!database.is_admin(alice.id).await.unwrap());

    // --- delete ---
    database.delete_organization(forge.id).await.unwrap();
    assert_eq!(database.organization(forge.id).await.unwrap(), None);
    let left: Vec<_> = database
        .memberships_of(alice.id)
        .await
        .unwrap()
        .into_iter()
        .map(|membership| membership.organization.id)
        .collect();
    assert_eq!(left, [anvil.id]);
    // Deleting what is already gone is not an error.
    database.delete_organization(forge.id).await.unwrap();
    database.delete_organization(anvil.id).await.unwrap();
    database
        .delete_organization(OrganizationId(u64::from(u32::MAX) + 54321))
        .await
        .unwrap();
}

#[test]
fn roles_round_trip_and_nest() {
    for role in Role::ALL {
        assert_eq!(Role::parse(role.as_str()), Some(role));
    }
    assert_eq!(Role::parse("admin"), None);
    assert!(Role::Owner.can_manage_members() && Role::Owner.can_operate());
    assert!(!Role::Member.can_manage_members() && Role::Member.can_operate());
    assert!(!Role::Viewer.can_manage_members() && !Role::Viewer.can_operate());
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
