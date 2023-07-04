use std::{fmt::Display, sync::Arc, time::{Duration, SystemTime}, collections::HashMap};

use serde::{Serialize, Deserialize};
use shared::beam_id::ProxyId;
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

#[derive(Serialize, Clone)]
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

pub struct Health {
    pub vault: VaultStatus,
    pub proxies: HashMap<ProxyId, ProxyStatus>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStatus {
    last_active: SystemTime,
    #[serde(skip)]
    connections: u8,
}

impl ProxyStatus {
    pub fn online(&self) -> bool {
        self.connections > 0
    }

    pub fn disconnect(&mut self) {
        self.connections -= 1;
    }

    pub fn connect(&mut self) {
        self.connections += 1;
        self.last_active = SystemTime::now();
    }
}

impl ProxyStatus {
    pub fn new() -> ProxyStatus {
        ProxyStatus { last_active: SystemTime::now(), connections: 1 }
    }
}

pub struct Senders {
    pub vault: tokio::sync::watch::Sender<VaultStatus>,
}

impl Health {
    pub fn make() -> (Senders, Arc<RwLock<Self>>) {
        let health = Health {
            vault: VaultStatus::default(),
            proxies: HashMap::default()
        };
        let (vault_tx, mut vault_rx) = tokio::sync::watch::channel(VaultStatus::default());
        let health = Arc::new(RwLock::new(health));
        let health2 = health.clone();

        let vault_watcher = async move {
            while vault_rx.changed().await.is_ok() {
                let new_val = vault_rx.borrow().clone();
                let mut health = health2.write().await;
                match &new_val {
                    VaultStatus::Ok => info!("Vault connection is now healthy"),
                    x => warn!(
                        "Vault connection is degraded: {}",
                        serde_json::to_string(x).unwrap_or_default()
                    ),
                }
                health.vault = new_val;
            }
        };
        tokio::task::spawn(vault_watcher);

        let senders = Senders { vault: vault_tx };
        (senders, health)
    }
}
