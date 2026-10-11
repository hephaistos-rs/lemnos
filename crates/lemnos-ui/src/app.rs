// Root of the route tree: this module maps to `/`, and every module below it
// adds one URL segment (e.g. `app/vms.rs` -> `/vms`).

use lemnos_db::ProxmoxEnvironment;
use lemnos_proxmox::guests::Guest;
use serde::Deserialize;
use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    context::Cx,
    cookie::{Cookies, Key, RouterBuilderCookieExt, SameSite, cookies, time::Duration},
    font::{self, Font, fontsource::fontsource_font},
    router::{
        OriginPolicy, Router, RouterBuilderDiscoverExt, Slot,
        content::Form,
        error::{SeeOther, see_other},
        header, href, layout, module_router, page, query_params,
        request::headers,
        route,
    },
    session::{RouterBuilderSessionExt, SessionConfig},
    tailwind,
    view::{View, view},
};

use crate::components::{Theme, heat, notice, problem, shell_bar, theme_switch};

pub mod account;
pub mod auth;
pub mod proxmox;

use proxmox::environment::{
    self, Environment, fraction, is_running, kind_label, percent, power_buttons, power_notice,
    power_problem, row_class, state_label, usage,
};

// Self-hosted (bundled as assets) instead of loaded from Google Fonts.
// Only these weights/styles are bundled; add more here if the CSS needs them.
// Commissioner is drawn by a Greek designer and carries the Greek letters of
// the wordmark; JetBrains Mono is only for IDs, addresses and figures.
const COMMISSIONER: Font = fontsource_font!(
    COMMISSIONER,
    weight: [400, 500, 600, 700],
    style: [Normal],
    subset: [Latin, Greek],
    host: Asset,
);
const JETBRAINS_MONO: Font = fontsource_font!(
    JETBRAINS_MONO,
    weight: [400, 500],
    style: [Normal],
    host: Asset,
);

pub fn router(auth: auth::AuthState) -> Router {
    module_router!()
        .discover()
        // Serves bundled files (like the Tailwind stylesheet) under /_topcoat/assets.
        .assets(AssetBundle::load().unwrap())
        // Session tokens travel in a cookie, so sessions need cookie support.
        .cookies()
        .sessions(SessionConfig::default())
        .app_context(auth)
        // Encrypts the cookies that carry a half-finished sign-in. A new key
        // per start means those (not sessions) are dropped on restart.
        .app_context(Key::generate())
        // POSTs from other sites are refused, except a SAML provider
        // delivering its signed answer.
        .origin_policy(OriginPolicy::new().exempt_paths(["/auth/sso/{provider}/acs"]))
        .build()
}

/// The theme stored by the switch in the bar, or `System` if none was.
fn theme(cx: &Cx) -> Theme {
    cookies(cx)
        .get(Theme::COOKIE)
        .map_or(Theme::System, |cookie| Theme::parse(cookie.value()))
}

// Wraps every page in this module and below it with the bar, the main
// content area and the footer.
#[layout]
async fn app_shell(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let theme = theme(cx);

    Ok(view! {
        <!DOCTYPE html>
        // `data-theme` overrides the device's light or dark setting; see app.css.
        <html lang="en" data-theme=(theme.name())>
            <head>
                <title>"Lemnos"</title>
                <meta charset="UTF-8">
                <meta name="viewport" content="width=device-width, initial-scale=1.0">
                topcoat::dev::script()
                font::link(font: COMMISSIONER)
                font::link(font: JETBRAINS_MONO)
                <link rel="stylesheet" href=(tailwind::stylesheet!())>
            </head>
            <body class="min-h-screen flex flex-col">
                shell_bar(theme: theme)
                <main class="flex-1">
                    (slot)
                </main>
                <footer class="footer">
                    <p>"Lemnos. Forge your own cloud."</p>
                    // Shown here only on narrow screens; the bar has no room for it there.
                    theme_switch(theme: theme)
                </footer>
            </body>
        </html>
    })
}

#[derive(Deserialize)]
pub struct ThemeForm {
    theme: String,
}

// POST /theme: remembers the theme picked with the switch, then shows the page the
// visitor was on again.
#[route(POST "./theme")]
pub async fn set_theme(cx: &Cx, Form(input): Form<ThemeForm>) -> Result<SeeOther> {
    let chosen = Theme::parse(&input.theme);
    let jar = cookies(cx)
        .default_path("/")
        .default_same_site(SameSite::Lax)
        .default_max_age(Duration::days(365));
    if chosen == Theme::System {
        jar.remove(Theme::COOKIE);
    } else {
        jar.add((Theme::COOKIE, chosen.name()));
    }

    // Only the path of the page is reused, so this can't send anyone to
    // another site.
    let back = headers(cx)
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<topcoat::router::Uri>().ok())
        .and_then(|uri| uri.path_and_query().map(|path| path.as_str().to_owned()))
        .filter(|path| path.starts_with('/') && !path.starts_with("//"))
        .unwrap_or_else(|| "/".to_owned());
    Ok(see_other(back))
}

#[query_params(error = bad_request)]
struct Query {
    notice: Option<String>,
    error: Option<String>,
    // Which power button it was, and on which machine.
    action: Option<String>,
    machine: Option<String>,
}

/// The environment's name and why it could not be read.
struct Unreachable {
    message: String,
    reason: String,
    // Its own page, and the words for the link to it.
    page: String,
    open: String,
}

/// One environment and what could be read from it: its guests, or why not.
struct Loaded {
    environment: ProxmoxEnvironment,
    guests: std::result::Result<Vec<Guest>, String>,
}

async fn load(environment: ProxmoxEnvironment) -> Loaded {
    let guests = match proxmox::connect(&environment).await {
        Ok(client) => environment::overview(&client)
            .await
            .map(|overview| overview.guests),
        Err(error) => Err(error),
    };
    let guests = guests.map_err(|error| {
        eprintln!(
            "Proxmox environment `{}` failed: {error:?}",
            environment.name
        );
        error.to_string()
    });
    Loaded {
        environment,
        guests,
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

// `pub` so other modules (like the bar) can link to it with `href!`.
//
// Every machine in every environment. Visitors are sent to the sign-in page.
#[page]
pub async fn home(cx: &Cx) -> Result<impl View> {
    proxmox::require_user(cx).await?;
    let query = query_params::<Query>(cx)?;

    let mut loaded = Vec::new();
    // One after the other: there are only ever a handful of environments.
    for environment in proxmox::database(cx).proxmox_environments().await? {
        loaded.push(load(environment).await);
    }
    let machines: usize = loaded
        .iter()
        .filter_map(|entry| entry.guests.as_ref().ok())
        .map(Vec::len)
        .sum();
    let running = loaded
        .iter()
        .filter_map(|entry| entry.guests.as_ref().ok())
        .flatten()
        .filter(|guest| is_running(guest))
        .count();
    let summary = format!(
        "{} in {}, {running} running.",
        plural(machines, "machine", "machines"),
        plural(loaded.len(), "environment", "environments"),
    );
    let power_done = query
        .notice
        .as_deref()
        .map(|code| power_notice(code, query.action.as_deref(), query.machine.as_deref()));
    let power_failed = power_problem(query.machine.as_deref());
    let unreachable: Vec<Unreachable> = loaded
        .iter()
        .filter_map(|entry| {
            Some(Unreachable {
                reason: entry.guests.as_ref().err()?.clone(),
                message: format!(
                    "Lemnos can't reach {}, so its machines are missing below.",
                    entry.environment.name,
                ),
                page: href!(environment::page, Environment(entry.environment.id.0))
                    .resolve(cx)
                    .to_string(),
                open: format!("Check {}", entry.environment.name),
            })
        })
        .collect();

    Ok(view! {
        <div class="page">
            <div class="page-head">
                <div>
                    <h1>"Machines"</h1>
                    if !loaded.is_empty() {
                        <p>(summary.as_str())</p>
                    }
                </div>
            </div>

            if let Some(message) = &power_done {
                notice(tone: "success", message: message.as_str())
            }
            if let Some(reason) = &query.error {
                problem(message: power_failed.as_str(), detail: reason.as_str(), link: ("", ""))
            }
            for entry in &unreachable {
                problem(
                    message: entry.message.as_str(),
                    detail: entry.reason.as_str(),
                    link: (entry.page.as_str(), entry.open.as_str()),
                )
            }

            if loaded.is_empty() {
                <div class="panel items-start">
                    <h2 class="section-title">"No environments yet"</h2>
                    <p class="text-ink-muted max-w-[65ch]">
                        "Machines show up here once Lemnos knows where they run. Connect the first environment to get started."
                    </p>
                    <a href=(href!(proxmox::page)) class="button button-primary">
                        "Add an environment"
                    </a>
                </div>
            } else {
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
                                <th><span class="sr-only">"Power"</span></th>
                            </tr>
                        </thead>
                        <tbody>
                            if machines == 0 {
                                <tr>
                                    <td colspan="8" class="empty wide">
                                        "No VMs or containers to show."
                                    </td>
                                </tr>
                            }
                            for entry in &loaded {
                                if let Ok(guests) = &entry.guests {
                                    for guest in guests {
                                        <tr class=(row_class(guest))>
                                            <td data-label="State">
                                                <span class=(state_label(guest).0)>
                                                    (state_label(guest).1)
                                                </span>
                                            </td>
                                            <td data-label="ID" class="figure meta">
                                                (guest.id.vmid.to_string())
                                            </td>
                                            <td data-label="Name" class="name">
                                                (guest.name.as_deref().unwrap_or("-"))
                                            </td>
                                            <td data-label="Kind" class="meta">(kind_label(guest.id.kind))</td>
                                            <td data-label="Environment" class="meta">
                                                <a
                                                    href=(href!(
                                                        environment::page,
                                                        Environment(entry.environment.id.0),
                                                    ))
                                                    class="link"
                                                >
                                                    (entry.environment.name.as_str())
                                                </a>
                                            </td>
                                            <td data-label="CPU" class=(if is_running(guest) { "" } else { "blank" })>
                                                if is_running(guest) {
                                                    heat(fraction: guest.cpu, figure: percent(guest.cpu))
                                                }
                                            </td>
                                            <td data-label="Memory">
                                                if is_running(guest) {
                                                    heat(
                                                        fraction: fraction(guest.mem, guest.max_mem),
                                                        figure: usage(guest.mem, guest.max_mem),
                                                    )
                                                } else {
                                                    heat(fraction: None, figure: usage(None, guest.max_mem))
                                                }
                                            </td>
                                            <td class="wide">
                                                power_buttons(
                                                    environment: entry.environment.id.0,
                                                    guest: guest,
                                                    back: "machines",
                                                )
                                            </td>
                                        </tr>
                                    }
                                }
                            }
                        </tbody>
                    </table>
                </div>
            }
        </div>
    })
}
