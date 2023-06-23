use serde::{Serialize, Deserialize};

use crate::{ids::AppOrProxyId, messages::MsgId};


#[derive(Debug, Serialize, Deserialize)]
pub struct SocketTask {
    pub to: Vec<AppOrProxyId>,
    pub from: AppOrProxyId,
    pub ttl: String,
    pub id: MsgId,
}
