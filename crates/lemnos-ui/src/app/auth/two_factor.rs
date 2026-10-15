// GET/POST /auth/two-factor: the code from an authenticator app, asked for
// after a correct password when the account has 2FA on.

use lemnos_auth::SecondFactorChallenge;
use serde::Deserialize;
use topcoat::{
    Result,
    context::Cx,
    cookie::SameSite,
    router::{
        content::Form,
        error::{SeeOther, see_other},
        href, page, query_params, route,
    },
    view::{View, view},
};

use super::{error_code, error_message, peek, sign_in, start_session, state, take};
use crate::components::notice;

/// Cookie holding the proof that the password step was passed.
pub const CHALLENGE: &str = "second-factor";

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let query = query_params::<Query>(cx)?;
    Ok(view! {
        <div class="page-narrow">
            <div class="flex flex-col gap-y-1">
                <h1 class="text-title font-semibold">"Two-factor code"</h1>
                <p class="text-ink-muted">"Your password was right. One more step to sign in."</p>
            </div>

            if let Some(code) = &query.error {
                notice(tone: "danger", message: error_message(code))
            }

            <form method="post" class="panel">
                <div class="field">
                    <label for="code" class="label">"Code from your authenticator app"</label>
                    <input type="text" id="code" name="code" class="input" inputmode="numeric" autocomplete="one-time-code" autofocus="" required="">
                </div>
                <button type="submit" class="button button-primary w-full">"Verify"</button>
            </form>

            <p class="text-ink-muted">
                <a href=(href!(sign_in::page)) class="link">"Back to sign in"</a>
            </p>
        </div>
    })
}

#[derive(Deserialize)]
pub struct CodeForm {
    code: String,
}

#[route(POST)]
pub async fn submit(cx: &Cx, Form(input): Form<CodeForm>) -> Result<SeeOther> {
    // Kept until the code is right, so a typo can be retried.
    let Some(challenge) = peek::<SecondFactorChallenge>(cx, CHALLENGE, SameSite::Lax) else {
        return Ok(see_other(sign_in::failed(cx, "expired")));
    };
    Ok(see_other(
        match state(cx).auth.verify_totp(&challenge, &input.code).await {
            Ok(authenticated) => {
                take::<SecondFactorChallenge>(cx, CHALLENGE, SameSite::Lax);
                start_session(cx, authenticated).await?;
                href!(crate::app::home).resolve(cx).to_string()
            }
            Err(lemnos_auth::AuthError::Expired) => sign_in::failed(cx, "expired"),
            Err(error) => format!("{}?error={}", href!(page).resolve(cx), error_code(&error)),
        },
    ))
}
