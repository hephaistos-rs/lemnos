// /auth/passkey/*: the JSON endpoints `passkey.js` calls. Each ceremony
// (registering a passkey, signing in with one) is a begin/finish pair with
// the browser's passkey prompt in between.

use lemnos_auth::{
    AuthError,
    passkey::{
        CreationChallengeResponse, PasskeyError, PasskeyLoginState, PasskeyRegistrationState,
        PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use topcoat::{
    Result,
    context::Cx,
    cookie::SameSite,
    router::{
        StatusCode,
        content::{Js, Json},
        href, route,
    },
};

use super::{current_user, start_session, stash, state, take};

const REGISTRATION: &str = "passkey-registration";
const SIGN_IN: &str = "passkey-sign-in";

/// What every endpoint here answers with: a status, so the script can tell
/// success from failure, and JSON.
type Reply = Result<(StatusCode, Json<Value>)>;

fn ok(body: impl serde::Serialize) -> Reply {
    Ok((StatusCode::OK, Json(serde_json::to_value(body)?)))
}

fn refuse(status: StatusCode, message: &str) -> Reply {
    Ok((status, Json(json!({ "error": message }))))
}

fn refuse_error(error: &AuthError) -> Reply {
    match error {
        AuthError::Expired => refuse(StatusCode::BAD_REQUEST, "That took too long. Try again."),
        AuthError::Passkey(PasskeyError::NoPasskeys) => refuse(
            StatusCode::BAD_REQUEST,
            "No passkey is registered for that username.",
        ),
        other => {
            eprintln!("passkey request failed: {other:?}");
            refuse(
                StatusCode::BAD_REQUEST,
                "The passkey could not be verified.",
            )
        }
    }
}

#[route(GET "./passkey.js")]
pub async fn script() -> Result<Js<&'static str>> {
    Ok(Js(include_str!("passkey.js")))
}

#[route(POST "./register/begin")]
pub async fn register_begin(cx: &Cx) -> Reply {
    let Some(user) = current_user(cx).await? else {
        return refuse(StatusCode::UNAUTHORIZED, "Sign in first.");
    };
    match state(cx).auth.begin_passkey_registration(&user).await {
        Ok((challenge, registration)) => {
            let challenge: CreationChallengeResponse = challenge;
            stash(cx, REGISTRATION, SameSite::Strict, registration)?;
            ok(challenge)
        }
        Err(error) => refuse_error(&error),
    }
}

#[derive(Deserialize)]
pub struct Registration {
    label: String,
    credential: RegisterPublicKeyCredential,
}

#[route(POST "./register/finish")]
pub async fn register_finish(cx: &Cx, Json(input): Json<Registration>) -> Reply {
    let Some(user) = current_user(cx).await? else {
        return refuse(StatusCode::UNAUTHORIZED, "Sign in first.");
    };
    let Some(registration) = take::<PasskeyRegistrationState>(cx, REGISTRATION, SameSite::Strict)
    else {
        return refuse(StatusCode::BAD_REQUEST, "That took too long. Try again.");
    };
    // The ceremony must have been started by the user who is finishing it.
    if registration.user() != user.id {
        return refuse(StatusCode::FORBIDDEN, "Sign in first.");
    }
    match state(cx)
        .auth
        .finish_passkey_registration(registration, &input.credential, &input.label)
        .await
    {
        Ok(_) => ok(json!({ "ok": true })),
        Err(error) => refuse_error(&error),
    }
}

#[derive(Deserialize)]
pub struct SignIn {
    username: String,
}

#[route(POST "./sign-in/begin")]
pub async fn sign_in_begin(cx: &Cx, Json(input): Json<SignIn>) -> Reply {
    match state(cx).auth.begin_passkey_login(&input.username).await {
        Ok((challenge, sign_in)) => {
            let challenge: RequestChallengeResponse = challenge;
            stash(cx, SIGN_IN, SameSite::Strict, sign_in)?;
            ok(challenge)
        }
        Err(error) => refuse_error(&error),
    }
}

#[route(POST "./sign-in/finish")]
pub async fn sign_in_finish(cx: &Cx, Json(credential): Json<PublicKeyCredential>) -> Reply {
    let Some(sign_in) = take::<PasskeyLoginState>(cx, SIGN_IN, SameSite::Strict) else {
        return refuse(StatusCode::BAD_REQUEST, "That took too long. Try again.");
    };
    match state(cx)
        .auth
        .finish_passkey_login(sign_in, &credential)
        .await
    {
        Ok(authenticated) => {
            start_session(cx, authenticated).await?;
            ok(json!({ "redirect": href!(crate::app::home).resolve(cx).to_string() }))
        }
        Err(error) => refuse_error(&error),
    }
}
