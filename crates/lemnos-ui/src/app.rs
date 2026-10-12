// Root of the route tree: this module maps to `/`, and every module below it
// adds one URL segment (e.g. `app/vms.rs` -> `/vms`). The two modules right
// below are the exception: `site` and `console` are groups, which add
// nothing to the URL and only decide what surrounds the page.
//
//   site      what anyone can read: the landing page, features, signing in
//   console   the application itself, for signed-in users only

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
        header, href, layout, module_router,
        request::headers,
        route,
    },
    session::{RouterBuilderSessionExt, SessionConfig},
    tailwind,
    view::{View, view},
};

use crate::components::Theme;

pub mod console;
pub mod site;

// Signing in is public, so it lives in `site`; most code just says `auth`.
pub use site::auth;

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

// The document around every page. What goes inside the body (which bar,
// which footer) is up to the layouts in `site` and `console`.
#[layout]
async fn document(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let theme = Theme::current(cx);

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
                (slot)
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

// Where these pages lived before the console got its own addresses.

#[route(GET "./proxmox")]
pub async fn old_environments(cx: &Cx) -> Result<SeeOther> {
    Ok(see_other(href!(console::environments::page).resolve(cx)))
}

#[route(GET "./account")]
pub async fn old_account(cx: &Cx) -> Result<SeeOther> {
    Ok(see_other(
        href!(console::settings::security::page).resolve(cx),
    ))
}
