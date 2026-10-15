// /settings/account: who the signed-in user is here, and which
// organisations they are in.

use topcoat::{
    Result,
    context::Cx,
    router::{href, page},
    view::{View, view},
};

use crate::{
    access::{Scope, access},
    app::console::organizations::{self, organization, role_label},
};

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let access = access(cx).await?;
    let looking_at = match &access.scope {
        Scope::Organization(membership) => Some(membership.organization.id),
        Scope::Everything | Scope::Nothing => None,
    };

    Ok(view! {
        <div class="page-mid">
            <div class="page-head">
                <div>
                    <h1>"Account"</h1>
                    <p>"Who you are in this Lemnos."</p>
                </div>
            </div>

            <section class="panel">
                <h2 class="section-title">"Profile"</h2>
                <dl class="facts">
                    <dt>"Username"</dt>
                    <dd class="figure text-base">(access.user.username.as_str())</dd>
                    <dt>"Name"</dt>
                    <dd>(access.user.name.as_str())</dd>
                    <dt>"Email"</dt>
                    <dd>(access.user.email.as_deref().unwrap_or("Not known"))</dd>
                    <dt>"Admin"</dt>
                    <dd>
                        if access.admin {
                            <span class="state state-running">"Yes, of this installation"</span>
                        } else {
                            <span class="state state-stopped">"No"</span>
                        }
                    </dd>
                </dl>
                <p class="hint">
                    "Name and email come from how you signed in the first time and can't be changed here yet."
                </p>
            </section>

            <section class="panel">
                <h2 class="section-title">"Organisations"</h2>
                if access.memberships.is_empty() {
                    <p class="text-ink-muted max-w-[65ch]">
                        "You are not in an organisation yet. An admin or an organisation's owner can add you; they need your username."
                    </p>
                } else {
                    <dl class="facts">
                        for membership in &access.memberships {
                            <dt>
                                <a
                                    href=(href!(
                                        organization::page,
                                        organization::Organization(membership.organization.id.0),
                                    ))
                                    class="link"
                                >
                                    (membership.organization.name.as_str())
                                </a>
                            </dt>
                            <dd>
                                (role_label(membership.role))
                                if looking_at == Some(membership.organization.id) {
                                    <span class="text-ink-faint">", the one you are looking at"</span>
                                }
                            </dd>
                        }
                    </dl>
                }
                <a href=(href!(organizations::page)) class="button self-start">"Open organisations"</a>
            </section>
        </div>
    })
}
