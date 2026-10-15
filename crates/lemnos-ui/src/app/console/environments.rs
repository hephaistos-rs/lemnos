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
use lemnos_proxmox::{
    client::{Client, Credentials, Endpoint},
    token::{self, TokenIdError},
};
use serde::Deserialize;
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{SeeOther, see_other},
        href, page, query_params, route,
    },
    view::{View, component, view},
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

/// Who signs in, and how: the user, realm and login, from what was entered.
///
/// `token_id` is empty for a password login. Otherwise it is the token's
/// name or the full ID Proxmox shows (`root@pam!lemnos`), which may leave
/// `user` and `realm` empty; see [`token::split`].
pub fn login_from(
    token_id: &str,
    user: &str,
    realm: &str,
    secret: String,
) -> std::result::Result<(String, String, ProxmoxLogin), TokenIdError> {
    if token_id.trim().is_empty() {
        let (user, realm) = (user.trim(), realm.trim());
        if user.is_empty() {
            return Err(TokenIdError::MissingUser);
        }
        if realm.is_empty() {
            return Err(TokenIdError::MissingRealm);
        }
        return Ok((
            user.to_owned(),
            realm.to_owned(),
            ProxmoxLogin::Password(secret),
        ));
    }
    let owner = token::split(token_id, user, realm)?;
    Ok((
        owner.user,
        owner.realm,
        ProxmoxLogin::ApiToken {
            id: owner.name,
            secret,
        },
    ))
}

/// The `?error=` code for a token ID or user that doesn't add up.
pub fn token_error_code(error: &TokenIdError) -> &'static str {
    match error {
        TokenIdError::Empty | TokenIdError::MissingUser | TokenIdError::MissingRealm => "owner",
        TokenIdError::UserConflict { .. } => "token-user",
        TokenIdError::RealmConflict { .. } => "token-realm",
        TokenIdError::ExtraSeparator
        | TokenIdError::MissingName
        | TokenIdError::UserOnly(_)
        | TokenIdError::BadName => "token",
    }
}

/// Writes the environments from the config file to the database: new names
/// are added, and known ones are made to match the file again. Returns the
/// names it wrote: those are the file's to change, not the UI's.
pub async fn sync_config(
    database: &Database,
    configured: Vec<ProxmoxConfig>,
) -> Result<Vec<String>, Box<dyn Error>> {
    let stored = database.proxmox_environments().await?;
    let mut managed = Vec::new();
    for entry in configured {
        let invalid = |problem: &dyn std::fmt::Display| {
            format!("proxmox environment `{}`: {problem}", entry.name)
        };
        let (token_id, secret) = match (&entry.token_id, entry.token_secret, entry.password) {
            (Some(id), Some(secret), None) => (id.as_str(), secret.expose().to_owned()),
            (None, None, Some(password)) => ("", password.expose().to_owned()),
            _ => {
                return Err(invalid(&"set either token_id and token_secret, or password").into());
            }
        };
        let (user, realm, login) = login_from(token_id, &entry.user, &entry.realm, secret)
            .map_err(|error| invalid(&error))?;
        managed.push(entry.name.clone());
        match stored.iter().find(|known| known.name == entry.name) {
            Some(known) => {
                database
                    .update_proxmox_environment(ProxmoxEnvironment {
                        id: known.id,
                        name: entry.name,
                        url: entry.url,
                        user,
                        realm,
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
                        user,
                        realm,
                        login,
                        accept_invalid_certs: entry.accept_invalid_certs,
                    })
                    .await?;
            }
        }
    }
    Ok(managed)
}

#[query_params(error = bad_request)]
struct Query {
    error: Option<String>,
}

/// What a `?error=` code from the add or edit form means.
pub fn error_message(code: &str) -> &'static str {
    match code {
        "name" => "Another environment already has that name.",
        "url" => "The address has to start with https:// or http://.",
        "owner" => {
            "Fill in the user and realm, or give the full API token ID, like root@pam!lemnos."
        }
        "token" => {
            "That API token ID doesn't look right. Give it the way Proxmox shows it, like root@pam!lemnos, or just the part after the !."
        }
        "token-user" => {
            "The user in the API token ID isn't the one in the User field. Make them match, or leave User empty."
        }
        "token-realm" => {
            "The realm in the API token ID isn't the one in the Realm field. Make them match, or leave Realm empty."
        }
        "secret" => {
            "Enter the secret again: the one Lemnos has belongs to the user or token you changed."
        }
        "managed" => {
            "This environment is managed by the config file. Change it there; Lemnos reads the file again when it starts."
        }
        _ => "Fill in the name, the address and the secret.",
    }
}

fn back(cx: &Cx, error: Option<&str>) -> String {
    let list = href!(page).resolve(cx);
    match error {
        Some(code) => format!("{list}?error={code}"),
        None => list,
    }
}

pub fn valid_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// How the environment signs in, in words.
fn signs_in_as(environment: &ProxmoxEnvironment) -> String {
    let user = format!("{}@{}", environment.user, environment.realm);
    match &environment.login {
        ProxmoxLogin::ApiToken { id, .. } => format!("API token {user}!{id}"),
        ProxmoxLogin::Password(_) => format!("{user} with a password"),
    }
}

/// Whether the config file declares this environment. Sync on start would
/// overwrite changes made here, so those are read-only in the UI.
pub fn managed_by_config(cx: &Cx, environment: &ProxmoxEnvironment) -> bool {
    state(cx)
        .managed_environments
        .contains(&environment.name)
}

/// What the add and edit forms start out with. Never holds a secret.
#[derive(Default)]
pub struct Fields {
    pub name: String,
    pub url: String,
    pub user: String,
    pub realm: String,
    /// In the full form Proxmox shows, so it reads the same as there.
    pub token_id: String,
    pub accept_invalid_certs: bool,
    /// Editing a stored environment: the secret may be left empty.
    pub editing: bool,
}

impl Fields {
    pub fn of(environment: &ProxmoxEnvironment) -> Self {
        Self {
            name: environment.name.clone(),
            url: environment.url.clone(),
            user: environment.user.clone(),
            realm: environment.realm.clone(),
            token_id: match &environment.login {
                ProxmoxLogin::ApiToken { id, .. } => {
                    format!("{}@{}!{id}", environment.user, environment.realm)
                }
                ProxmoxLogin::Password(_) => String::new(),
            },
            accept_invalid_certs: environment.accept_invalid_certs,
            editing: true,
        }
    }
}

// The fields of the add and edit forms, without the form and its button.
#[component]
pub async fn environment_fields(fields: &Fields) -> Result<impl View> {
    let secret_hint = if fields.editing {
        "Leave empty to keep the current one. It isn't shown again once saved."
    } else {
        "The token's secret, or the user's password."
    };
    Ok(view! {
        <div class="field">
            <label for="name" class="label">"Name"</label>
            <input
                type="text"
                id="name"
                name="name"
                class="input"
                maxlength="64"
                placeholder="e.g. Lab"
                value=(fields.name.as_str())
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
                value=(fields.url.as_str())
                required=""
            >
        </div>
        <div class="field">
            <label for="token_id" class="label">"API token ID"</label>
            <input
                type="text"
                id="token_id"
                name="token_id"
                class="input"
                maxlength="200"
                placeholder="root@pam!lemnos"
                value=(fields.token_id.as_str())
                autocomplete="off"
            >
            <p class="hint">
                "As Proxmox shows it under Datacenter > Permissions > API Tokens, e.g. root@pam!lemnos. Leave empty to sign in with a password instead."
            </p>
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
                    value=(fields.user.as_str())
                    autocomplete="off"
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
                    value=(fields.realm.as_str())
                >
            </div>
        </div>
        <p class="hint -mt-2">"Only needed for a password, or a token ID without the part before the !."</p>
        <div class="field">
            <label for="secret" class="label">"Secret"</label>
            if fields.editing {
                <input
                    type="password"
                    id="secret"
                    name="secret"
                    class="input"
                    autocomplete="off"
                >
            } else {
                <input
                    type="password"
                    id="secret"
                    name="secret"
                    class="input"
                    autocomplete="off"
                    required=""
                >
            }
            <p class="hint">(secret_hint)</p>
        </div>
        <label class="check">
            if fields.accept_invalid_certs {
                <input type="checkbox" name="accept_invalid_certs" value="on" checked="">
            } else {
                <input type="checkbox" name="accept_invalid_certs" value="on">
            }
            "Accept a self-signed certificate"
        </label>
    })
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    admin_access(cx).await?;
    let query = query_params::<Query>(cx)?;
    let environments = database(cx).proxmox_environments().await?;
    let personal = database(cx).personal_environment().await?;
    let blank = Fields::default();

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
                                    <th><span class="sr-only">"Actions"</span></th>
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
                                                if managed_by_config(cx, known) {
                                                    <span class="text-meta text-ink-faint">"Managed by config file"</span>
                                                } else {
                                                    <a
                                                        href=(href!(
                                                            environment::edit::page,
                                                            environment::Environment(known.id.0),
                                                        ))
                                                        class="button button-sm"
                                                    >
                                                        "Edit"
                                                    </a>
                                                }
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

                    if !environments.is_empty() {
                        <section class="panel">
                            <h2 class="section-title">"Personal machines"</h2>
                            <p class="text-ink-muted max-w-[65ch]">
                                "Everyone may create one machine of their own, from the templates in this environment. Admins may create as many as they need, in any environment."
                            </p>
                            <form method="post" action=(href!(set_personal)) class="inline-form">
                                <select name="environment" class="input" aria-label="Environment for personal machines">
                                    if personal.is_none() {
                                        <option value="" selected="">"None: nobody can create one"</option>
                                    } else {
                                        <option value="">"None: nobody can create one"</option>
                                    }
                                    for known in &environments {
                                        if Some(known.id) == personal {
                                            <option value=(known.id.0.to_string()) selected="">(known.name.as_str())</option>
                                        } else {
                                            <option value=(known.id.0.to_string())>(known.name.as_str())</option>
                                        }
                                    }
                                </select>
                                <button type="submit" class="button">"Save"</button>
                            </form>
                        </section>
                    }
                </div>

                <section class="panel">
                    <h2 class="section-title">"Add an environment"</h2>
                    if let Some(code) = &query.error {
                        notice(tone: "danger", message: error_message(code))
                    }
                    <form method="post" class="flex flex-col gap-y-4">
                        environment_fields(fields: &blank)
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
    if [name, url].contains(&"") || input.secret.is_empty() {
        return Ok(see_other(back(cx, Some("fields"))));
    }
    if !valid_url(url) {
        return Ok(see_other(back(cx, Some("url"))));
    }
    let (user, realm, login) =
        match login_from(&input.token_id, &input.user, &input.realm, input.secret) {
            Ok(login) => login,
            Err(error) => return Ok(see_other(back(cx, Some(token_error_code(&error))))),
        };

    let new = NewProxmoxEnvironment {
        name: name.to_owned(),
        url: url.to_owned(),
        user,
        realm,
        login,
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
pub struct PersonalForm {
    // Empty for none.
    environment: String,
}

// Where personal machines are created.
#[route(POST "./personal")]
pub async fn set_personal(cx: &Cx, Form(input): Form<PersonalForm>) -> Result<SeeOther> {
    admin_access(cx).await?;
    let database = database(cx);
    let chosen = match input.environment.parse() {
        Ok(id) => database
            .proxmox_environment(ProxmoxEnvironmentId(id))
            .await?
            .map(|environment| environment.id),
        Err(_) => None,
    };
    database.set_personal_environment(chosen).await?;
    Ok(see_other(back(cx, None)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_token_id_fills_in_user_and_realm() {
        let (user, realm, login) = login_from("root@pam!lemnos", "", "", "s".into()).unwrap();
        assert_eq!((user.as_str(), realm.as_str()), ("root", "pam"));
        assert_eq!(
            login,
            ProxmoxLogin::ApiToken {
                id: "lemnos".into(),
                secret: "s".into()
            }
        );
        // Stored the way it always was, so older rows read the same.
        let (_, _, short) = login_from("lemnos", "root", "pam", "s".into()).unwrap();
        assert_eq!(short, login);
    }

    #[test]
    fn a_password_login_needs_user_and_realm() {
        assert!(matches!(
            login_from("", "root", "pam", "pw".into()),
            Ok((_, _, ProxmoxLogin::Password(_)))
        ));
        let missing = login_from("", "", "pam", "pw".into()).unwrap_err();
        assert_eq!(token_error_code(&missing), "owner");
    }

    #[test]
    fn a_conflict_gets_its_own_message() {
        let conflict = login_from("root@pam!lemnos", "root", "pve", "s".into()).unwrap_err();
        assert_eq!(token_error_code(&conflict), "token-realm");
        assert!(error_message("token-realm").contains("realm"));
    }
}
