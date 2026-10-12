// /settings: what a signed-in user can see and change about their own
// account. `account.rs` is who they are here, `security.rs` how they sign in.

use topcoat::{
    Result,
    context::Cx,
    router::{
        Slot,
        error::{SeeOther, see_other},
        href, layout,
        request::uri,
        route,
    },
    view::{View, view},
};

pub mod account;
pub mod security;

// The two settings pages share a row of tabs above them.
#[layout]
async fn settings_tabs(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let path = uri(cx).path();
    let current = |prefix: &str| {
        if path.starts_with(prefix) {
            "page"
        } else {
            "false"
        }
    };

    Ok(view! {
        <nav class="tabs" aria-label="Settings">
            <a href=(href!(account::page)) aria-current=(current("/settings/account"))>"Account"</a>
            <a href=(href!(security::page)) aria-current=(current("/settings/security"))>"Security"</a>
        </nav>
        (slot)
    })
}

// /settings itself has nothing to show.
#[route(GET)]
pub async fn index(cx: &Cx) -> Result<SeeOther> {
    Ok(see_other(href!(account::page).resolve(cx)))
}
