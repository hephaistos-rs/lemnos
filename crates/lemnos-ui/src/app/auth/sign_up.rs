// GET /auth/sign-up

use topcoat::{
    Result,
    router::page,
    view::{View, view},
};

#[page]
pub async fn page() -> Result<impl View> {
    Ok(view! {
        <h2 class="header">"Sign up"</h2>
    })
}
