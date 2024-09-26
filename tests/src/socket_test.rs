use std::{convert::Infallible, time::Duration};

use futures::{stream, StreamExt};
use anyhow::Result;
use rand::{Rng, RngCore};
use crate::*;

#[tokio::test]
async fn test_full() -> Result<()> {
    let metadata: &'static _ = Box::leak(Box::new(serde_json::json!({
        "foo": vec![1, 2, 3],
    })));
    let data = Vec::from_iter((0..1000).map(|_| {
        let mut chunk = vec![0; 1024];
        rand::thread_rng().fill_bytes(&mut chunk);
        chunk
    }));
    let stream = stream::iter(data.clone())
        .map(Ok::<_, Infallible>)
        .then(move |b| async {
            if rand::thread_rng().gen_ratio(1, 10) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            b
        });
    let app1 = async move {
        let res = tokio::time::timeout(Duration::from_secs(60), client1().create_socket_with_metadata(&APP2, stream, metadata)).await??;
        assert!(res.status().is_success());
        Ok(())
    };
    let app2 = async {
        let task = client2()
            .get_socket_tasks(&beam_lib::BlockingOptions::from_count(1))
            .await?
            .pop()
            .ok_or(anyhow::anyhow!("Failed to get a socket task"))?;
        assert_eq!(&task.metadata, metadata);
        let s = client2().connect_socket(&task.id).await?;
        let expected = data.into_iter().flatten().collect::<Vec<_>>();
        let mut buf = Vec::with_capacity(expected.len());
        s.for_each(|b| {
            buf.extend(b.unwrap());
            futures::future::ready(())
        }).await;
        assert_eq!(buf, expected);
        anyhow::Ok(())
    };

    tokio::try_join!(app1, app2)?;
    Ok(())
}
