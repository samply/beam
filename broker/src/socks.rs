use std::{collections::{HashMap, HashSet}, sync::Arc};

use fast_socks5::{server::{Socks5Server, Config, Socks5Socket, Authentication}, AuthenticationMethod};
use once_cell::sync::Lazy;
use shared::{config::CONFIG_CENTRAL, beam_id::AppId};
use tokio::{io::{AsyncWriteExt, AsyncReadExt}, net::TcpStream, sync::RwLock};
use tracing::{info, warn};
use tokio_stream::StreamExt;

/// Connections where one side already connected
/// Maps the connection secret to the assosiated socket
static WAITING_CONNECTIONS: Lazy<Arc<RwLock<HashMap<String, Socks5Socket<TcpStream>>>>> = Lazy::new(|| Arc::default());
/// Allowed tokens that are permited to create sockets
pub static ALLOWED_TOKENS: Lazy<Arc<RwLock<HashSet<String>>>> = Lazy::new(|| Arc::default());

struct Authenticator;

impl Authentication for Authenticator {
    fn authenticate(&self, username: &str, password: &str) -> bool {
        info!("{username} is authenticating a socket connection");
        ALLOWED_TOKENS.blocking_read().contains(password)
    }
}

pub async fn serve() -> anyhow::Result<()> {
    let mut addr = CONFIG_CENTRAL.bind_addr.clone();
    addr.set_port(addr.port() + 10);
    info!("Socks server running on {addr}");
    let mut server = Socks5Server::bind(addr).await?;
    let mut config = Config::default();
    config.set_execute_command(false);
    config.set_authentication(Authenticator);
    server.set_config(config);

    let mut incomng = server.incoming();
    while let Some(socket_res) = incomng.next().await {
        match socket_res {
            Ok(socket) => {
                let socket = socket.upgrade_to_socks5().await?;
                let AuthenticationMethod::Password { username, password } = socket.auth() else {
                    warn!("Someone is trying to connect without pw");
                    continue;
                };
                info!("{username} has connected to broker socket");
                let mut waiting_cons = WAITING_CONNECTIONS.write().await;
                if let Some(other) = waiting_cons.remove(password) {
                    // Send connect signal to both sockets and tokio copy bidirectional
                } else {
                    WAITING_CONNECTIONS.write().await.insert(password.clone(), socket);
                }
            },
            Err(e) => warn!("{}", e),
        }
    }
    Ok(())
}
