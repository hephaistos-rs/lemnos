// What anyone can read without signing in: the landing page, what Lemnos
// does, and the sign-in pages. A group, so it adds nothing to the URL
// (`site/features.rs` -> `/features`).

use topcoat::{
    Result,
    context::Cx,
    router::{Slot, href, layout, page, segment},
    view::{View, view},
};

use crate::{
    app::{auth::current_user, console::machines},
    components::{Theme, footer, heat, site_bar},
};

pub mod auth;
pub mod features;

segment!(kind = Group);

/// Where the source lives.
pub const REPOSITORY: &str = "https://github.com/hephaistos-rs/lemnos";

// Wraps every public page with the bar, the main content area and the footer.
#[layout]
async fn site_shell(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let theme = Theme::current(cx);

    Ok(view! {
        site_bar(theme: theme)
        <main class="flex-1">
            (slot)
        </main>
        footer(theme: theme)
    })
}

/// One row of the made-up list on the landing page.
struct Example {
    state: (&'static str, &'static str),
    id: u32,
    name: &'static str,
    kind: &'static str,
    environment: &'static str,
    // Share of its CPUs in use, and GiB of memory in use; `None` when it is off.
    cpu: Option<f64>,
    memory: Option<f64>,
    memory_total: f64,
}

const RUNNING: (&str, &str) = ("state state-running", "Running");
const STOPPED: (&str, &str) = ("state state-stopped", "Stopped");
const PAUSED: (&str, &str) = ("state state-paused", "Paused");

// Not real machines: the page says so right below the list.
#[rustfmt::skip]
const EXAMPLES: &[Example] = &[
    Example { state: RUNNING, id: 101, name: "staging-api", kind: "VM", environment: "Lab", cpu: Some(0.37), memory: Some(5.2), memory_total: 8.0 },
    Example { state: RUNNING, id: 102, name: "build-runner-01", kind: "VM", environment: "Lab", cpu: Some(0.91), memory: Some(13.6), memory_total: 16.0 },
    Example { state: RUNNING, id: 118, name: "postgres-dev", kind: "Container", environment: "Lab", cpu: Some(0.08), memory: Some(1.1), memory_total: 4.0 },
    Example { state: PAUSED, id: 121, name: "load-test", kind: "VM", environment: "Office", cpu: None, memory: None, memory_total: 8.0 },
    Example { state: STOPPED, id: 130, name: "docs-preview", kind: "Container", environment: "Office", cpu: None, memory: None, memory_total: 2.0 },
];

// What a first config file looks like. Shown as it would be typed.
const CONFIG: &str = r#"database_url = "sqlite:lemnos.db?mode=rwc"

# The first admin, with a password to change after signing in.
[[admin]]
username = "admin"
password = "at-least-twelve-characters"

# A cluster to manage.
[[proxmox]]
name = "Lab"
url = "https://pve.example.com:8006"
user = "root"
realm = "pam"
token_id = "lemnos"
token_secret = "..."

# Everyone who signs in through the directory joins Platform.
[[organization]]
name = "Platform"
providers = ["ldap"]"#;

// `pub` so other modules (like the bar) can link to it with `href!`.
#[page]
pub async fn home(cx: &Cx) -> Result<impl View> {
    let signed_in = current_user(cx).await?.is_some();

    Ok(view! {
        <section class="hero">
            <div class="hero-inner">
                <h1>"Forge your own cloud."</h1>
                <p>
                    "Lemnos is a self-hosted console that lets the people in your organisation see and run their own machines, without an account on the hypervisor behind them."
                </p>
                <div class="flex flex-wrap gap-3">
                    if signed_in {
                        <a href=(href!(machines::page)) class="button button-primary">"Go to dashboard"</a>
                    } else {
                        <a href=(href!(auth::sign_in::page)) class="button button-primary">"Sign in"</a>
                    }
                    <a href=(href!(features::page)) class="button">"See what it does"</a>
                </div>
            </div>
        </section>

        // The console's own list, drawn with the console's own parts.
        <section class="site-section showcase" aria-label="What the list of machines looks like">
            <div class="data data-stack data-machines">
                <table>
                    <thead>
                        <tr>
                            <th>"State"</th>
                            <th>"ID"</th>
                            <th>"Name"</th>
                            <th>"Kind"</th>
                            <th>"Environment"</th>
                            <th>"CPU"</th>
                            <th>"Memory"</th>
                        </tr>
                    </thead>
                    <tbody>
                        for example in EXAMPLES {
                            <tr class=(if example.cpu.is_some() { "" } else { "idle" })>
                                <td data-label="State">
                                    <span class=(example.state.0)>(example.state.1)</span>
                                </td>
                                <td data-label="ID" class="figure meta">(example.id.to_string())</td>
                                <td data-label="Name" class="name">(example.name)</td>
                                <td data-label="Kind" class="meta">(example.kind)</td>
                                <td data-label="Environment" class="meta">(example.environment)</td>
                                <td data-label="CPU" class=(if example.cpu.is_some() { "" } else { "blank" })>
                                    if let Some(cpu) = example.cpu {
                                        heat(fraction: Some(cpu), figure: format!("{:.0}%", cpu * 100.0))
                                    }
                                </td>
                                <td data-label="Memory">
                                    if let Some(memory) = example.memory {
                                        heat(
                                            fraction: Some(memory / example.memory_total),
                                            figure: format!("{memory:.1} / {:.1} GiB", example.memory_total),
                                        )
                                    } else {
                                        heat(fraction: None, figure: format!("{:.1} GiB", example.memory_total))
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            </div>
            <p class="text-meta text-ink-faint">
                "Example data. This is the list every signed-in person gets, limited to the machines of their own organisation."
            </p>
        </section>

        <section class="site-section">
            <div class="site-columns">
                <div>
                    <h2 class="section-title">"Self-service, without the hypervisor"</h2>
                    <p>
                        "People see the machines that are theirs and start, shut down, reboot or stop them. They never get a login on the cluster, and never see anyone else's."
                    </p>
                </div>
                <div>
                    <h2 class="section-title">"The sign-in you already have"</h2>
                    <p>
                        "Local accounts, an LDAP directory, SAML or OpenID Connect single sign-on, and passkeys, with two-factor codes on top. A config file decides which are offered."
                    </p>
                </div>
                <div>
                    <h2 class="section-title">"Yours to run"</h2>
                    <p>
                        "One program on your own hardware, keeping its data in SQLite, PostgreSQL or MariaDB. No hosted service, no account with us, nothing held back behind a paid plan."
                    </p>
                </div>
            </div>
        </section>

        <section class="site-section">
            <div class="site-split">
                <div class="site-prose">
                    <h2 class="text-title font-semibold">"Read a machine like a piece of bronze"</h2>
                    <p>
                        "Lemnos is the island where Hephaistos kept his forge, and the console borrows his metal. A machine in use stays polished. One left alone grows a patina. How hard it works shows as the heat it has run to."
                    </p>
                    <p>
                        "Every state has its own shape as well as its own colour, so the list still reads in greyscale, and red is kept for the one thing that needs you: something has failed."
                    </p>
                </div>
                <dl class="legend">
                    <dt><span class="state state-running">"Running"</span></dt>
                    <dd>"In use. Polished bronze."</dd>
                    <dt><span class="state state-stopped">"Stopped"</span></dt>
                    <dd>"Left alone. A ring of patina."</dd>
                    <dt><span class="state state-paused">"Paused"</span></dt>
                    <dd>"Held mid-work, ready to resume."</dd>
                    <dt><span class="state state-down">"Unreachable"</span></dt>
                    <dd>"Failed. The only red on the page."</dd>
                    <dt>
                        <div class="heat">
                            <div class="heat-track" aria-hidden="true"><span style="--v: 100%"></span></div>
                        </div>
                    </dt>
                    <dd>"Load, on one scale: idle green, through bronze, to the violet of metal run too hot."</dd>
                </dl>
            </div>
        </section>

        <section class="site-section">
            <div class="site-split">
                <div class="site-prose">
                    <h2 class="text-title font-semibold">"Running in three steps"</h2>
                    <ol class="steps">
                        <li>
                            <h3>"Write a config file"</h3>
                            <p>"Name the first admin, the clusters to manage and how people sign in. Everything but the database has a default."</p>
                        </li>
                        <li>
                            <h3>"Start Lemnos"</h3>
                            <p>"It creates its tables on first start and keeps them up to date on every later one."</p>
                        </li>
                        <li>
                            <h3>"Hand it over"</h3>
                            <p>"Give each organisation its environments and the tag that marks its machines. Its people sign in and find them waiting."</p>
                        </li>
                    </ol>
                </div>
                <div class="flex flex-col gap-y-2 min-w-0">
                    <p class="text-meta font-medium text-ink-muted">"lemnos.toml"</p>
                    <pre class="code">(CONFIG)</pre>
                </div>
            </div>
        </section>

        <section class="site-section">
            <div class="site-close">
                <div>
                    <h2 class="text-title font-semibold">"Forge your own."</h2>
                    <p class="text-ink-muted">"Lemnos is part of hephaistos-rs. The source is where to start."</p>
                </div>
                <div class="flex flex-wrap gap-3">
                    <a href=(REPOSITORY) class="button button-primary">"Get the source"</a>
                    if signed_in {
                        <a href=(href!(machines::page)) class="button">"Go to dashboard"</a>
                    } else {
                        <a href=(href!(auth::sign_in::page)) class="button">"Sign in"</a>
                    }
                </div>
            </div>
        </section>
    })
}
