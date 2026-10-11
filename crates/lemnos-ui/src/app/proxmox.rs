// /proxmox: the Proxmox VE environments Lemnos manages. This page lists
// them and adds or removes one; `environment.rs` shows what runs on one.
// Talking to Proxmox itself is `lemnos-proxmox`'s job.

use std::error::Error;

use lemnos_auth::User;
use lemnos_db::{
    Database, NewProxmoxEnvironment, ProxmoxEnvironment, ProxmoxEnvironmentId, ProxmoxLogin,
    ProxmoxStoreError,
};
use lemnos_proxmox::client::{Client, Credentials, Endpoint};
use serde::Deserialize;
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{RouterErrorExt, SeeOther, see_other},
        href, page, query_params, route,
    },
    view::{View, view},
};

use crate::{
    app::auth::{current_user, sign_in, state},
    config::ProxmoxConfig,
};

pub mod environment;

pub fn database(cx: &Cx) -> &Database {
    state(cx).auth.store()
}

/// The signed-in user; visitors are sent to the sign-in page instead.
pub async fn require_user(cx: &Cx) -> Result<User> {
    Ok(current_user(cx)
        .await?
        .ok_or_redirect(href!(sign_in::page).resolve(cx))?)
}

/// The stored details, as the two values `lemnos-proxmox` connects with.
pub fn login_details(environment: &ProxmoxEnvironment) -> (Endpoint, Credentials) {
    let endpoint = Endpoint::new(&environment.url, &environment.user, &environment.realm)
        .accept_invalid_certs(environment.accept_invalid_certs);
    let credentials = match &environment.login {
        ProxmoxLogin::ApiToken { id, secret } => Credentials::ApiToken {
            id: id.clone(),
            secret: secret.clone(),
        },
        ProxmoxLogin::Password(password) => Credentials::Password(password.clone()),
    };
    (endpoint, credentials)
}

/// Signs in to the environment. Done anew for every request that needs it.
pub async fn connect(
    environment: &ProxmoxEnvironment,
) -> Result<Client, lemnos_proxmox::client::Error> {
    let (endpoint, credentials) = login_details(environment);
    lemnos_proxmox::client::connect(&endpoint, &credentials).await
}

/// Writes the environments from the config file to the database: new names
/// are added, and known ones are made to match the file again.
pub async fn sync_config(
    database: &Database,
    configured: Vec<ProxmoxConfig>,
) -> Result<(), Box<dyn Error>> {
    let stored = database.proxmox_environments().await?;
    for entry in configured {
        let login = match (entry.token_id, entry.token_secret, entry.password) {
            (Some(id), Some(secret), None) => ProxmoxLogin::ApiToken {
                id,
                secret: secret.expose().to_owned(),
            },
            (None, None, Some(password)) => ProxmoxLogin::Password(password.expose().to_owned()),
            _ => {
                return Err(format!(
                    "proxmox environment `{}`: set either token_id and token_secret, or password",
                    entry.name
                )
                .into());
            }
        };
        match stored.iter().find(|known| known.name == entry.name) {
            Some(known) => {
                database
                    .update_proxmox_environment(ProxmoxEnvironment {
                        id: known.id,
                        name: entry.name,
                        url: entry.url,
                        user: entry.user,
                        realm: entry.realm,
                        login,
                        accept_invalid_certs: entry.accept_invalid_certs,
                    })
                    .await?;
            }
            None => {
                database
                    .create_proxmox_environment(NewProxmoxEnvironment {
                        name: entry.name,
                        url: entry.url,
                        user: entry.user,
                        realm: entry.realm,
                        login,
                        accept_invalid_certs: entry.accept_invalid_certs,
                    })
                    .await?;
            }
        }
    }
    Ok(())
}

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
}

fn error_message(code: &str) -> &'static str {
    match code {
        "name" => "Another environment already has that name.",
        "url" => "The address has to start with https:// or http://.",
        _ => "Fill in every field except the token name.",
    }
}

fn back(cx: &Cx, error: Option<&str>) -> String {
    let list = href!(page).resolve(cx);
    match error {
        Some(code) => format!("{list}?error={code}"),
        None => list,
    }
}

/// How the environment signs in, in words.
fn signs_in_as(environment: &ProxmoxEnvironment) -> String {
    let user = format!("{}@{}", environment.user, environment.realm);
    match &environment.login {
        ProxmoxLogin::ApiToken { id, .. } => format!("{user} with API token {id}"),
        ProxmoxLogin::Password(_) => format!("{user} with a password"),
    }
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    require_user(cx).await?;
    let query = query_params::<Query>(cx)?;
    let environments = database(cx).proxmox_environments().await?;

    Ok(view! {
        <div class="max-w-3xl mx-auto mt-12 flex flex-col gap-y-8">
            <div>
                <h2 class="text-2xl font-bold text-secondary-50">"Proxmox"</h2>
                <p class="text-secondary-300">
                    "The Proxmox VE environments Lemnos manages."
                </p>
            </div>

            <div class="table">
                <table>
                    <thead>
                        <tr>
                            <th>"Name"</th>
                            <th>"Address"</th>
                            <th>"Signs in as"</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        if environments.is_empty() {
                            <tr>
                                <td colspan="4" class="text-secondary-300">
                                    "None yet. Add one below."
                                </td>
                            </tr>
                        }
                        for known in &environments {
                            <tr>
                                <td>
                                    <a
                                        href=(href!(
                                            environment::page,
                                            environment::Environment(known.id.0),
                                        ))
                                        class="font-medium text-primary-200 hover:underline"
                                    >
                                        (known.name.as_str())
                                    </a>
                                </td>
                                <td>(known.url.as_str())</td>
                                <td>(signs_in_as(known))</td>
                                <td class="text-right">
                                    <form method="post" action=(href!(remove))>
                                        <input
                                            type="hidden"
                                            name="id"
                                            value=(known.id.0.to_string())
                                        >
                                        <button type="submit" class="button button-danger">
                                            "Remove"
                                        </button>
                                    </form>
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            </div>

            <section class="p-6 rounded-lg bg-secondary-700 flex flex-col gap-y-4">
                <h3 class="text-lg font-bold text-secondary-50">
                    "Add an environment"
                </h3>
                if let Some(code) = &query.error {
                    <p class="text-sm text-danger-200">(error_message(code))</p>
                }
                <form method="post" class="flex flex-col gap-y-4">
                    <div class="flex flex-col gap-y-1">
                        <label for="name" class="label">"Name"</label>
                        <div class="input-wrapper">
                            <input
                                type="text"
                                id="name"
                                name="name"
                                maxlength="64"
                                placeholder="e.g. Lab"
                                required=""
                            >
                        </div>
                    </div>
                    <div class="flex flex-col gap-y-1">
                        <label for="url" class="label">"Address"</label>
                        <div class="input-wrapper">
                            <input
                                type="url"
                                id="url"
                                name="url"
                                maxlength="255"
                                placeholder="https://pve.example.com:8006"
                                required=""
                            >
                        </div>
                    </div>
                    <div class="grid grid-cols-2 gap-x-4">
                        <div class="flex flex-col gap-y-1">
                            <label for="user" class="label">"User"</label>
                            <div class="input-wrapper">
                                <input
                                    type="text"
                                    id="user"
                                    name="user"
                                    maxlength="64"
                                    placeholder="root"
                                    autocomplete="off"
                                    required=""
                                >
                            </div>
                        </div>
                        <div class="flex flex-col gap-y-1">
                            <label for="realm" class="label">"Realm"</label>
                            <div class="input-wrapper">
                                <input
                                    type="text"
                                    id="realm"
                                    name="realm"
                                    maxlength="32"
                                    placeholder="pam"
                                    required=""
                                >
                            </div>
                        </div>
                    </div>
                    <div class="flex flex-col gap-y-1">
                        <label for="token_id" class="label">
                            "API token name (leave empty to sign in with the user's password)"
                        </label>
                        <div class="input-wrapper">
                            <input
                                type="text"
                                id="token_id"
                                name="token_id"
                                maxlength="64"
                                placeholder="lemnos"
                                autocomplete="off"
                            >
                        </div>
                    </div>
                    <div class="flex flex-col gap-y-1">
                        <label for="secret" class="label">
                            "Token secret, or the password"
                        </label>
                        <div class="input-wrapper">
                            <input
                                type="password"
                                id="secret"
                                name="secret"
                                autocomplete="off"
                                required=""
                            >
                        </div>
                    </div>
                    <label class="flex items-center gap-x-2 text-secondary-200">
                        <input type="checkbox" name="accept_invalid_certs" value="on">
                        "Accept a self-signed certificate"
                    </label>
                    <button type="submit" class="button button-primary">"Add"</button>
                </form>
            </section>
        </div>
    })
}

#[derive(Deserialize)]
pub struct NewEnvironmentForm {
    name: String,
    url: String,
    user: String,
    realm: String,
    token_id: String,
    secret: String,
    // Browsers leave an unticked checkbox out of the form altogether.
    accept_invalid_certs: Option<String>,
}

#[route(POST)]
pub async fn add(cx: &Cx, Form(input): Form<NewEnvironmentForm>) -> Result<SeeOther> {
    require_user(cx).await?;
    let name = input.name.trim();
    let url = input.url.trim();
    let user = input.user.trim();
    let realm = input.realm.trim();
    let token_id = input.token_id.trim();
    if [name, url, user, realm].contains(&"") || input.secret.is_empty() {
        return Ok(see_other(back(cx, Some("fields"))));
    }
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Ok(see_other(back(cx, Some("url"))));
    }

    let new = NewProxmoxEnvironment {
        name: name.to_owned(),
        url: url.to_owned(),
        user: user.to_owned(),
        realm: realm.to_owned(),
        login: if token_id.is_empty() {
            ProxmoxLogin::Password(input.secret)
        } else {
            ProxmoxLogin::ApiToken {
                id: token_id.to_owned(),
                secret: input.secret,
            }
        },
        accept_invalid_certs: input.accept_invalid_certs.is_some(),
    };
    Ok(see_other(
        match database(cx).create_proxmox_environment(new).await {
            Ok(created) => {
                href!(environment::page, environment::Environment(created.id.0)).resolve(cx)
            }
            Err(ProxmoxStoreError::NameTaken) => back(cx, Some("name")),
            Err(error) => return Err(error.into()),
        },
    ))
}

#[derive(Deserialize)]
pub struct RemoveForm {
    id: u64,
}

// Only forgets the environment here; nothing changes on the Proxmox side.
#[route(POST "./remove")]
pub async fn remove(cx: &Cx, Form(input): Form<RemoveForm>) -> Result<SeeOther> {
    require_user(cx).await?;
    database(cx)
        .delete_proxmox_environment(ProxmoxEnvironmentId(input.id))
        .await?;
    Ok(see_other(back(cx, None)))
}
