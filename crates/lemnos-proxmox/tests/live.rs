//! Read-only checks against a real Proxmox VE, skipped unless asked for.
//!
//! It signs in with the token ID in the full form Proxmox shows, and lists
//! what is there. It changes nothing. Set the variables, then run it:
//!
//! ```text
//! set -a; . ./.env; set +a
//! cargo test -p lemnos-proxmox --test live -- --ignored
//! ```
//!
//! - `PROXMOX_URL`, e.g. `https://pve.example.com:8006`
//! - `PROXMOX_TOKEN_ID`, e.g. `root@pam!lemnos`
//! - `PROXMOX_TOKEN`, the token's secret
//! - `PROXMOX_ACCEPT_INVALID_CERTS=1` for a self-signed certificate (optional)

use lemnos_proxmox::{
    client::{Credentials, Endpoint, connect},
    guests, nodes, token,
};

struct Live {
    url: String,
    token_id: String,
    secret: String,
    accept_invalid_certs: bool,
}

impl Live {
    fn from_env() -> Self {
        let var = |name: &str| {
            std::env::var(name).unwrap_or_else(|_| panic!("set {name}; see the top of this file"))
        };
        Self {
            url: var("PROXMOX_URL"),
            token_id: var("PROXMOX_TOKEN_ID"),
            secret: var("PROXMOX_TOKEN"),
            accept_invalid_certs: std::env::var("PROXMOX_ACCEPT_INVALID_CERTS")
                .is_ok_and(|value| value == "1" || value == "true"),
        }
    }

    /// An error as text with every value from the environment taken out:
    /// errors from the HTTP client name the URL they were sending to.
    fn redact(&self, error: impl std::fmt::Display) -> String {
        error
            .to_string()
            .replace(self.url.trim_end_matches('/'), "<PROXMOX_URL>")
            .replace(&self.secret, "<PROXMOX_TOKEN>")
            .replace(&self.token_id, "<PROXMOX_TOKEN_ID>")
    }
}

#[tokio::test]
#[ignore = "needs a real Proxmox; see the top of this file"]
async fn signs_in_with_the_full_token_id_and_lists() {
    let live = Live::from_env();
    let owner = token::split(&live.token_id, "", "")
        .unwrap_or_else(|error| panic!("PROXMOX_TOKEN_ID: {}", live.redact(error)));
    let endpoint = Endpoint::new(&live.url, &owner.user, &owner.realm)
        .accept_invalid_certs(live.accept_invalid_certs);
    let credentials = Credentials::ApiToken {
        id: owner.name.clone(),
        secret: live.secret.clone(),
    };

    let client = connect(&endpoint, &credentials)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "signing in failed ({:?}): {}",
                error.failure(),
                live.redact(&error)
            )
        });

    let nodes = nodes::list(&client)
        .await
        .unwrap_or_else(|error| panic!("listing nodes: {}", live.redact(error)));
    assert!(!nodes.is_empty(), "a cluster has at least one node");
    let guests = guests::list(&client)
        .await
        .unwrap_or_else(|error| panic!("listing guests: {}", live.redact(error)));
    // Counts only: what the variables hold stays out of the output.
    println!(
        "signed in with the full token ID; {} node(s), {} guest(s)",
        nodes.len(),
        guests.len()
    );
}
