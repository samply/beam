use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, SystemTimeError},
};

use shared::{EncryptedMsgTaskRequest, MsgId, MsgSigned, MyUuid};
use tokio::{
    select,
    sync::{broadcast::Receiver, RwLock, RwLockReadGuard},
};
use tracing::{debug, error, info, warn};

struct Latest {
    id: Option<MsgId>,
    expire: Option<SystemTime>,
}

async fn get_soonest(tasks: &HashMap<MyUuid, MsgSigned<EncryptedMsgTaskRequest>>) -> Latest {
    match get_shortest(tasks) {
        Some(x) => Latest {
            id: Some(x.msg.id),
            expire: Some(x.msg.expire),
        },
        None => Latest {
            id: None,
            expire: None,
        },
    }
}

pub(crate) async fn watch(
    tasks: Arc<RwLock<HashMap<MyUuid, MsgSigned<EncryptedMsgTaskRequest>>>>,
    mut new_task_rx: Receiver<MsgSigned<EncryptedMsgTaskRequest>>,
) -> Result<(), SystemTimeError> {
    let mut soonest = {
        let tasks = tasks.read().await;
        get_soonest(&tasks).await
    };
    loop {
        let until = match &soonest.expire {
            Some(soonest) => match soonest.duration_since(SystemTime::now()) {
                Ok(x) => x,
                Err(expired_since) => {
                    debug!(
                        "Tried to wait on a task that had in fact expired since {}.",
                        expired_since
                    );
                    Duration::from_secs(0)
                }
            },
            None => Duration::MAX,
        };
        debug!(
            "Next task {} will expire in {} seconds",
            {
                if soonest.id.is_some() {
                    soonest.id.unwrap().to_string()
                } else {
                    "(none)".to_string()
                }
            },
            until.as_secs()
        );
        select! {
            // New Task created => check if it will expire sooner than all the other ones
            Ok(new) = new_task_rx.recv() => {
                if let Some(expire) = soonest.expire {
                    if new.msg.expire < expire {
                        soonest.id = Some(new.msg.id);
                        soonest.expire = Some(new.msg.expire);
                    }
                } else {
                    soonest.id = Some(new.msg.id);
                    soonest.expire = Some(new.msg.expire);
                }
            },
            // Timer met (=> task has expired)
            _ = tokio::time::sleep(until) => {
                let mut tasks = tasks.write().await;
                let removed = tasks.remove(&soonest.id.unwrap());
                if let Some(removed) = removed {
                    info!("Removed expired task {}.", removed.msg.id);
                } else {
                    warn!("Tried to remove expired task {} but it was already gone.", soonest.id.unwrap());
                }
                // Now that we removed the previously-soonest task, what is the next one?
                soonest = get_soonest(&tasks).await;
            }
        }
    }
}

fn get_shortest(
    tasks: &HashMap<MyUuid, MsgSigned<EncryptedMsgTaskRequest>>,
) -> Option<&MsgSigned<EncryptedMsgTaskRequest>> {
    let mut shortest = tasks.values().next()?;
    for task in tasks.values() {
        if task.msg.expire < shortest.msg.expire {
            shortest = task;
        }
    }
    Some(shortest)
}
