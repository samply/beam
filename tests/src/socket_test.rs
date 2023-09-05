use rand::RngCore;
use tokio::io::{AsyncWriteExt, AsyncReadExt, AsyncRead, AsyncWrite};
use anyhow::Result;
use crate::*;

async fn test_connection<T: AsyncRead + AsyncWrite + Unpin>(mut a: T, mut b: T) -> Result<()> {
    const N: usize = 2_usize.pow(13);
    let test_data: &mut [u8; N] = &mut [0; N];
    rand::thread_rng().fill_bytes(test_data);
    let mut read_buf = [0; N];
    a.write_all(test_data).await?;
    a.flush().await?;
    b.read_exact(&mut read_buf).await?;
    assert_eq!(test_data, &read_buf);
    Ok(())
}

#[tokio::test]
async fn test_full() -> Result<()> {
    let app1 = async {
        CLIENT1.create_socket(&APP2).await.map_err(anyhow::Error::from)
    };
    let app2 = async {
        let id = CLIENT2
            .get_socket_tasks(&beam_lib::BlockingOptions::from_count(1))
            .await?
            .pop()
            .ok_or(anyhow::anyhow!("Failed to get a socket task"))?
            .id;
        Ok(CLIENT2.connect_socket(&id).await?)
    };

    let (app1, app2) = tokio::try_join!(app1, app2)?;
    test_connection(app1, app2).await
}
