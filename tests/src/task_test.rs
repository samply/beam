use std::time::Duration;

use anyhow::{Result, bail};
use beam_lib::{MsgId, TaskRequest, TaskResult, WorkStatus, BlockingOptions};
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::oneshot;

use crate::{CLIENT1, APP1, APP2, CLIENT2};

#[tokio::test]
async fn test_full_task_cycle() -> Result<()> {
    let (id_tx, id_rx) = oneshot::channel();
    let client = async {
        let id = post_task(()).await?;
        id_tx.send(id).expect("Sender dropped");
        assert_eq!(poll_result::<()>(id, &BlockingOptions::from_count(1)).await?.body, ());
        Ok(())
    };
    let server = async {
        let task = poll_task::<()>(id_rx.await?).await?;
        put_result(task.id, task.body, None).await
    };
    tokio::try_join!(client, server)?;
    Ok(())
}

#[tokio::test]
async fn test_task_claiming() -> Result<()> {
    let id = post_task(()).await?;
    put_result(id, (), Some(WorkStatus::Claimed)).await?;
    // Test waiting for 1 ready result which is not there yet
    let block = BlockingOptions::from_count(1);
    tokio::select! {
        _ = poll_result::<()>(id, &block) => {
            bail!("Got claimed result although we wanted to wait for a finished result");
        }
        _ = tokio::time::sleep(Duration::from_secs(2)) => ()
    };
    let block = BlockingOptions { wait_time: Some(Duration::from_secs(1)), wait_count: Some(1) };
    tokio::select! {
        res = poll_result::<()>(id, &block) => {
            assert_eq!(res?.status, WorkStatus::Claimed, "Workstatus did not match")
        }
        _ = tokio::time::sleep(Duration::from_secs(2)) => bail!("This took longer than 2s when it should have returned the claimed result!")
    };
    put_result(id, (), None).await?;
    assert_eq!(poll_result::<()>(id, &BlockingOptions::from_count(1)).await?.status, WorkStatus::Succeeded);
    Ok(())
}

pub async fn post_task<T: Serialize + 'static>(body: T) -> Result<MsgId> {
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

pub async fn poll_task<T: DeserializeOwned + 'static>(expected_id: MsgId) -> Result<TaskRequest<T>> {
    CLIENT2.poll_pending_tasks(&BlockingOptions::from_time(Duration::from_secs(5)))
        .await?
        .into_iter()
        .find(|t| t.id == expected_id)
        .ok_or(anyhow::anyhow!("Did not find expected task"))
}

pub async fn poll_result<T: DeserializeOwned + 'static>(task_id: MsgId, block: &BlockingOptions) -> Result<TaskResult<T>> {
    CLIENT1.poll_results(&task_id, block)
        .await?
        .pop()
        .ok_or(anyhow::anyhow!("Got no task"))
}

pub async fn put_result<T: Serialize + 'static>(task_id: MsgId, body: T, status: Option<beam_lib::WorkStatus>) -> Result<()> {
    CLIENT2.put_result(&TaskResult {
        from: APP2.clone(),
        to: vec![APP1.clone()],
        task: task_id,
        status: status.unwrap_or(beam_lib::WorkStatus::Succeeded),
        body,
        metadata: serde_json::Value::Null,
    }, &task_id).await?;
    Ok(())
}
