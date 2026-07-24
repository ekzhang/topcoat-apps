#[tokio::main]
async fn main() {
    topcoat::start(topcoat_apps::router()).await.unwrap();
}
