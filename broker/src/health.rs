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

#[derive(Debug, Serialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum VaultStatus {
    Ok,
    #[default]
    Unknown,
    OtherError,
    LockedOrSealed,
    Unreachable,
}

#[derive(Debug, Serialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum InitStatus {
    #[default]
    Unknown,
    FetchingIntermediateCert,
    Done
}

#[derive(Debug, Default)]
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
