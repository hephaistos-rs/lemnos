mod app;
mod components;
mod config;

#[tokio::main]
async fn main() {
    let config = config::Config::load().unwrap_or_else(|error| fail("config", &*error));
    let auth = app::auth::AuthState::from_config(config)
        .await
        .unwrap_or_else(|error| fail("sign-in setup", &*error));
    topcoat::start(app::router(auth)).await.unwrap();
}

fn fail(what: &str, error: &dyn std::error::Error) -> ! {
    eprintln!("Lemnos can't start ({what}): {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
    std::process::exit(1)
}
