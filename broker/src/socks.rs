use fast_socks5::server::{Socks5Server, Config};
use shared::config::CONFIG_CENTRAL;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use tracing::{info, warn};
use tokio_stream::StreamExt;

pub async fn serve() -> anyhow::Result<()> {
    let mut addr = CONFIG_CENTRAL.bind_addr.clone();
    addr.set_port(addr.port() + 10);
    info!("Socks server running on {addr}");
    let mut server = Socks5Server::bind(addr).await?;
    server.set_config(Config::default());

    let mut incomng = server.incoming();
    while let Some(socket_res) = incomng.next().await {
        match socket_res {
            Ok(socket) => {
                let mut upgraded = socket.upgrade_to_socks5().await?;
                upgraded.write(b"Hello world!").await.unwrap();
            },
            Err(e) => warn!("{}", e),
        }
    }
    Ok(())
}
