// GET /auth/sign-up shows the form; POST /auth/sign-up creates a local
// account. Only when `allow_sign_up` is set in the config.

use lemnos_auth::{NewUser, Username};
use serde::Deserialize;
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{SeeOther, see_other},
        href, page, query_params, route,
    },
    view::{View, view},
};

use super::{error_code, error_message, sign_in, state};

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let query = query_params::<Query>(cx)?;
    Ok(view! {
        <div class="max-w-md mx-auto mt-12 p-8 rounded-lg bg-secondary-700">
            <h2 class="text-2xl font-bold mb-6 text-center text-secondary-50">"Sign up"</h2>
            if let Some(code) = &query.error {
                <p class="mb-4 text-sm text-danger-200">(error_message(code))</p>
            }
            if state(cx).allow_sign_up {
                <form method="post" class="flex flex-col gap-y-4">
                    <div class="flex flex-col gap-y-1">
                        <label for="username" class="label">"Username"</label>
                        <div class="input-wrapper">
                            <input type="text" id="username" name="username" autocomplete="username" required="">
                        </div>
                    </div>
                    <div class="flex flex-col gap-y-1">
                        <label for="password" class="label">"Password (at least 12 characters)"</label>
                        <div class="input-wrapper">
                            <input type="password" id="password" name="password" autocomplete="new-password" minlength="12" required="">
                        </div>
                    </div>
                    <button type="submit" class="button button-primary w-full mt-2">"Create account"</button>
                </form>
            } else {
                <p class="text-secondary-200">
                    "Creating accounts is turned off. Set allow_sign_up = true in the Lemnos config to enable it."
                </p>
            }
        </div>
    })
}

#[derive(Deserialize)]
pub struct NewAccount {
    username: String,
    password: String,
}

#[route(POST)]
pub async fn submit(cx: &Cx, Form(input): Form<NewAccount>) -> Result<SeeOther> {
    let state = state(cx);
    let failed = |code: &str| format!("{}?error={code}", href!(page).resolve(cx));
    if !state.allow_sign_up {
        return Ok(see_other(failed("closed")));
    }
    let Ok(username) = Username::parse(&input.username) else {
        return Ok(see_other(failed("username")));
    };
    // Checked before the account exists, so a weak password doesn't leave
    // behind an account nobody can sign in to.
    if state.auth.password_policy().check(&input.password).is_err() {
        return Ok(see_other(failed("weak")));
    }
    let user = match state.auth.create_user(NewUser::new(username)).await {
        Ok(user) => user,
        Err(error) => return Ok(see_other(failed(error_code(&error)))),
    };
    state.auth.set_password(user.id, &input.password).await?;
    Ok(see_other(format!(
        "{}?notice=created",
        href!(sign_in::page).resolve(cx)
    )))
}
