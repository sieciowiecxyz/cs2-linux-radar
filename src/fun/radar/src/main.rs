mod app;
mod map_registry;
mod model;
mod reader;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let _ = shared_logging::init("info");
    app::run().await
}
