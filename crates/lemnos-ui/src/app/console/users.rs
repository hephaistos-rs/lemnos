// /users: every account, and who is an admin. Admins only.
//
// Accounts are not made here: people get one by signing in or signing up,
// and are then added to organisations on the organisation's page.

use lemnos_auth::UserId;
use serde::Deserialize;
use topcoat::{
    Result,
    context::Cx,
    router::{
        content::Form,
        error::{SeeOther, see_other},
        href, page, route,
    },
    view::{View, view},
};

use crate::access::{admin_access, database};

/// One account, as its row shows it.
struct Row {
    id: u64,
    username: String,
    name: String,
    admin: bool,
    organizations: String,
    own: bool,
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let access = admin_access(cx).await?;
    let database = database(cx);

    let mut rows = Vec::new();
    for account in database.accounts().await? {
        let organizations: Vec<String> = database
            .memberships_of(account.user)
            .await?
            .into_iter()
            .map(|membership| membership.organization.name)
            .collect();
        rows.push(Row {
            id: account.user.0,
            own: account.user == access.user.id,
            username: account.username,
            name: account.name,
            admin: account.admin,
            organizations: organizations.join(", "),
        });
    }

    Ok(view! {
        <div class="page">
            <div class="page-head">
                <div>
                    <h1>"Users"</h1>
                    <p>"Everyone with an account here. People get one by signing in; organisations are where they are given machines."</p>
                </div>
            </div>

            <div class="data data-stack">
                <table>
                    <thead>
                        <tr>
                            <th>"Username"</th>
                            <th>"Name"</th>
                            <th>"Organisations"</th>
                            <th>"Admin"</th>
                            <th><span class="sr-only">"Change"</span></th>
                        </tr>
                    </thead>
                    <tbody>
                        for row in &rows {
                            <tr>
                                <td data-label="Username" class="name">(row.username.as_str())</td>
                                <td data-label="Name">(row.name.as_str())</td>
                                <td data-label="Organisations" class=(if row.organizations.is_empty() { "blank" } else { "" })>
                                    (row.organizations.as_str())
                                </td>
                                <td data-label="Admin">
                                    if row.admin {
                                        <span class="state state-running">"Admin"</span>
                                    } else {
                                        <span class="state state-stopped">"No"</span>
                                    }
                                </td>
                                <td class="wide">
                                    // Nobody takes their own admin rights away by accident.
                                    if !row.own {
                                        <div class="actions">
                                            <form method="post" action=(href!(set_admin))>
                                                <input type="hidden" name="user" value=(row.id.to_string())>
                                                if row.admin {
                                                    <input type="hidden" name="admin" value="no">
                                                    <button type="submit" class="button button-sm button-danger">
                                                        "Remove admin"
                                                    </button>
                                                } else {
                                                    <input type="hidden" name="admin" value="yes">
                                                    <button type="submit" class="button button-sm">
                                                        "Make admin"
                                                    </button>
                                                }
                                            </form>
                                        </div>
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            </div>
            <p class="text-meta text-ink-faint">
                "An admin connects environments, creates organisations and sees every machine."
            </p>
        </div>
    })
}

#[derive(Deserialize)]
pub struct AdminForm {
    user: u64,
    admin: String,
}

#[route(POST "./admin")]
pub async fn set_admin(cx: &Cx, Form(input): Form<AdminForm>) -> Result<SeeOther> {
    let access = admin_access(cx).await?;
    let user = UserId(input.user);
    if user != access.user.id {
        database(cx).set_admin(user, input.admin == "yes").await?;
    }
    Ok(see_other(href!(page).resolve(cx)))
}
