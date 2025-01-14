use std::time::Duration;

use anyhow::{Result, bail};
use beam_lib::{MsgId, TaskRequest, TaskResult, WorkStatus, BlockingOptions};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::{client1, APP1, APP2, client2};

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
    assert!(poll_task::<()>(id).await.is_err(), "Got task although it was already claimed by us");
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

#[tokio::test]
async fn test_claim_after_success() -> Result<()> {
    // We dont want to update a successful result to claimed which is almost always a http race condition where we select on claiming and answerering a task at the same time.
    // Example:
    // We might claim a task and have not gotten a response yet so the future is still not completed and might be at some unfair proxy.
    // In parallel we are computing the result of that task and finished it so we drop the future thats waiting on the response and imidiatly send the successful result.
    // This result might end up arriving before the request that claims the task so when the claiming request arrived we should not override the result.
    let id = post_task(()).await?;
    put_result(id, (), Some(WorkStatus::Succeeded)).await?;
    put_result(id, (), Some(WorkStatus::Claimed)).await?;
    let res = tokio::time::timeout(Duration::from_secs(10), poll_result::<()>(id, &BlockingOptions::from_count(1))).await??;
    assert_eq!(res.status, WorkStatus::Succeeded);
    Ok(())
}

#[tokio::test]
async fn test_polling_tasks_yields_more_than_specified_wait_count() -> Result<()> {
    let id1 = post_task(()).await?;
    let id2 = post_task(()).await?;
    let tasks = client2().poll_pending_tasks::<Value>(&BlockingOptions::from_count(1)).await?;
    assert_eq!(tasks.iter().filter(|t| [id1, id2].contains(&t.id)).count(), 2);
    Ok(())
}

pub async fn post_task<T: Serialize + 'static>(body: T) -> Result<MsgId> {
    let id = MsgId::new();
    client1().post_task(&TaskRequest {
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
    client2().poll_pending_tasks::<Value>(&BlockingOptions::from_time(Duration::from_secs(1)))
        .await?
        .into_iter()
        .find(|t| t.id == expected_id)
        .ok_or(anyhow::anyhow!("Did not find expected task"))
        .and_then(|TaskRequest { id, from, to, body, ttl, failure_strategy, metadata }| Ok(TaskRequest {
            id, from, to, ttl, failure_strategy, metadata,
            body: serde_json::from_value(body)?
        }))
}

pub async fn poll_result<T: DeserializeOwned + 'static>(task_id: MsgId, block: &BlockingOptions) -> Result<TaskResult<T>> {
    client1().poll_results(&task_id, block)
        .await?
        .pop()
        .ok_or(anyhow::anyhow!("Got no task"))
}

pub async fn put_result<T: Serialize + 'static>(task_id: MsgId, body: T, status: Option<beam_lib::WorkStatus>) -> Result<()> {
    client2().put_result(&TaskResult {
        from: APP2.clone(),
        to: vec![APP1.clone()],
        task: task_id,
        status: status.unwrap_or(beam_lib::WorkStatus::Succeeded),
        body,
        metadata: serde_json::Value::Null,
    }, &task_id).await?;
    Ok(())
}
