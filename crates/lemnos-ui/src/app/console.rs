// The application itself: every page in here is for signed-in users. A
// group, so it adds nothing to the URL (`console/machines.rs` -> `/machines`).

use serde::{Deserialize, Serialize};
use topcoat::{
    Result,
    context::Cx,
    router::{
        Body, Method, Next, Slot,
        content::Form,
        error::{SeeOther, see_other},
        href, layer, layout,
        request::{method, uri},
        response::Response,
        route, segment,
    },
    view::{View, view},
};

use crate::{
    app::auth::{current_user, sign_in},
    components::{Theme, footer, shell_bar},
};

pub mod environments;
pub mod machines;
pub mod organizations;
pub mod settings;
pub mod users;
pub mod welcome;

segment!(kind = Group);

#[derive(Serialize)]
struct Return<'a> {
    next: &'a str,
}

// Runs before every page and route in the console. A visitor is sent to
// sign in, and brought back to the page they asked for afterwards.
#[layer]
async fn signed_in_only(cx: &Cx, body: Body, next: Next<'_>) -> Result<Response> {
    if current_user(cx).await?.is_some() {
        return next.run(cx, body).await;
    }
    let sign_in = href!(sign_in::page);
    // Only a page that was being opened is worth coming back to; a form
    // that was being sent can't be sent again from there.
    let asked_for = uri(cx).path_and_query().map(|path| path.as_str());
    Err(see_other(match asked_for {
        Some(path) if method(cx) == Method::GET => sign_in.query(Return { next: path }).resolve(cx),
        _ => sign_in.resolve(cx),
    })
    .into())
}

// Wraps every console page with the bar, the main content area and the footer.
#[layout]
async fn console_shell(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let theme = Theme::current(cx);

    Ok(view! {
        shell_bar(theme: theme)
        <main class="flex-1">
            (slot)
        </main>
        footer(theme: theme)
    })
}

#[derive(Deserialize)]
pub struct ScopeForm {
    organization: String,
}

// POST /scope: switches to another of the user's organisations (or, for an
// admin, to everything), then shows its machines.
#[route(POST "./scope")]
pub async fn choose_scope(cx: &Cx, Form(input): Form<ScopeForm>) -> Result<SeeOther> {
    crate::access::choose(cx, &input.organization);
    Ok(see_other(href!(machines::page).resolve(cx)))
}
