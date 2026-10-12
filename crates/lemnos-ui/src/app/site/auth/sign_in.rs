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

use super::{
    after_sign_in, error_code, error_message, forget_return, is_local_path, remember_return,
    start_session, stash, state, two_factor,
};
use crate::{access::join_through, components::notice};

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
    notice: Option<String>,
    // The page to show once signed in; set by the console when it sends a
    // visitor here.
    next: Option<String>,
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let query = query_params::<Query>(cx)?;
    match &query.next {
        Some(next) if is_local_path(next) => remember_return(cx, next)?,
        // Coming here on their own, a visitor is not on the way anywhere.
        // A failed attempt comes back with a message and keeps its place.
        None if query.error.is_none() && query.notice.is_none() => forget_return(cx),
        _ => {}
    }
    let state = state(cx);
    let providers: Vec<_> = state.sso.providers().collect();

    Ok(view! {
        <div class="page-narrow">
            <div class="flex flex-col gap-y-1">
                <h1 class="text-title font-semibold">"Sign in"</h1>
                <p class="text-lead text-ink-muted">"Forge your own cloud."</p>
            </div>

            if let Some(code) = &query.error {
                notice(tone: "danger", message: error_message(code))
            }
            if query.notice.as_deref() == Some("created") {
                notice(tone: "success", message: "Account created. You can sign in now.")
            }
            if query.notice.as_deref() == Some("password") {
                notice(tone: "success", message: "Password changed. Sign in again with the new one.")
            }

            <div class="panel">
                // POST keeps the password out of the URL.
                <form method="post" class="flex flex-col gap-y-4">
                    <div class="field">
                        <label for="username" class="label">"Username"</label>
                        <input type="text" id="username" name="username" class="input" autocomplete="username webauthn" required="">
                        if state.ldap {
                            <p class="hint">"Directory (LDAP) accounts work here too."</p>
                        }
                    </div>
                    <div class="field">
                        <label for="password" class="label">"Password"</label>
                        <input type="password" id="password" name="password" class="input" autocomplete="current-password" required="">
                    </div>
                    <button type="submit" class="button button-primary w-full">"Sign in"</button>
                </form>
                if state.passkeys {
                    <div class="or">"or"</div>
                    <div class="flex flex-col gap-y-2">
                        // Uses the username typed above; see passkey.js.
                        <button type="button" class="button w-full" data-passkey-sign-in="">"Sign in with a passkey"</button>
                        <p class="text-meta text-danger-text" data-passkey-error=""></p>
                    </div>
                    <script src="/auth/passkey/passkey.js" defer=""></script>
                }
                if !providers.is_empty() {
                    <div class="or">"or"</div>
                    <div class="flex flex-col gap-y-2">
                        for provider in &providers {
                            <a href=(format!("/auth/sso/{}/start", provider.id)) class="button w-full">
                                "Continue with " (provider.display_name)
                            </a>
                        }
                    </div>
                }
            </div>

            if state.allow_sign_up {
                <p class="text-ink-muted">
                    "New here? "
                    <a href=(href!(super::sign_up::page)) class="link">"Create an account"</a>
                </p>
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
    let mut through_ldap = false;
    if state.ldap && matches!(result, Err(AuthError::InvalidCredentials)) {
        through_ldap = true;
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
            let user = start_session(cx, authenticated).await?;
            if through_ldap {
                join_through(cx, &user, "ldap").await?;
            }
            after_sign_in(cx)
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
