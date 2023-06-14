use shared::config::{Config, CONFIG_SHARED};


#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    proxy::main(Config::load()?, Config::load()?).await
}
