use log::{error, info};
use std::sync::Arc;
use tokio::sync::RwLock;
use twinz_plugin::Plugin;
use twinz_storage::BitCask;
use twinz_transport::{TwinzAddress, TwinzStream, TwinzTransport};
pub mod codec;
pub mod types;

pub use codec::ValueCodec;
pub use types::Value;

pub struct Kernel {
    storage: Arc<BitCask>,
    // 使用 bind 返回的 Box<dyn TwinzListener>。
}

impl Kernel {
    pub fn new(storage: Arc<BitCask>) -> Self {
        Self { storage }
    }

    pub async fn run<T, P>(
        &self,
        transport: T,
        address: TwinzAddress,
        plugin: Arc<P>,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        T: TwinzTransport,
        P: Plugin,
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
