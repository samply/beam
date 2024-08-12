use std::{fmt::Display, sync::Arc, time::{Duration, SystemTime}, collections::HashMap};

use serde::{Serialize, Deserialize};
use beam_lib::ProxyId;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Healthy,
    Unhealthy,
    Unknown,
}

impl Default for Verdict {
    fn default() -> Self {
        Verdict::Unknown
    }
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum VaultStatus {
    Ok,
    Unknown,
    OtherError,
    LockedOrSealed,
    Unreachable,
}

impl Default for VaultStatus {
    fn default() -> Self {
        VaultStatus::Unknown
    }
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum InitStatus {
    Unknown,
    FetchingIntermediateCert,
    Done
}

impl Default for InitStatus {
    fn default() -> Self {
        InitStatus::Unknown
    }
}

pub struct Health {
    pub vault: VaultStatus,
    pub initstatus: InitStatus,
    pub proxies: HashMap<ProxyId, ProxyStatus>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStatus {
    last_connect: SystemTime,
    last_disconnect: Option<SystemTime>,
    #[serde(skip)]
    connections: u8,
}

impl ProxyStatus {
    pub fn online(&self) -> bool {
        self.connections > 0
    }

    pub fn disconnect(&mut self) {
        self.last_disconnect = Some(SystemTime::now());
        self.connections -= 1;
    }

    pub fn connect(&mut self) {
        self.connections += 1;
        self.last_connect = SystemTime::now();
    }

    pub fn _last_seen(&self) -> SystemTime {
        if self.online() {
            SystemTime::now()
        } else {
            self.last_disconnect.expect("Should always exist as the proxy is not online")
        }
    }
}

impl ProxyStatus {
    pub fn new() -> ProxyStatus {
        ProxyStatus { last_connect: SystemTime::now(), connections: 1, last_disconnect: None }
    }
}

pub struct Senders {
    pub vault: tokio::sync::watch::Sender<VaultStatus>,
    pub init: tokio::sync::watch::Sender<InitStatus>,
}

impl Health {
    pub fn make() -> (Senders, Arc<RwLock<Self>>) {
        let health = Health {
            vault: VaultStatus::default(),
            initstatus: InitStatus::default(),
            proxies: HashMap::default()
        };
        let (vault_tx, mut vault_rx) = tokio::sync::watch::channel(VaultStatus::default());
        let (init_tx, mut init_rx) = tokio::sync::watch::channel(InitStatus::default());
        let health = Arc::new(RwLock::new(health));
        let health2 = health.clone();
        let health3 = health.clone();

        let vault_watcher = async move {
            while vault_rx.changed().await.is_ok() {
                let new_val = vault_rx.borrow().clone();
                let mut health = health2.write().await;
                health.vault = new_val;
                match &health.vault {
                    VaultStatus::Ok => info!("Vault connection is now healthy"),
                    x => warn!(
                        "Vault connection is degraded: {}",
                        serde_json::to_string(x).unwrap_or_default()
                    ),
                }
            }
        };

        tokio::task::spawn(vault_watcher);
        let initstatus_watcher = async move {
            while init_rx.changed().await.is_ok() {
                let new_val = init_rx.borrow().clone();
                let mut health = health3.write().await;
                health.initstatus = new_val;
                match &health.initstatus {
                    InitStatus::Done => {
                        info!("Initialization is now complete");
                        return;
                    },
                    x => warn!(
                        "Still initializing: {}",
                        serde_json::to_string(x).unwrap_or_default()
                    ),
                }
            }
        };
        tokio::task::spawn(initstatus_watcher);

        let senders = Senders { vault: vault_tx, init: init_tx };
        (senders, health)
    }
}
