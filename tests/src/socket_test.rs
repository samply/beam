use std::time::Duration;

use beam_lib::{BlockingOptions, MsgId};
use rand::RngCore;
use tokio::io::{AsyncWriteExt, AsyncReadExt, AsyncRead, AsyncWrite};
use anyhow::Result;
use crate::*;

async fn test_connection<T: AsyncRead + AsyncWrite + Unpin>(mut a: T, mut b: T) -> Result<()> {
    const N: usize = 2_usize.pow(8);
    for _ in 0..10 {
        let test_data: &mut [u8; N] = &mut [0; N];
        rand::thread_rng().fill_bytes(test_data);
        let mut read_buf = [0; N];
        a.write_all(test_data).await?;
        a.flush().await?;
        b.read_exact(&mut read_buf).await?;
        assert_eq!(test_data, &read_buf);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(())
}

#[tokio::test]
async fn test_full() -> Result<()> {
    let id = MsgId::new();
    let id_str = id.to_string();
    let metadata = serde_json::json!({
        "foo": vec![1, 2, 3],
        "id": id
    });
    let app1 = async {
        CLIENT1.create_socket_with_metadata(&APP2, &metadata).await.map_err(anyhow::Error::from)
    };
    let app2 = async {
        let task = CLIENT2
            .get_socket_tasks(&BlockingOptions::from_time(Duration::from_secs(1)))
            .await?
            .into_iter()
            .find(|t| t.metadata["id"].as_str() == Some(&id_str))
            .ok_or(anyhow::anyhow!("Failed to get a socket task"))?;
        assert_eq!(&task.metadata, &metadata);
        Ok(CLIENT2.connect_socket(&task.id).await?)
    };

    let (app1, app2) = tokio::try_join!(app1, app2)?;
    test_connection(app1, app2).await
}
