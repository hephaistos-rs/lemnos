// POST /auth/sign-out (a POST, not a link, because it changes state)

use lemnos_auth::SessionId;
use topcoat::{
    Result,
    context::Cx,
    router::{
        error::{SeeOther, see_other},
        href, route,
    },
    session,
};

use super::state;

#[route(POST)]
pub async fn sign_out(cx: &Cx) -> Result<SeeOther> {
    // Clearing the cookie is not enough: the record is deleted too, so a
    // copy of the token stops working.
    if let Some(hash) = session::stop(cx).await? {
        state(cx).auth.end_session(&SessionId(*hash)).await?;
    }
    Ok(see_other(href!(crate::app::home).resolve(cx)))
}
