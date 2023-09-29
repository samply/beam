use std::io;

use anyhow::{Result, bail, anyhow};
use async_sse::Event;
use beam_lib::TaskResult;
use futures::{StreamExt, TryStreamExt};
use http::{Method, header, HeaderValue};

use crate::{CLIENT1, task_test};

#[tokio::test]
async fn test_sse() -> Result<()> {
    let id = task_test::post_task("test").await?;
    let res = CLIENT1
        .raw_beam_request(
            Method::GET,
            &format!("v1/tasks/{id}/results?wait_count=1"),
        )
        .header(
            header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        )
        .send()
        .await?;
    task_test::put_result(id, "foo", Some(beam_lib::WorkStatus::Claimed)).await?;
    task_test::put_result(id, "foo", Some(beam_lib::WorkStatus::Claimed)).await?;
    task_test::put_result(id, "bar", Some(beam_lib::WorkStatus::Succeeded)).await?;
    let mut stream = async_sse::decode(res.bytes_stream()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
        .into_async_read()
    );
    assert_body(stream.next().await, "foo")?;
    assert_body(stream.next().await, "foo")?;
    assert_body(stream.next().await, "bar")?;
    assert!(matches!(stream.next().await, None), "Got more results than specified");
    Ok(())
}

fn assert_body<E>(event: Option<Result<Event, E>>, expected_body: &str) -> Result<()> {
    let Ok(event) = event.ok_or(anyhow!("SSE stream ended early"))? else {
        bail!("Unexpected error parsing SSE")
    };
    let Event::Message(m) = event else {
        bail!("Event is not a message");
    };
    let result = serde_json::from_slice::<TaskResult<String>>(m.data())?;
    assert_eq!(result.body, expected_body);
    Ok(())
}
