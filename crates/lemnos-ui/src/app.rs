// Root of the route tree: this module maps to `/`, and every module below it
// adds one URL segment (e.g. `app/vms.rs` -> `/vms`).

use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    cookie::RouterBuilderCookieExt,
    font::{self, Font, fontsource::fontsource_font},
    router::{Router, RouterBuilderDiscoverExt, Slot, layout, module_router, page},
    session::{RouterBuilderSessionExt, SessionConfig},
    tailwind,
    view::{View, view},
};

use crate::components::{hello, nav_bar};

pub mod auth;

// Self-hosted (bundled as assets) instead of loaded from Google Fonts.
// Only these weights/styles are bundled; add more here if the CSS needs them.
const DM_SANS: Font = fontsource_font!(
    DM_SANS,
    weight: [400, 500, 700],
    style: [Normal, Italic],
    host: Asset,
);

pub fn router() -> Router {
    module_router!()
        .discover()
        // Serves bundled files (like the Tailwind stylesheet) under /_topcoat/assets.
        .assets(AssetBundle::load().unwrap())
        // Session tokens travel in a cookie, so sessions need cookie support.
        .cookies()
        .sessions(SessionConfig::default())
        .build()
}

// Wraps every page in this module and below it with the header, footer, and main content area.
#[layout]
async fn app_shell(slot: Slot<'_>) -> Result<impl View> {
    Ok(view! {
        <!DOCTYPE html>
        <html>
            <head>
                <title>"Lemnos"</title>
                <meta charset="UTF-8">
                <meta name="viewport" content="width=device-width, initial-scale=1.0">
                topcoat::dev::script()
                font::link(font: DM_SANS)
                <link rel="stylesheet" href=(tailwind::stylesheet!())>
            </head>
            <body class="min-h-screen flex flex-col">
                <header class="py-4 bg-secondary-900 border-b-4 border-primary-300">
                    <div class="container flex items-center justify-between">
                        <h1 class="text-xl font-bold text-secondary-50">"Lemnos"</h1>
                        nav_bar()
                    </div>
                </header>
                <main class="container flex-1 py-6">
                    (slot)
                </main>
                <footer class="py-4 text-sm text-secondary-400 border-t border-secondary-600">
                    <div class="container">
                        <p>"Forge your own cloud."</p>
                    </div>
                    <nav>
                        <ul class="flex space-x-4">

                        </ul>
                    </nav>
                </footer>
            </body>
        </html>
    })
}

// `pub` so other modules (like the nav bar) can link to it with `href!`.
#[page]
pub async fn home() -> Result<impl View> {
    Ok(view! {
        hello(name: "World")
    })
}

