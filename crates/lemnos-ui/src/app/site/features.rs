// /features: what Lemnos does today, and what it does not do yet.

use topcoat::{
    Result,
    context::Cx,
    router::{href, page},
    view::{View, view},
};

use crate::app::{
    auth::{current_user, sign_in},
    console::machines,
};

/// A group of things Lemnos does, as (title, one line each).
type Group = (&'static str, &'static [&'static str]);

const BUILT: &[Group] = &[
    (
        "Machines",
        &[
            "One list of every VM and container you may see, across all environments.",
            "State, CPU and memory for each, compared at a glance.",
            "Start, shut down, reboot, stop and resume, each a deliberate button with its result reported in plain words.",
            "An environment that can't be reached is a message above the list, never a broken page.",
        ],
    ),
    (
        "Environments",
        &[
            "Proxmox VE clusters, signed in to with an API token or a password.",
            "Declared in the config file or added in the console.",
            "Per environment: its nodes, its machines and its storage.",
        ],
    ),
    (
        "Organisations and roles",
        &[
            "Several organisations on the same clusters; a person can belong to more than one.",
            "An organisation sees the machines in its environments that carry its tag.",
            "Owners manage members, members use machines, viewers only look.",
            "Instance admins connect environments and create organisations.",
        ],
    ),
    (
        "Signing in",
        &[
            "Local accounts with a password policy and throttling of failed attempts.",
            "LDAP directories, SAML, OpenID Connect and plain OAuth2.",
            "Passkeys, and two-factor codes from an authenticator app.",
            "Each method is switched on in the config file; none needs a rebuild.",
        ],
    ),
    (
        "Running it",
        &[
            "Self-hosted, as one program.",
            "SQLite, PostgreSQL or MariaDB, chosen by a connection URL.",
            "Pages are rendered on the server and work without scripts; only passkeys need one.",
            "Light and dark, following the device or your own choice.",
        ],
    ),
];

// Said out loud, so nobody installs Lemnos for something it can't do.
const NOT_YET: &[&str] = &[
    "Creating, resizing or deleting machines.",
    "A console (screen and keyboard) in the browser.",
    "Showing a machine's network address.",
    "Providers other than Proxmox VE.",
    "Quotas and requests.",
];

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let signed_in = current_user(cx).await?.is_some();

    Ok(view! {
        <div class="page">
            <div class="page-head">
                <div>
                    <h1>"Features"</h1>
                    <p>"What Lemnos does today, and what it does not do yet."</p>
                </div>
                if signed_in {
                    <a href=(href!(machines::page)) class="button button-primary">"Go to dashboard"</a>
                } else {
                    <a href=(href!(sign_in::page)) class="button button-primary">"Sign in"</a>
                }
            </div>

            <div class="feature-list">
                for (title, lines) in BUILT {
                    <section>
                        <h2 class="section-title">(*title)</h2>
                        <ul>
                            for line in *lines {
                                <li>(*line)</li>
                            }
                        </ul>
                    </section>
                }
                <section>
                    <h2 class="section-title text-ink-muted">"Not yet"</h2>
                    <ul class="text-ink-muted">
                        for line in NOT_YET {
                            <li>(*line)</li>
                        }
                    </ul>
                </section>
            </div>
        </div>
    })
}
