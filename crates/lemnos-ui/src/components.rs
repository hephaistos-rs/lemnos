// Reusable pieces used inside pages. These are not routes.

use topcoat::{
    Result,
    context::Cx,
    router::href,
    view::{View, component, view},
};

use crate::app::auth::current_user;

#[component]
pub async fn hello(name: &str) -> Result<impl View> {
    Ok(view! {
        <h1>"Hello, " (name) "!"</h1>
    })
}

// Shows sign in/up links to visitors, and the user's name plus a sign-out
// button once signed in. `cx` is passed in by Topcoat: call it as `nav_bar()`.
#[component]
pub async fn nav_bar(cx: &Cx) -> Result<impl View> {
    let user = current_user(cx).await?;

    Ok(view! {
        <nav class="flex items-center gap-x-6">
            <div class="hidden md:flex items-center gap-x-1">
                <a href=(href!(crate::app::home)) class="nav-link">"Home"</a>
                <a href="/about" class="nav-link">"About"</a>
                <a href="/contact" class="nav-link">"Contact"</a>
            </div>
            <div class="flex items-center gap-x-2">
                if let Some(user) = &user {
                    <a href=(href!(crate::app::account::page)) class="nav-link">(&user.name)</a>
                    // A form, not a link: signing out changes state, so it must be a POST.
                    <form method="post" action=(href!(crate::app::auth::sign_out::sign_out))>
                        <button type="submit" class="button">"Sign out"</button>
                    </form>
                } else {
                    <a href=(href!(crate::app::auth::sign_in::page)) class="button">"Sign in"</a>
                    <a href=(href!(crate::app::auth::sign_up::page)) class="button button-primary">"Sign up"</a>
                }
            </div>
        </nav>
    })
}
