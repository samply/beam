use std::{collections::{HashMap, HashSet}, sync::Arc, net::{SocketAddr, IpAddr, Ipv4Addr}};

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
// pub static ALLOWED_TOKENS: Lazy<Arc<RwLock<HashSet<String>>>> = Lazy::new(|| Arc::default());
#[cfg(debug_assertions)]
pub static ALLOWED_TOKENS: Lazy<Arc<RwLock<HashSet<String>>>> = Lazy::new(|| Arc::new(RwLock::new({
    let mut h = HashSet::new();
    h.insert("test".to_string());
    h
})));
#[cfg(not(debug_assertions))]
pub static ALLOWED_TOKENS: Lazy<Arc<RwLock<HashSet<String>>>> = Lazy::new(|| Arc::default());

struct Authenticator;

impl Authentication for Authenticator {
    fn authenticate(&self, _username: &str, _password: &str) -> bool {
        // We need to do auth asyncronously so we cant use this, but we still need to supply a dummy auth provider so that we have access to the auth data.
        true
    }
}

async fn authenticate(auth: &AuthenticationMethod) -> Option<(&String, &String)> {
    let AuthenticationMethod::Password { username, password } = auth else {
        warn!("Someone is trying to connect without pw");
        return None;
    };
    info!("{username} is authenticating a socket connection");
    if ALLOWED_TOKENS.read().await.contains(password) {
        Some((username, password))
    } else {
        None
    }
}

pub async fn serve() -> anyhow::Result<()> {
    let mut addr = CONFIG_CENTRAL.bind_addr.clone();
    addr.set_port(addr.port() + 10);
    info!("Socks server running on {addr}");
    let mut server = Socks5Server::bind(addr).await?;
    let mut config = Config::default();
    config.set_execute_command(false);
    config.set_dns_resolve(false);
    config.set_authentication(Authenticator);
    server.set_config(config);

    let mut incomng = server.incoming();
    while let Some(socket_res) = incomng.next().await {
        match socket_res {
            Ok(socket) => {
                let socket = socket.upgrade_to_socks5().await?;
                let auth = socket.auth();
                let Some((username, password)) = authenticate(auth).await else {
                    warn!("Failed to authenticate socket connection");
                    continue;
                };
                info!("{username} has connected to broker socket");
                let mut waiting_cons = WAITING_CONNECTIONS.write().await;
                if let Some(other) = waiting_cons.remove(password) {
                    if let Err(e) = tunnel(socket, other).await {
                        warn!("Failed to send replys to sockets. Err: {e}");
                    }
                } else {
                    waiting_cons.insert(password.clone(), socket);
                }
            },
            Err(e) => warn!("{}", e),
        }
    }
    Ok(())
}

/// Our version of the private send_reply of fast_socks https://github.com/dizda/fast-socks5/blob/master/src/server.rs#L726
async fn send_reply_successfull_connection(socket: &mut Socks5Socket<TcpStream>) -> std::io::Result<()> {
    use fast_socks5::consts;
    const REPLY: &[u8; 10] = &[
        consts::SOCKS5_VERSION,
        consts::SOCKS5_REPLY_SUCCEEDED,
        0x0, // Reserved
        consts::SOCKS5_ADDR_TYPE_IPV4,
        127, 0, 0, 1, // Ip address
        0, 0 // Port
    ];
    socket.write_all(REPLY).await?;
    socket.flush().await
}

async fn tunnel(mut a: Socks5Socket<TcpStream>, mut b: Socks5Socket<TcpStream>) -> std::io::Result<()> {
    send_reply_successfull_connection(&mut a).await?;
    send_reply_successfull_connection(&mut b).await?;
    tokio::spawn(async move {
        let result = tokio::io::copy_bidirectional(&mut a, &mut b).await;
        if let Err(e) = result {
            warn!("Error relaying socket connect: {e}");
        }
        // when we are done we remove the token from the set of allowed tokens
        let AuthenticationMethod::Password { password, .. } = a.auth() else {
            unreachable!("This is checked earlier");
        };
        if cfg!(debug_assertions) {
            info!("Would have removed token from set in prod.");
        } else {
            ALLOWED_TOKENS.write().await.remove(password);
        }
    });
    Ok(())
}
