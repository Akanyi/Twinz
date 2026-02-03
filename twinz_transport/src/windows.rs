use crate::{TwinzAddress, TwinzListener, TwinzStream, TwinzTransport};
use async_trait::async_trait;
use std::io::Result;
use std::path::PathBuf;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

pub struct TwinzWindowsTransport;

pub struct TwinzWindowsListener {
    addr: PathBuf,
    first: bool,
}

#[async_trait]
impl TwinzTransport for TwinzWindowsTransport {
    type Stream = WindowsStream;

    async fn bind(
        &self,
        addr: &TwinzAddress,
    ) -> Result<Box<dyn TwinzListener<Stream = Self::Stream>>> {
        let path = addr.resolve();
        Ok(Box::new(TwinzWindowsListener {
            addr: path,
            first: true,
        }))
    }

    async fn connect(&self, addr: &TwinzAddress) -> Result<Self::Stream> {
        let path = addr.resolve();
        let client = ClientOptions::new().open(path)?;
        Ok(WindowsStream::Client(client))
    }
}

// Windows Named Pipe 的服务端 (NamedPipeServer) 和客户端 (NamedPipeClient) 是不同的类型。
// 为了统一 Stream 类型，这里使用枚举进行封装。

pub enum WindowsStream {
    Server(NamedPipeServer),
    Client(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl tokio::io::AsyncRead for WindowsStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<Result<()>> {
        match self.get_mut() {
            WindowsStream::Server(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            WindowsStream::Client(c) => std::pin::Pin::new(c).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for WindowsStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize>> {
        match self.get_mut() {
            WindowsStream::Server(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            WindowsStream::Client(c) => std::pin::Pin::new(c).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<()>> {
        match self.get_mut() {
            WindowsStream::Server(s) => std::pin::Pin::new(s).poll_flush(cx),
            WindowsStream::Client(c) => std::pin::Pin::new(c).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<()>> {
        match self.get_mut() {
            WindowsStream::Server(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            WindowsStream::Client(c) => std::pin::Pin::new(c).poll_shutdown(cx),
        }
    }
}

#[async_trait]
impl TwinzListener for TwinzWindowsListener {
    type Stream = WindowsStream;

    async fn accept(&mut self) -> Result<Self::Stream> {
        // 为每次 accept 创建一个新的管道服务端实例。
        // 在 Windows 上，创建管道，等待连接。
        // 一旦连接，将其交出，并为下一个人创建一个 NEW 管道。

        let server = ServerOptions::new()
            .first_pipe_instance(self.first)
            .create(&self.addr)?;

        if self.first {
            self.first = false;
        }

        server.connect().await?;
        Ok(WindowsStream::Server(server))
    }
}
