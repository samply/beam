use serde::{Serialize, Deserialize};

use crate::{AddressingId, MsgId};


#[derive(Debug, Serialize, Deserialize)]
pub struct SocketTask {
    pub to: Vec<AddressingId>,
    pub from: AddressingId,
    pub ttl: String,
    pub id: MsgId,
}
