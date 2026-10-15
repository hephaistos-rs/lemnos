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
use crate::components::notice;

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
        <div class="page-mid">
            <div class="page-head">
                <div>
                    <h1>"Account"</h1>
                    <p>"Signed in as " (user.username.as_str())</p>
                </div>
            </div>

            if let Some(code) = &query.error {
                notice(tone: "danger", message: error_message(code))
            }

            <section class="panel">
                <div class="flex items-center justify-between gap-x-4">
                    <h2 class="section-title">"Two-factor codes"</h2>
                    if totp_enabled {
                        <span class="state state-running">"On"</span>
                    } else {
                        <span class="state state-stopped">"Off"</span>
                    }
                </div>
                if totp_enabled {
                    <p class="text-ink-muted max-w-[65ch]">
                        "Signing in with your password also asks for a code."
                    </p>
                    <form method="post" action=(href!(totp_disable)) class="flex flex-col gap-y-4">
                        <div class="field">
                            <label for="password" class="label">"Password, to turn it off"</label>
                            <input type="password" id="password" name="password" class="input" autocomplete="current-password" required="">
                        </div>
                        <button type="submit" class="button button-danger self-start">"Turn off"</button>
                    </form>
                } else {
                    <p class="text-ink-muted max-w-[65ch]">
                        "Turn it on to require a code from an authenticator app after your password."
                    </p>
                    <form method="post" action=(href!(totp_begin)) class="flex flex-col">
                        <button type="submit" class="button button-primary self-start">"Set up"</button>
                    </form>
                }
            </section>

            if state.passkeys {
                <section class="panel">
                    <h2 class="section-title">"Passkeys"</h2>
                    if passkeys.is_empty() {
                        <p class="text-ink-muted max-w-[65ch]">
                            "None yet. A passkey lets you sign in with your fingerprint, face or a security key instead of a password."
                        </p>
                    } else {
                        <div class="flex flex-col">
                            for passkey in &passkeys {
                                <form method="post" action=(href!(passkey_remove)) class="flex items-center justify-between gap-x-4 py-2 border-b border-line">
                                    <span class="font-medium break-all">(passkey.label.as_str())</span>
                                    <input type="hidden" name="credential_id" value=(to_hex(&passkey.credential_id))>
                                    <button type="submit" class="button button-sm button-danger">"Remove"</button>
                                </form>
                            }
                        </div>
                    }
                    <div class="flex flex-col gap-y-4">
                        <div class="field">
                            <label for="passkey-label" class="label">"Name for a new passkey"</label>
                            <input type="text" id="passkey-label" class="input" placeholder="e.g. Laptop">
                            <p class="text-meta text-danger-text empty:hidden" data-passkey-error=""></p>
                        </div>
                        <button type="button" class="button button-primary self-start" data-passkey-register="">"Add a passkey"</button>
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

    // The QR code holds the same `otpauth://` link as the button below, so
    // scanning it with a phone sets up the app in one go.
    let qr_code = crate::qr::data_url(&enrollment.otpauth_url);

    Ok(view! {
        <div class="page-narrow">
            <div class="flex flex-col gap-y-2">
                <a href=(href!(page)) class="back-link self-start">"Account"</a>
                <div class="page-head">
                    <div>
                        <h1>"Set up two-factor codes"</h1>
                        <p>"Scan this code with an authenticator app on your phone, then enter the 6-digit code the app shows. Nothing changes until you do."</p>
                    </div>
                </div>
            </div>

            <section class="panel">
                if let Some(qr_code) = &qr_code {
                    <img src=(qr_code.as_str()) alt="QR code for your authenticator app" width="220" height="220" class="self-center">
                }
                <div class="flex flex-col gap-y-1">
                    <p class="text-meta text-ink-muted">"Can't scan it? Type this key into the app instead:"</p>
                    <p class="figure text-base break-all">(enrollment.secret_base32.as_str())</p>
                </div>
                <div class="flex flex-col gap-y-1.5">
                    <p class="text-meta text-ink-muted">"Authenticator app on this same device?"</p>
                    <a href=(enrollment.otpauth_url.as_str()) class="button self-start">"Open it here"</a>
                </div>
                <form method="post" action=(href!(totp_confirm)) class="flex flex-col gap-y-4 pt-4 border-t border-line">
                    <div class="field">
                        <label for="code" class="label">"Code from the app"</label>
                        <input type="text" id="code" name="code" class="input" inputmode="numeric" autocomplete="one-time-code" autofocus="" required="">
                    </div>
                    <button type="submit" class="button button-primary w-full">"Turn on"</button>
                </form>
            </section>
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
