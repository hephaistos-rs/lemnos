// /auth/*: signing in to Lemnos. The real logic lives in `lemnos-auth`; these
// pages only turn requests into calls to it.

use lemnos_auth::User;
use topcoat::{Result, context::Cx};

pub mod sign_in;
pub mod sign_out;
pub mod sign_up;

/// The user signed in on this request, if any.
pub async fn current_user(cx: &Cx) -> Result<Option<User>> {
    let Some(_hash) = topcoat::session::token_hash(cx).await? else {
        return Ok(None);
    };
    // TODO: look up `_hash` in session storage (via `lemnos-auth`) and return
    // its user while the session is unexpired. Until then nobody is signed in.
    Ok(None)
}
