//! Smoke test for a Proxmox VE setup: logs in and prints the version, nodes
//! and guests. It's read-only, so nothing on the server gets changed.
//!
//! Configured through `PROXMOX_URL`, `PROXMOX_USER` and `PROXMOX_REALM`, plus
//! either `PROXMOX_TOKEN_ID` and `PROXMOX_TOKEN_SECRET` or `PROXMOX_PASSWORD`.
//! Set `PROXMOX_INSECURE=1` to accept a self-signed certificate.

use std::env;

use lemnos_proxmox::client::{Credentials, Endpoint, connect};
use lemnos_proxmox::{guests, nodes};
use proxmox_api::version::VersionClient;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = Endpoint::new(
        env::var("PROXMOX_URL")?,
        env::var("PROXMOX_USER")?,
        env::var("PROXMOX_REALM")?,
    )
    .accept_invalid_certs(env::var("PROXMOX_INSECURE").is_ok_and(|v| v == "1"));

    let credentials = match env::var("PROXMOX_TOKEN_ID") {
        Ok(id) => Credentials::ApiToken {
            id,
            secret: env::var("PROXMOX_TOKEN_SECRET")?,
        },
        Err(_) => Credentials::Password(env::var("PROXMOX_PASSWORD")?),
    };

    let client = connect(&endpoint, &credentials).await?;
    let version = VersionClient::new(&client).get().await?;
    println!("Proxmox VE {} ({})", version.version, version.release);

    for node in nodes::list(&client).await? {
        let state = if node.online { "online" } else { "offline" };
        println!("node {} {state}", node.name);
    }
    for guest in guests::list(&client).await? {
        let name = guest.name.as_deref().unwrap_or("-");
        println!(
            "{:?} {} {name} on {}: {:?}",
            guest.id.kind, guest.id.vmid, guest.id.node, guest.status
        );
    }

    Ok(())
}
