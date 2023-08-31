use anyhow::Result;
use beam_lib::{MsgId, TaskRequest, TaskResult};
use serde::{de::DeserializeOwned, Serialize};

use crate::{CLIENT1, APP1, APP2, CLIENT2};

#[tokio::test]
async fn test_full_task_cycle() -> Result<()> {
    let client = async {
        let id = post_task(()).await?;
        assert_eq!(poll_result::<()>(id).await?.body, ());
        Ok(())
    };
    let server = async {
        let task = poll_task::<()>().await?;
        put_result(task.id, task.body).await
    };
    tokio::try_join!(client, server)?;
    Ok(())
}

pub async fn post_task<T: Serialize>(body: T) -> Result<MsgId> {
    let id = MsgId::new();
    CLIENT1.post_task(&TaskRequest {
        id,
        from: APP1.clone(),
        to: vec![APP2.clone()],
        body,
        ttl: "10s".to_string(),
        failure_strategy: beam_lib::FailureStrategy::Discard,
        metadata: serde_json::Value::Null,
    }).await?;
    Ok(id)
}

pub async fn poll_task<T: DeserializeOwned>() -> Result<TaskRequest<T>> {
    CLIENT2.poll_pending_tasks(&beam_lib::BlockingOptions::from_count(1))
        .await?
        .pop()
        .ok_or(anyhow::anyhow!("Got no task"))
}

pub async fn poll_result<T: DeserializeOwned>(task_id: MsgId) -> Result<TaskResult<T>> {
    CLIENT1.poll_results(&task_id, &beam_lib::BlockingOptions::from_count(1))
        .await?
        .pop()
        .ok_or(anyhow::anyhow!("Got no task"))
}

pub async fn put_result<T: Serialize>(task_id: MsgId, body: T) -> Result<()> {
    CLIENT2.put_result(&TaskResult {
        from: APP2.clone(),
        to: vec![APP1.clone()],
        task: task_id,
        status: beam_lib::WorkStatus::Succeeded,
        body,
        metadata: serde_json::Value::Null,
    }, &task_id).await?;
    Ok(())
}
