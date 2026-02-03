use tokio::io::{AsyncReadExt, AsyncWriteExt};
use twinz_transport::{TwinzAddress, TwinzTransport};

#[cfg(windows)]
#[tokio::test]
async fn test_windows_named_pipe() {
    let pipe_name = "test_pipe_integration";
    let addr = TwinzAddress::Namespace(pipe_name.to_string());

    let transport = twinz_transport::windows::TwinzWindowsTransport;

    // Start Server
    let mut listener = transport.bind(&addr).await.unwrap();

    let server_handle = tokio::spawn(async move {
        let mut stream = listener.accept().await.unwrap();
        let mut buf = [0u8; 128];
        let n = stream.read(&mut buf).await.unwrap();
        stream.write_all(&buf[..n]).await.unwrap();
    });

    // Wait a bit for server to listen
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Client Connect
    let mut client = transport.connect(&addr).await.unwrap();
    client.write_all(b"hello").await.unwrap();

    let mut buf = [0u8; 128];
    let n = client.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"hello");

    server_handle.await.unwrap();
}
