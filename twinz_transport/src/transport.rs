use crate::TwinzAddress;
use async_trait::async_trait;
use std::io::Result;
use tokio::io::{AsyncRead, AsyncWrite};

/// 底层流的标记 trait。
/// 必须满足 AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static (通常情况下)
pub trait TwinzStream: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static> TwinzStream for T {}

#[async_trait]
pub trait TwinzTransport: Send + Sync {
    type Stream: TwinzStream;

    /// 绑定到指定地址，返回一个监听器 (Listener)。
    /// 监听器可以调用 `accept` 来获取传入的连接流。

    async fn bind(
        &self,
        addr: &TwinzAddress,
    ) -> Result<Box<dyn TwinzListener<Stream = Self::Stream>>>;
    async fn connect(&self, addr: &TwinzAddress) -> Result<Self::Stream>;
}

#[async_trait]
pub trait TwinzListener: Send + Sync {
    type Stream: TwinzStream;
    async fn accept(&mut self) -> Result<Self::Stream>;
}
