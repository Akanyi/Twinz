use async_trait::async_trait;
use std::sync::Arc;
use twinz_storage::BitCask;
use twinz_transport::TwinzStream;

/// Core 编译所需的最小化 Plugin trait
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// 当建立新连接时调用。
    /// 插件接管流的所有权并对其进行处理。
    /// 它还可以访问 Storage 引擎。
    async fn handle_connection(
        &self,
        stream: Box<dyn TwinzStream>,
        storage: Arc<BitCask>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
