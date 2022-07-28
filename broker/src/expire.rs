use std::{sync::Arc, collections::HashMap, time::{SystemTime, Duration, SystemTimeError}};

use shared::{MyUuid, MsgSigned, MsgTaskRequest, MsgId};
use tokio::{sync::{RwLock, broadcast::Receiver, RwLockReadGuard}, select};
use tracing::{debug, warn, info};

struct Latest {
    id: Option<MsgId>,
    expire: Option<SystemTime>
}

pub(crate) async fn watch(tasks: Arc<RwLock<HashMap<MyUuid, MsgSigned<MsgTaskRequest>>>>, mut new_task_rx: Receiver<MsgSigned<MsgTaskRequest>>) -> Result<(), SystemTimeError> {
    let mut soonest = {
        let tasks = tasks.read().await;
        match get_shortest(&tasks) {
            Some(x) => Latest { id: Some(x.msg.id), expire: Some(x.msg.expire) },
            None => Latest { id: None, expire: None },
        }
    };
    loop {
        let until = match &soonest.expire {
            Some(soonest) => soonest.duration_since(SystemTime::now())?,
            None => Duration::MAX,
        };
        select! {
            // New Task created => check if it will expire sooner than all the other ones
            Ok(new) = new_task_rx.recv() => {
                if let Some(expire) = soonest.expire {
                    if new.msg.expire < expire {
                        soonest.id = Some(new.msg.id);
                        soonest.expire = Some(new.msg.expire);
                        debug!("Next task will expire at {:?}", new.msg.expire);
                    }
                } else {
                    soonest.id = Some(new.msg.id);
                    soonest.expire = Some(new.msg.expire);
                    debug!("Next task will expire at {:?}", new.msg.expire);
                }
            },
            // Timer meet (=> task has expired)
            _ = tokio::time::sleep(until) => {
                let mut tasks = tasks.write().await;
                let removed = tasks.remove(&soonest.id.unwrap());
                if let Some(removed) = removed {
                    info!("Removed expired task {}.", removed.msg.id);
                } else {
                    warn!("Tried to remove expired task {} but it was already gone.", soonest.id.unwrap());
                }
            }
        }
    }
}

fn get_shortest<'a>(tasks: &'a RwLockReadGuard<HashMap<MyUuid, MsgSigned<MsgTaskRequest>>>) -> Option<&'a MsgSigned<MsgTaskRequest>> {
    let mut shortest = tasks.values().next()?;
    for task in tasks.values() {
        if task.msg.expire < shortest.msg.expire {
            shortest = task;
        }
    }
    Some(shortest)
}