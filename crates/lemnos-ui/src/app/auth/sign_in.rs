// GET /auth/sign-in

use topcoat::{
    Result,
    router::page,
    view::{View, view},
};

#[page]
pub async fn page() -> Result<impl View> {
    Ok(view! {
        <div class="max-w-md mx-auto mt-12 p-8 rounded-lg bg-secondary-700">
            <h2 class="text-2xl font-bold mb-6 text-center text-secondary-50">"Sign in"</h2>
            // POST keeps the password out of the URL. TODO: add a `#[page(POST)]` handler here.
            <form method="post" class="flex flex-col gap-y-4">
                <div class="flex flex-col gap-y-1">
                    <label for="username" class="label">"Username"</label>
                    <div class="input-wrapper">
                        <input type="text" id="username" name="username" autocomplete="username" required="">
                    </div>
                </div>
                <div class="flex flex-col gap-y-1">
                    <label for="password" class="label">"Password"</label>
                    <div class="input-wrapper">
                        <input type="password" id="password" name="password" autocomplete="current-password" required="">
                    </div>
                </div>
                <button type="submit" class="button button-primary w-full mt-2">"Sign in"</button>
            </form>
        </div>
    })
}
