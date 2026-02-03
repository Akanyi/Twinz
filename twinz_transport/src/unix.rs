use crate::{TwinzAddress, TwinzListener, TwinzStream, TwinzTransport};
use async_trait::async_trait;
use std::io::Result;
use std::path::PathBuf;
use tokio::net::{UnixListener, UnixStream};

pub struct TwinzUnixTransport;

pub struct TwinzUnixListener {
    listener: UnixListener,
}

#[async_trait]
impl TwinzTransport for TwinzUnixTransport {
    type Stream = UnixStream;

    async fn bind(
        &self,
        addr: &TwinzAddress,
    ) -> Result<Box<dyn TwinzListener<Stream = Self::Stream>>> {
        let path = addr.resolve();
        // 确保父目录存在？
        // UDS bind 通常要求文件不存在，或者我们需要 unlink 它。
        if path.exists() {
            std::fs::remove_file(&path).ok();
        }

        let listener = UnixListener::bind(path)?;
        Ok(Box::new(TwinzUnixListener { listener }))
    }

    async fn connect(&self, addr: &TwinzAddress) -> Result<Self::Stream> {
        let path = addr.resolve();
        UnixStream::connect(path).await
    }
}

#[async_trait]
impl TwinzListener for TwinzUnixListener {
    type Stream = UnixStream;

    async fn accept(&mut self) -> Result<Self::Stream> {
        let (stream, _addr) = self.listener.accept().await?;
        Ok(stream)
    }
}
