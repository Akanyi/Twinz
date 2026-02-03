use log::{error, info};
use std::sync::Arc;
use tokio::sync::RwLock;
// use twinz_plugin::Plugin; // 避免循环，删除
use twinz_storage::BitCask;
use twinz_transport::{TwinzAddress, TwinzStream, TwinzTransport};
pub mod codec;
pub mod types;

pub use codec::ValueCodec;
pub use types::Value;

// 核心 Plugin trait 定义
#[async_trait::async_trait]
pub trait Plugin: Send + Sync + 'static {
    async fn handle_connection(
        &self,
        stream: Box<dyn TwinzStream>,
        storage: Arc<BitCask>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

pub struct Kernel {
    storage: Arc<BitCask>,
    // 监听器句柄
}

impl Kernel {
    pub fn new(storage: Arc<BitCask>) -> Self {
        Self { storage }
    }

    pub async fn run<T>(
        &self,
        transport: T,
        address: TwinzAddress,
        plugin: Arc<dyn Plugin>,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        T: TwinzTransport,
    {
        info!("Binding to address: {:?}", address);
        let mut listener = transport
            .bind(&address)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

        info!("Server started, waiting for connections...");

        loop {
            match listener.accept().await {
                Ok(stream) => {
                    // 生成任务
                    let plugin = plugin.clone();
                    let storage = self.storage.clone();

                    // 将 stream 装箱以匹配 Plugin trait 签名 (Box<dyn TwinzStream>)
                    let boxed_stream: Box<dyn TwinzStream> = Box::new(stream);

                    tokio::spawn(async move {
                        if let Err(e) = plugin.handle_connection(boxed_stream, storage).await {
                            error!("Error handling connection: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("作为错误: {}", e);
                    // 继续，也可中断，这里选择继续
                }
            }
        }
    }
}
