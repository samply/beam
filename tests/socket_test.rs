
use fast_socks5::client::{Config, Socks5Stream};
use tokio::io::{AsyncWriteExt, AsyncReadExt};

#[tokio::test]
async fn test_sockets() {
    let (a, b) = tokio::join!(
        Socks5Stream::connect_with_password("localhost:8090", "We dont care".to_string(), 1337, "test1".to_string(), "test".to_string(), Config::default()),
        Socks5Stream::connect_with_password("localhost:8090", "We dont care".to_string(), 1337, "test2".to_string(), "test".to_string(), Config::default())
    );
    let mut a = a.unwrap();
    let mut b = b.unwrap();
    const TEST_DATA: &[u8; 11] = b"hello world";
    a.write_all(TEST_DATA).await.unwrap();
    a.flush().await.unwrap();
    let recieved = &mut [0 as u8; TEST_DATA.len()];
    b.read_exact(recieved).await.unwrap();
    assert_eq!(TEST_DATA, recieved);
}
