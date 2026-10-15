// Reusable pieces used inside pages. These are not routes.

use topcoat::{
    Result,
    context::Cx,
    cookie::{Cookies, cookies},
    router::{href, request::uri},
    view::{View, component, view},
};

use crate::{
    access::access,
    app::{
        auth::{self, current_user},
        console::{self, environments, machines, organizations, settings, users},
        site,
    },
};

/// Which theme the visitor picked with the theme switch. `System`
/// follows the device and is what everyone starts with.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    /// Cookie holding the choice. Absent means `System`.
    pub const COOKIE: &str = "theme";

    pub fn parse(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    /// The theme stored by the theme switch, or `System` if none was.
    pub fn current(cx: &Cx) -> Self {
        cookies(cx)
            .get(Self::COOKIE)
            .map_or(Self::System, |cookie| Self::parse(cookie.value()))
    }

    /// The value for the cookie, the form and `<html data-theme>`.
    pub fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

fn pressed(on: bool) -> &'static str {
    if on { "true" } else { "false" }
}

fn current(on: bool) -> &'static str {
    if on { "page" } else { "false" }
}

// The name, with the island's own spelling beside it. `to` is where it leads.
#[component]
async fn wordmark(to: String) -> Result<impl View> {
    Ok(view! {
        <a href=(to) class="wordmark" aria-label="Lemnos">
            "Lemnos"
            <span class="wordmark-greek" aria-hidden="true">"ΛΗΜΝΟΣ"</span>
        </a>
    })
}

// The verdigris bar of the console: where you are, where you can go, whose
// machines you are looking at, and who you are. `cx` is passed in by
// Topcoat: call it as `shell_bar(theme: ...)`.
#[component]
pub async fn shell_bar(cx: &Cx, theme: Theme) -> Result<impl View> {
    let access = access(cx).await?;
    let path = uri(cx).path();
    // What the user can look at, as (value for the form, name).
    let scopes = access.scopes();
    let looking_at = access.scope_choice();

    Ok(view! {
        <header class="shell">
            <div class="shell-inner">
                wordmark(to: href!(machines::page).resolve(cx))
                <nav class="shell-nav" aria-label="Main">
                    <a
                        href=(href!(machines::page))
                        aria-current=(current(path.starts_with("/machines")))
                    >
                        "Machines"
                    </a>
                    if access.admin {
                        <a
                            href=(href!(environments::page))
                            aria-current=(current(path.starts_with("/environments")))
                        >
                            "Environments"
                        </a>
                    }
                    <a
                        href=(href!(organizations::page))
                        aria-current=(current(path.starts_with("/organizations")))
                    >
                        "Organisations"
                    </a>
                    if access.admin {
                        <a
                            href=(href!(users::page))
                            aria-current=(current(path.starts_with("/users")))
                        >
                            "Users"
                        </a>
                    }
                </nav>
                <div class="shell-end">
                    // Only worth a menu when there is something to switch to.
                    if scopes.len() > 1 {
                        <details class="menu">
                            <summary aria-label=(format!("Looking at {}. Switch", access.scope_name()))>
                                (access.scope_name())
                            </summary>
                            // Buttons, not links: the choice is stored, so it must be a POST.
                            <form method="post" action=(href!(console::choose_scope))>
                                for (value, name) in &scopes {
                                    <button
                                        type="submit"
                                        name="organization"
                                        value=(value.as_str())
                                        aria-current=(if *value == looking_at { "true" } else { "false" })
                                    >
                                        (name.as_str())
                                    </button>
                                }
                            </form>
                        </details>
                    } else if let Some((_, name)) = scopes.first() {
                        <span class="shell-scope">(name.as_str())</span>
                    }
                    theme_switch(theme: theme)
                    <a href=(href!(settings::account::page)) class="shell-link">(&access.user.name)</a>
                    // A form, not a link: signing out changes state, so it must be a POST.
                    <form method="post" action=(href!(auth::sign_out::sign_out))>
                        <button type="submit" class="button button-sm">"Sign out"</button>
                    </form>
                </div>
            </div>
        </header>
    })
}

// The same bar on the public pages: what Lemnos is, and the way in.
#[component]
pub async fn site_bar(cx: &Cx, theme: Theme) -> Result<impl View> {
    let user = current_user(cx).await?;
    let path = uri(cx).path();

    Ok(view! {
        <header class="shell">
            <div class="shell-inner">
                wordmark(to: href!(site::home).resolve(cx))
                <nav class="shell-nav" aria-label="Main">
                    <a
                        href=(href!(site::features::page))
                        aria-current=(current(path.starts_with("/features")))
                    >
                        "Features"
                    </a>
                </nav>
                <div class="shell-end">
                    theme_switch(theme: theme)
                    if user.is_some() {
                        <a href=(href!(machines::page)) class="button button-sm">"Go to dashboard"</a>
                    } else {
                        <a href=(href!(auth::sign_in::page)) class="button button-sm">"Sign in"</a>
                    }
                </div>
            </div>
        </header>
    })
}

// The line at the bottom of every page.
#[component]
pub async fn footer(theme: Theme) -> Result<impl View> {
    Ok(view! {
        <footer class="footer">
            <p>"Lemnos. Forge your own cloud."</p>
            // Shown here only on narrow screens; the bar has no room for it there.
            theme_switch(theme: theme)
        </footer>
    })
}

// System, light or dark. Three submit buttons, not a script: the choice is
// stored in a cookie by `set_theme` in app.rs.
#[component]
pub async fn theme_switch(theme: Theme) -> Result<impl View> {
    Ok(view! {
                <form
            method="post"
            action=(href!(crate::app::set_theme))
            class="theme-switch"
            aria-label="Theme"
        >
            <button
                type="submit"
                name="theme"
                value="system"
                aria-pressed=(pressed(theme == Theme::System))
                aria-label="Follow this device"
                title="Follow this device"
            >
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <rect x="1.75" y="2.75" width="12.5" height="8.5" rx="1"></rect>
                    <path d="M5.5 13.75h5"></path>
                </svg>
            </button>
            <button
                type="submit"
                name="theme"
                value="light"
                aria-pressed=(pressed(theme == Theme::Light))
                aria-label="Light"
                title="Light"
            >
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <circle cx="8" cy="8" r="2.75"></circle>
                    <path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.05 1.05M11.55 11.55l1.05 1.05M3.4 12.6l1.05-1.05M11.55 4.45l1.05-1.05"></path>
                </svg>
            </button>
            <button
                type="submit"
                name="theme"
                value="dark"
                aria-pressed=(pressed(theme == Theme::Dark))
                aria-label="Dark"
                title="Dark"
            >
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M13.25 9.6A5.75 5.75 0 0 1 6.4 2.75a5.75 5.75 0 1 0 6.85 6.85Z"></path>
                </svg>
            </button>
        </form>
    })
}

// Something that went wrong: what it means for the reader in `message`, and
// the technical reason folded away below it in `detail`. `link` is where to
// go to look into it, as (address, words), or two empty strings for none.
#[component]
pub async fn problem(message: &str, detail: &str, link: (&str, &str)) -> Result<impl View> {
    Ok(view! {
        <div class="notice notice-danger" role="alert">
            <svg width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M9 2.25 16.25 15H1.75L9 2.25Z"></path>
                <path d="M9 7.25v3.5M9 12.9v.1"></path>
            </svg>
            <div>
                <p>
                    (message)
                    if !link.0.is_empty() {
                        " "
                        <a href=(link.0) class="link">(link.1)</a>
                    }
                </p>
                <details>
                    <summary>"Technical details"</summary>
                    <p class="figure">(detail)</p>
                </details>
            </div>
        </div>
    })
}

// A message about what just happened. `tone` is "danger", "success" or
// "warning"; anything else is neutral.
#[component]
pub async fn notice(tone: &str, message: &str) -> Result<impl View> {
    let class = match tone {
        "danger" => "notice notice-danger",
        "success" => "notice notice-success",
        "warning" => "notice notice-warning",
        _ => "notice",
    };
    // Problems are announced to screen readers right away; the rest can wait.
    let role = if tone == "danger" { "alert" } else { "status" };

    Ok(view! {
        <div class=(class) role=(role)>
            <svg width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                if tone == "success" {
                    <circle cx="9" cy="9" r="7.25"></circle>
                    <path d="m5.75 9.25 2.25 2.25 4.25-4.75"></path>
                } else if tone == "danger" || tone == "warning" {
                    <path d="M9 2.25 16.25 15H1.75L9 2.25Z"></path>
                    <path d="M9 7.25v3.5M9 12.9v.1"></path>
                } else {
                    <circle cx="9" cy="9" r="7.25"></circle>
                    <path d="M9 8.25v4M9 5.6v.1"></path>
                }
            </svg>
            <p>(message)</p>
        </div>
    })
}

// A heat strip: how much of something is in use, as a fraction from 0 to 1,
// with `figure` (e.g. "37%" or "2.1 / 8.0 GiB") beside it. Without a
// fraction the strip stays empty.
#[component]
pub async fn heat(fraction: Option<f64>, figure: String) -> Result<impl View> {
    let percent = fraction.map_or(0.0, |fraction| (fraction * 100.0).clamp(0.0, 100.0));

    Ok(view! {
        <div class="heat">
            <div class="heat-track" aria-hidden="true">
                <span style=(format!("--v: {percent:.0}%"))></span>
            </div>
            <span class="figure">(figure)</span>
        </div>
    })
}
