use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{MsgState, serialize_time, MsgId, Msg, DecryptableMsg, Plain, Encrypted, EncryptableMsg, HasWaitId};
use beam_lib::AppOrProxyId;


#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MsgSocketRequest<State>
where State: MsgState {
    pub from: AppOrProxyId,
    // TODO: Tell serde to serialize only one
    pub to: Vec<AppOrProxyId>,
    #[serde(with="serialize_time", rename="ttl")]
    pub expire: SystemTime,
    pub id: MsgId,
    #[serde(skip_serializing_if = "MsgState::is_empty")]
    pub secret: State,
    #[serde(default)]
    pub metadata: Value
}

impl<State: MsgState> Msg for MsgSocketRequest<State> {
    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &self.to
    }

    fn get_metadata(&self) -> &Value {
        &self.metadata
    }
}

impl DecryptableMsg for MsgSocketRequest<Encrypted> {
    type Output = MsgSocketRequest<Plain>;

    fn get_encryption(&self) -> Option<&Encrypted> {
        Some(&self.secret)
    }

    fn convert_self(self, body: String) -> Self::Output {
        let Self { from, to, expire, id, metadata, .. } = self;
        Self::Output { from, to, expire, secret: body.into(), id, metadata }
    }
}

impl EncryptableMsg for MsgSocketRequest<Plain> {
    type Output = MsgSocketRequest<Encrypted>;

    fn convert_self(self, body: Encrypted) -> Self::Output {
        let Self { from, to, expire, id, metadata, .. } = self;
        Self::Output { from, to, expire, secret: body, id, metadata }
    }

    fn get_plain(&self) -> &Plain {
        &self.secret
    }
}

impl<State: MsgState> HasWaitId<MsgId> for MsgSocketRequest<State> {
    fn wait_id(&self) -> MsgId {
        self.id
    }
}
