use std::collections::HashMap;
use dataobjects::{beam_id::AppOrProxyId, MsgTaskRequest, MsgTaskResult};
use shared::MsgSigned;

struct RequestResultContainer{
    request: MsgTaskRequest,
    result: HashMap<AppOrProxyId, MsgSigned<MsgTaskResult>>
}
