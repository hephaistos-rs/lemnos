mod app;
mod components;

#[tokio::main]
async fn main() {
    topcoat::start(app::router()).await.unwrap();
}
