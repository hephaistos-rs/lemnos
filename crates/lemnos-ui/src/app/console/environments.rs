// /environments: the environments Lemnos manages. Admins only: this is
// where addresses and service logins are kept.
// Proxmox VE is the only provider so far. This page lists them and adds or
// removes one; `environment.rs` shows what runs on one.
// Talking to Proxmox itself is `lemnos-proxmox`'s job.

use std::error::Error;

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
        error::{SeeOther, see_other},
        href, page, query_params, route,
    },
    view::{View, view},
};

use crate::{access::admin_access, app::auth::state, components::notice, config::ProxmoxConfig};

pub mod environment;

pub fn database(cx: &Cx) -> &Database {
    state(cx).auth.store()
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
    admin_access(cx).await?;
    let query = query_params::<Query>(cx)?;
    let environments = database(cx).proxmox_environments().await?;

    Ok(view! {
        <div class="page">
            <div class="page-head">
                <div>
                    <h1>"Environments"</h1>
                    <p>"Where machines run. Lemnos signs in to each one to list and manage them."</p>
                </div>
            </div>

            // The list takes the room that is left; the form keeps its width.
            <div class="grid gap-8 lg:grid-cols-[minmax(0,1fr)_24rem] items-start">
                <div class="flex flex-col gap-y-3">
                    <div class="data data-stack">
                        <table>
                            <thead>
                                <tr>
                                    <th>"Name"</th>
                                    <th>"Provider"</th>
                                    <th>"Address"</th>
                                    <th>"Signs in as"</th>
                                    <th><span class="sr-only">"Remove"</span></th>
                                </tr>
                            </thead>
                            <tbody>
                                if environments.is_empty() {
                                    <tr>
                                        <td colspan="5" class="empty wide">
                                            "No environments are connected yet. Add the first one with the form."
                                        </td>
                                    </tr>
                                }
                                for known in &environments {
                                    <tr>
                                        <td data-label="Name" class="name">
                                            <a
                                                href=(href!(
                                                    environment::page,
                                                    environment::Environment(known.id.0),
                                                ))
                                                class="link"
                                            >
                                                (known.name.as_str())
                                            </a>
                                        </td>
                                        // The only provider so far.
                                        <td data-label="Provider">"Proxmox VE"</td>
                                        <td data-label="Address" class="figure">
                                            (known.url.as_str())
                                        </td>
                                        <td data-label="Signs in as">(signs_in_as(known))</td>
                                        <td class="wide">
                                            <div class="actions">
                                                <form method="post" action=(href!(remove))>
                                                    <input
                                                        type="hidden"
                                                        name="id"
                                                        value=(known.id.0.to_string())
                                                    >
                                                    <button
                                                        type="submit"
                                                        class="button button-sm button-danger"
                                                    >
                                                        "Remove"
                                                    </button>
                                                </form>
                                            </div>
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </div>
                    <p class="text-meta text-ink-faint">
                        "Removing an environment only makes Lemnos forget it, and takes it away from the organisations that were given it. Nothing changes on the provider's side."
                    </p>
                </div>

                <section class="panel">
                    <h2 class="section-title">"Add an environment"</h2>
                    if let Some(code) = &query.error {
                        notice(tone: "danger", message: error_message(code))
                    }
                    <form method="post" class="flex flex-col gap-y-4">
                        <div class="field">
                            <label for="name" class="label">"Name"</label>
                            <input
                                type="text"
                                id="name"
                                name="name"
                                class="input"
                                maxlength="64"
                                placeholder="e.g. Lab"
                                required=""
                            >
                        </div>
                        <div class="field">
                            <label for="url" class="label">"Address"</label>
                            <input
                                type="url"
                                id="url"
                                name="url"
                                class="input"
                                maxlength="255"
                                placeholder="https://pve.example.com:8006"
                                required=""
                            >
                        </div>
                        <div class="grid grid-cols-2 gap-x-4">
                            <div class="field">
                                <label for="user" class="label">"User"</label>
                                <input
                                    type="text"
                                    id="user"
                                    name="user"
                                    class="input"
                                    maxlength="64"
                                    placeholder="root"
                                    autocomplete="off"
                                    required=""
                                >
                            </div>
                            <div class="field">
                                <label for="realm" class="label">"Realm"</label>
                                <input
                                    type="text"
                                    id="realm"
                                    name="realm"
                                    class="input"
                                    maxlength="32"
                                    placeholder="pam"
                                    required=""
                                >
                            </div>
                        </div>
                        <div class="field">
                            <label for="token_id" class="label">"API token name"</label>
                            <input
                                type="text"
                                id="token_id"
                                name="token_id"
                                class="input"
                                maxlength="64"
                                placeholder="lemnos"
                                autocomplete="off"
                            >
                            <p class="hint">"Leave empty to sign in with the user's password."</p>
                        </div>
                        <div class="field">
                            <label for="secret" class="label">"Secret"</label>
                            <input
                                type="password"
                                id="secret"
                                name="secret"
                                class="input"
                                autocomplete="off"
                                required=""
                            >
                            <p class="hint">"The token's secret, or the user's password."</p>
                        </div>
                        <label class="check">
                            <input type="checkbox" name="accept_invalid_certs" value="on">
                            "Accept a self-signed certificate"
                        </label>
                        <button type="submit" class="button button-primary self-start">
                            "Add environment"
                        </button>
                    </form>
                </section>
            </div>
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
    admin_access(cx).await?;
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
    admin_access(cx).await?;
    database(cx)
        .delete_proxmox_environment(ProxmoxEnvironmentId(input.id))
        .await?;
    Ok(see_other(back(cx, None)))
}
