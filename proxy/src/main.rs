mod auth;
mod reverse;
mod config;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    shared::logger::init_logger()?;
    let (config, _) = config::get_config()?;

    reverse::reverse_proxy(config).await?;
    Ok(())
}

