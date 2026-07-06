use anyhow::{Result, bail, anyhow};
use beam_lib::TaskResult;
use futures::StreamExt;
use reqwest::{header::{self, HeaderValue}, Method};
use sse_stream::SseStream;

use crate::{client1, task_test};

#[tokio::test]
async fn test_sse() -> Result<()> {
    let id = task_test::post_task("test").await?;
    let res = client1()
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
    let mut stream = SseStream::from_byte_stream(res.bytes_stream());
    assert_body(stream.next().await, "foo")?;
    assert_body(stream.next().await, "foo")?;
    assert_body(stream.next().await, "bar")?;
    assert!(matches!(stream.next().await, None), "Got more results than specified");
    Ok(())
}

fn assert_body<E>(event: Option<Result<sse_stream::Sse, E>>, expected_body: &str) -> Result<()> {
    let Ok(event) = event.ok_or(anyhow!("SSE stream ended early"))? else {
        bail!("Unexpected error parsing SSE")
    };
    let result = serde_json::from_str::<TaskResult<String>>(
        event.data.as_deref().ok_or(anyhow!("Event has no data"))?,
    )?;
    assert_eq!(result.body, expected_body);
    Ok(())
}
