// /welcome: what to do first. An admin of a new installation gets the steps
// to a first working environment; someone who is in no organisation yet is
// pointed at their personal machine, and told who can add them.

use topcoat::{
    Result,
    context::Cx,
    router::{href, page},
    view::{View, component, view},
};

use super::{environments, machines, organizations};
use crate::access::{access, database};

// One step: whether it is done, what it is, and where to do it. `next` marks
// the one to do now, which gets the bronze button.
#[component]
async fn step(
    done: bool,
    next: bool,
    title: &str,
    about: &str,
    to: String,
    action: &str,
) -> Result<impl View> {
    Ok(view! {
        <li class=(if done { "done" } else { "" })>
            <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
                <h2 class="font-semibold">(title)</h2>
                if done {
                    <span class="state state-running">"Done"</span>
                } else {
                    <span class="state state-stopped">"To do"</span>
                }
            </div>
            <p class="text-ink-muted">(about)</p>
            <a href=(to) class=(if next { "button button-primary self-start" } else { "button self-start" })>
                (action)
            </a>
        </li>
    })
}

#[page]
pub async fn page(cx: &Cx) -> Result<impl View> {
    let access = access(cx).await?;
    let database = database(cx);

    let environments = database.proxmox_environments().await?;
    let organizations = database.organizations().await?;
    // Whether any organisation has both an environment and somebody in it.
    let mut handed_over = false;
    if access.admin {
        for organization in &organizations {
            if !database.grants(organization.id).await?.is_empty()
                && !database.members(organization.id).await?.is_empty()
            {
                handed_over = true;
                break;
            }
        }
    }
    let admins: Vec<String> = if access.admin {
        Vec::new()
    } else {
        database
            .accounts()
            .await?
            .into_iter()
            .filter(|account| account.admin)
            .map(|account| account.name)
            .collect()
    };
    let waiting = access.memberships.is_empty();
    // The first step that is still open is the one to do now.
    let done = [
        !environments.is_empty(),
        !organizations.is_empty(),
        handed_over,
    ];
    let next = done.iter().position(|done| !done);

    Ok(view! {
        <div class="page-mid">
            if access.admin {
                <div class="page-head">
                    <div>
                        <h1>"Welcome to Lemnos"</h1>
                        <p>"You run this installation. Three steps take it from empty to in use."</p>
                    </div>
                </div>

                <ol class="steps steps-setup">
                    step(
                        done: done[0],
                        next: next == Some(0),
                        title: "Connect an environment",
                        about: "Tell Lemnos where machines run and how to sign in there. Proxmox VE is the provider so far.",
                        to: href!(environments::page).resolve(cx).to_string(),
                        action: if environments.is_empty() { "Add an environment" } else { "Open environments" },
                    )
                    step(
                        done: done[1],
                        next: next == Some(1),
                        title: "Create an organisation",
                        about: "An organisation is a group of people who share machines. Several can use the same environment.",
                        to: href!(organizations::page).resolve(cx).to_string(),
                        action: if organizations.is_empty() { "Add an organisation" } else { "Open organisations" },
                    )
                    step(
                        done: done[2],
                        next: next == Some(2),
                        title: "Hand it over",
                        about: "Give the organisation an environment, with the tag that marks its machines there, and add the people who belong to it.",
                        to: href!(organizations::page).resolve(cx).to_string(),
                        action: "Choose an organisation",
                    )
                </ol>

                if !environments.is_empty() {
                    <p class="text-ink-muted">
                        "As an admin you already see every machine. "
                        <a href=(href!(machines::page)) class="link">"Go to machines"</a>
                    </p>
                }
            } else if waiting {
                <div class="page-head">
                    <div>
                        <h1>"You're in"</h1>
                        <p>"Your account works. Start with a machine of your own, or join an organisation to share its machines."</p>
                    </div>
                </div>

                <section class="panel">
                    <h2 class="section-title">"Your own machine"</h2>
                    <p class="text-ink-muted max-w-[65ch]">
                        "Everyone gets one personal machine, copied from a template and ready to start."
                    </p>
                    <a href=(href!(machines::new::page)) class="button button-primary self-start">"Create your machine"</a>
                </section>

                <section class="panel">
                    <h2 class="section-title">"Join an organisation"</h2>
                    <p class="text-ink-muted max-w-[65ch]">
                        "An admin or an organisation's owner can add you. They need your username to find you:"
                    </p>
                    <p class="figure text-base">(access.user.username.as_str())</p>
                    if !admins.is_empty() {
                        <p class="text-ink-muted max-w-[65ch]">
                            "Admins of this installation: " (admins.join(", ")) "."
                        </p>
                    }
                </section>
            } else {
                <div class="page-head">
                    <div>
                        <h1>"You're all set"</h1>
                        <p>"You are in " (access.scope_name()) "."</p>
                    </div>
                </div>
                <a href=(href!(machines::page)) class="button button-primary self-start">"Go to machines"</a>
            }
        </div>
    })
}
