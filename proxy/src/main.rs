use shared::{config::{CONFIG_PROXY, Config}};


#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    proxy::main(Config::load()?).await
}
