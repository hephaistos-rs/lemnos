// POST /auth/sign-out (a POST, not a link, because it changes state)

use topcoat::{
    Result,
    context::Cx,
    router::{
        error::{SeeOther, see_other},
        href, route,
    },
    session,
};

#[route(POST)]
pub async fn sign_out(cx: &Cx) -> Result<SeeOther> {
    if let Some(_hash) = session::stop(cx).await? {
        // TODO: delete the session record for `_hash` (via `lemnos-auth`).
    }
    Ok(see_other(href!(crate::app::home).resolve(cx)))
}
