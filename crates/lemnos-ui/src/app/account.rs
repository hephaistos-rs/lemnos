// /account: the signed-in user's own security settings (two-factor codes
// and passkeys).

use lemnos_auth::{AuthError, User};
use serde::Deserialize;
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{RouterErrorExt, SeeOther, redirect, see_other},
        href, page, query_params, route,
    },
    view::{View, view},
};

use crate::app::auth::{current_user, error_code, error_message, sign_in, state};

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
}

/// The signed-in user; visitors are sent to the sign-in page instead.
async fn require_user(cx: &Cx) -> Result<User> {
    Ok(current_user(cx)
        .await?
        .ok_or_redirect(href!(sign_in::page).resolve(cx))?)
}

fn back(cx: &Cx, error: Option<&str>) -> String {
    let account = href!(page).resolve(cx);
    match error {
        Some(code) => format!("{account}?error={code}"),
        None => account.to_string(),
    }
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let user = require_user(cx).await?;
    let query = query_params::<Query>(cx)?;
    let state = state(cx);
    let totp_enabled = state.auth.totp_enabled(user.id).await?;
    let passkeys = if state.passkeys {
        state.auth.passkeys(user.id).await?
    } else {
        Vec::new()
    };

    Ok(view! {
        <div class="max-w-xl mx-auto mt-12 flex flex-col gap-y-8">
            <div>
                <h2 class="text-2xl font-bold text-secondary-50">"Account"</h2>
                <p class="text-secondary-300">"Signed in as " (user.username.as_str())</p>
                if let Some(code) = &query.error {
                    <p class="mt-2 text-sm text-danger-200">(error_message(code))</p>
                }
            </div>

            <section class="p-6 rounded-lg bg-secondary-700 flex flex-col gap-y-4">
                <h3 class="text-lg font-bold text-secondary-50">"Two-factor codes"</h3>
                if totp_enabled {
                    <p class="text-success-200">"On: signing in with your password also asks for a code."</p>
                    <form method="post" action=(href!(totp_disable)) class="flex flex-col gap-y-2">
                        <label for="password" class="label">"Password, to turn it off"</label>
                        <div class="input-wrapper">
                            <input type="password" id="password" name="password" autocomplete="current-password" required="">
                        </div>
                        <button type="submit" class="button">"Turn off"</button>
                    </form>
                } else {
                    <p class="text-secondary-200">"Off. Turn it on to require a code from an authenticator app after your password."</p>
                    <form method="post" action=(href!(totp_begin))>
                        <button type="submit" class="button button-primary">"Set up"</button>
                    </form>
                }
            </section>

            if state.passkeys {
                <section class="p-6 rounded-lg bg-secondary-700 flex flex-col gap-y-4">
                    <h3 class="text-lg font-bold text-secondary-50">"Passkeys"</h3>
                    if passkeys.is_empty() {
                        <p class="text-secondary-200">"None yet. A passkey lets you sign in with your fingerprint, face or a security key instead of a password."</p>
                    }
                    for passkey in &passkeys {
                        <form method="post" action=(href!(passkey_remove)) class="flex items-center justify-between gap-x-4">
                            <span class="text-secondary-50">(passkey.label.as_str())</span>
                            <input type="hidden" name="credential_id" value=(to_hex(&passkey.credential_id))>
                            <button type="submit" class="button">"Remove"</button>
                        </form>
                    }
                    <div class="flex flex-col gap-y-2">
                        <label for="passkey-label" class="label">"Name for a new passkey"</label>
                        <div class="input-wrapper">
                            <input type="text" id="passkey-label" placeholder="e.g. Laptop">
                        </div>
                        <button type="button" class="button button-primary" data-passkey-register="">"Add a passkey"</button>
                        <p class="text-sm text-danger-200" data-passkey-error=""></p>
                    </div>
                    <script src="/auth/passkey/passkey.js" defer=""></script>
                </section>
            }
        </div>
    })
}

// POST /account/totp/begin: makes a new secret and shows it once.
#[page(POST "./totp/begin")]
pub async fn totp_begin(cx: &Cx) -> Result<impl View> {
    let user = require_user(cx).await?;
    let enrollment = match state(cx).auth.begin_totp_enrollment(&user).await {
        Ok(enrollment) => enrollment,
        Err(AuthError::TotpAlreadyEnabled) => return Err(redirect(back(cx, None)).into()),
        Err(error) => return Err(error.into()),
    };

    Ok(view! {
        <div class="max-w-xl mx-auto mt-12 p-6 rounded-lg bg-secondary-700 flex flex-col gap-y-4">
            <h2 class="text-2xl font-bold text-secondary-50">"Set up two-factor codes"</h2>
            <p class="text-secondary-200">"Add this key to your authenticator app, then enter the code it shows. Nothing changes until you do."</p>
            <p class="font-mono text-lg break-all text-secondary-50">(enrollment.secret_base32.as_str())</p>
            <a href=(enrollment.otpauth_url.as_str()) class="nav-link">"Open in an authenticator app on this device"</a>
            <form method="post" action=(href!(totp_confirm)) class="flex flex-col gap-y-2">
                <label for="code" class="label">"Code"</label>
                <div class="input-wrapper">
                    <input type="text" id="code" name="code" inputmode="numeric" autocomplete="one-time-code" autofocus="" required="">
                </div>
                <button type="submit" class="button button-primary">"Turn on"</button>
            </form>
        </div>
    })
}

#[derive(Deserialize)]
pub struct CodeForm {
    code: String,
}

#[route(POST "./totp/confirm")]
pub async fn totp_confirm(cx: &Cx, Form(input): Form<CodeForm>) -> Result<SeeOther> {
    let user = require_user(cx).await?;
    Ok(see_other(
        match state(cx).auth.confirm_totp(user.id, &input.code).await {
            Ok(()) => back(cx, None),
            Err(error) => back(cx, Some(error_code(&error))),
        },
    ))
}

#[derive(Deserialize)]
pub struct PasswordForm {
    password: String,
}

// Asks for the password again, so someone who finds an unlocked session
// can't quietly switch the second factor off.
#[route(POST "./totp/disable")]
pub async fn totp_disable(cx: &Cx, Form(input): Form<PasswordForm>) -> Result<SeeOther> {
    let user = require_user(cx).await?;
    let auth = &state(cx).auth;
    Ok(see_other(
        match auth
            .login_password(user.username.as_str(), &input.password)
            .await
        {
            Ok(_) => {
                auth.disable_totp(user.id).await?;
                back(cx, None)
            }
            Err(error) => back(cx, Some(error_code(&error))),
        },
    ))
}

#[derive(Deserialize)]
pub struct PasskeyForm {
    credential_id: String,
}

#[route(POST "./passkeys/remove")]
pub async fn passkey_remove(cx: &Cx, Form(input): Form<PasskeyForm>) -> Result<SeeOther> {
    let user = require_user(cx).await?;
    if let Some(credential_id) = from_hex(&input.credential_id) {
        state(cx)
            .auth
            .remove_passkey(user.id, &credential_id)
            .await?;
    }
    Ok(see_other(back(cx, None)))
}

// Passkey IDs are raw bytes; hex makes them fit in a form field.

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|start| u8::from_str_radix(hex.get(start..start + 2)?, 16).ok())
        .collect()
}
