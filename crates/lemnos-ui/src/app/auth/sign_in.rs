// GET /auth/sign-in shows the form; POST /auth/sign-in checks it.

use lemnos_auth::{AuthError, Login};
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

use super::{error_code, error_message, start_session, stash, state, two_factor};

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
    notice: Option<String>,
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let query = query_params::<Query>(cx)?;
    let state = state(cx);
    let providers: Vec<_> = state.sso.providers().collect();

    Ok(view! {
        <div class="max-w-md mx-auto mt-12 p-8 rounded-lg bg-secondary-700">
            <h2 class="text-2xl font-bold mb-6 text-center text-secondary-50">"Sign in"</h2>
            if let Some(code) = &query.error {
                <p class="mb-4 text-sm text-danger-200">(error_message(code))</p>
            }
            if query.notice.as_deref() == Some("created") {
                <p class="mb-4 text-sm text-success-200">"Account created. You can sign in now."</p>
            }
            // POST keeps the password out of the URL.
            <form method="post" class="flex flex-col gap-y-4">
                <div class="flex flex-col gap-y-1">
                    <label for="username" class="label">"Username"</label>
                    <div class="input-wrapper">
                        <input type="text" id="username" name="username" autocomplete="username webauthn" required="">
                    </div>
                </div>
                <div class="flex flex-col gap-y-1">
                    <label for="password" class="label">"Password"</label>
                    <div class="input-wrapper">
                        <input type="password" id="password" name="password" autocomplete="current-password" required="">
                    </div>
                </div>
                <button type="submit" class="button button-primary w-full mt-2">"Sign in"</button>
                if state.ldap {
                    <p class="text-xs text-secondary-300">"Directory (LDAP) accounts work here too."</p>
                }
            </form>
            if state.passkeys {
                <div class="mt-6 pt-6 border-t border-secondary-500 flex flex-col gap-y-2">
                    // Uses the username typed above; see passkey.js.
                    <button type="button" class="button w-full" data-passkey-sign-in="">"Sign in with a passkey"</button>
                    <p class="text-sm text-danger-200" data-passkey-error=""></p>
                </div>
                <script src="/auth/passkey/passkey.js" defer=""></script>
            }
            if !providers.is_empty() {
                <div class="mt-6 pt-6 border-t border-secondary-500 flex flex-col gap-y-2">
                    for provider in &providers {
                        <a href=(format!("/auth/sso/{}/start", provider.id)) class="button w-full text-center">
                            "Continue with " (provider.display_name)
                        </a>
                    }
                </div>
            }
        </div>
    })
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

#[route(POST)]
pub async fn submit(cx: &Cx, Form(input): Form<Credentials>) -> Result<SeeOther> {
    let state = state(cx);
    let mut result = state
        .auth
        .login_password(&input.username, &input.password)
        .await;
    // Local accounts are tried first. A directory user has no local
    // password, so that fails and the directory gets its turn.
    if state.ldap && matches!(result, Err(AuthError::InvalidCredentials)) {
        result = state
            .auth
            .login_ldap(
                &input.username,
                &input.password,
                state.unknown_identity("ldap"),
            )
            .await;
    }

    Ok(see_other(match result {
        Ok(Login::Complete(authenticated)) => {
            start_session(cx, authenticated).await?;
            href!(crate::app::home).resolve(cx).to_string()
        }
        Ok(Login::SecondFactor(challenge)) => {
            stash(cx, two_factor::CHALLENGE, SameSite::Lax, challenge)?;
            href!(two_factor::page).resolve(cx).to_string()
        }
        Err(error) => failed(cx, error_code(&error)),
    }))
}

/// The sign-in page's URL with an error message on it.
pub fn failed(cx: &Cx, code: &str) -> String {
    format!("{}?error={code}", href!(page).resolve(cx))
}
