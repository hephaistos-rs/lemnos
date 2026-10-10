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
        <div class="max-w-md mx-auto mt-12 p-8 rounded-lg bg-secondary-700">
            <h2 class="text-2xl font-bold mb-6 text-center text-secondary-50">"Two-factor code"</h2>
            if let Some(code) = &query.error {
                <p class="mb-4 text-sm text-danger-200">(error_message(code))</p>
            }
            <form method="post" class="flex flex-col gap-y-4">
                <div class="flex flex-col gap-y-1">
                    <label for="code" class="label">"Code from your authenticator app"</label>
                    <div class="input-wrapper">
                        <input type="text" id="code" name="code" inputmode="numeric" autocomplete="one-time-code" autofocus="" required="">
                    </div>
                </div>
                <button type="submit" class="button button-primary w-full mt-2">"Verify"</button>
            </form>
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
