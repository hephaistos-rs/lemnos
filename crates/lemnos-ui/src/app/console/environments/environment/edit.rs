// /environments/{environment}/edit: changing how Lemnos reaches and signs in
// to an environment. Admins only.
//
//   GET  /environments/{environment}/edit   the form, filled in except for the secret
//   POST /environments/{environment}/edit   saves it, then shows the environment
//
// One declared in the config file is shown, not edited: sync on start would
// put the file's version back.

use lemnos_db::{ProxmoxEnvironmentEdit, ProxmoxLogin, ProxmoxStoreError};
use serde::{Deserialize, Serialize};
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

use super::{Environment, stored};
use crate::{
    access::admin_access,
    app::console::environments::{
        Fields, database, environment_fields, error_message, login_from, managed_by_config,
        token_error_code, valid_url,
    },
    components::notice,
};

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    admin_access(cx).await?;
    let query = query_params::<Query>(cx)?;
    let environment = stored(cx).await?;
    let id = environment.id.0;
    let managed = managed_by_config(cx, &environment);
    let fields = Fields::of(&environment);

    Ok(view! {
        <div class="page">
            <div class="flex flex-col gap-y-2">
                <a href=(href!(super::page, Environment(id))) class="back-link self-start">
                    (environment.name.as_str())
                </a>
                <div class="page-head">
                    <div>
                        <h1>"Edit environment"</h1>
                        <p>"Where Lemnos finds it, and how it signs in."</p>
                    </div>
                </div>
            </div>

            <section class="panel max-w-xl">
                if managed {
                    notice(tone: "", message: error_message("managed"))
                } else {
                    if let Some(code) = &query.error {
                        notice(tone: "danger", message: error_message(code))
                    }
                    <form method="post" class="flex flex-col gap-y-4">
                        environment_fields(fields: &fields)
                        <button type="submit" class="button button-primary self-start">
                            "Save"
                        </button>
                    </form>
                }
            </section>
        </div>
    })
}

#[derive(Deserialize)]
pub struct EditForm {
    name: String,
    url: String,
    user: String,
    realm: String,
    token_id: String,
    // Empty keeps the stored one.
    secret: String,
    accept_invalid_certs: Option<String>,
}

#[derive(Serialize)]
struct Problem<'a> {
    error: &'a str,
}

#[derive(Serialize)]
struct Saved<'a> {
    notice: &'a str,
}

#[route(POST)]
pub async fn save(cx: &Cx, Form(input): Form<EditForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    let environment = stored(cx).await?;
    let id = environment.id.0;
    let again = |code: &str| {
        see_other(
            href!(page, Environment(id))
                .query(Problem { error: code })
                .resolve(cx),
        )
    };
    if managed_by_config(cx, &environment) {
        return Ok(again("managed"));
    }

    let name = input.name.trim();
    let url = input.url.trim();
    if [name, url].contains(&"") {
        return Ok(again("fields"));
    }
    if !valid_url(url) {
        return Ok(again("url"));
    }
    // Parsed with a stand-in, so the same rules apply as when adding; only
    // what it says about user, realm and token is kept.
    let (user, realm, login) =
        match login_from(&input.token_id, &input.user, &input.realm, String::new()) {
            Ok(login) => login,
            Err(error) => return Ok(again(token_error_code(&error))),
        };
    let token_id = match login {
        ProxmoxLogin::ApiToken { id, .. } => Some(id),
        ProxmoxLogin::Password(_) => None,
    };
    let edit = ProxmoxEnvironmentEdit {
        name: name.to_owned(),
        url: url.to_owned(),
        user,
        realm,
        token_id,
        secret: (!input.secret.is_empty()).then_some(input.secret),
        accept_invalid_certs: input.accept_invalid_certs.is_some(),
    };
    match database(cx)
        .edit_proxmox_environment(environment.id, edit)
        .await
    {
        Ok(_) => Ok(see_other(
            href!(super::page, Environment(id))
                .query(Saved { notice: "saved" })
                .resolve(cx),
        )),
        Err(ProxmoxStoreError::NameTaken) => Ok(again("name")),
        Err(ProxmoxStoreError::SecretNeeded) => Ok(again("secret")),
        Err(error) => Err(error.into()),
    }
}
