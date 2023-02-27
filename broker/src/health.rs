use std::time::Duration;

use tracing::{info, warn};

#[derive(thiserror::Error, Debug, Clone, Copy)]
pub enum VaultStatus {
    #[error("ok")]
    Ok,
    #[error("unknown")]
    Unknown,
    #[error("other-error")]
    OtherError,
    #[error("locked-or-sealed")]
    LockedOrSealed,
    #[error("notreachable")]
    Unreachable,
}

impl Default for VaultStatus {
    fn default() -> Self {
        VaultStatus::Unknown
    }
}

pub struct Health {
    vault: VaultStatus
}

pub struct Senders {
    pub vault: tokio::sync::watch::Sender<VaultStatus>
}

impl Health {
    pub fn make() -> (Senders, Self) {
        let mut health = Health {
            vault: VaultStatus::default()
        };
        let (vault_tx, mut vault_rx) = tokio::sync::watch::channel(health.vault);

        let vault_watcher = async move {
            while vault_rx.changed().await.is_ok() {
                health.vault = *vault_rx.borrow();
                match health.vault {
                    VaultStatus::Ok => info!("Vault connection is now healthy"),
                    x => warn!("Vault connection is degraded: {x}"),
                }
            }
        };
        tokio::task::spawn(vault_watcher);

        let senders = Senders {
            vault: vault_tx
        };
        (senders, health)
    }
}