use clap::{Parser, Subcommand, ValueEnum};
use log::{error, info};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use twinz_core::{Kernel, Value, ValueCodec};
use twinz_plugin::Plugin;
use twinz_storage::{BitCask, BitCaskOptions, SyncStrategy};
use twinz_transport::{TwinzAddress, TwinzStream, TwinzTransport};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliSyncMode {
    Always,
    Interval,
    Os,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 启动 Twinz 服务器
    Server {
        /// 管道名称或地址
        #[arg(short, long, default_value = "twinz_default")]
        name: String,

        /// 存储目录
        #[arg(short, long, default_value = "./data")]
        storage_dir: String,

        /// 同步模式: 'always', 'interval', 'os'
        #[arg(long, value_enum, default_value_t = CliSyncMode::Os)]
        sync_mode: CliSyncMode,

        /// 同步间隔 (仅在 'interval' 模式下有效)
        #[arg(long, default_value_t = 1)]
        sync_interval: u64,
    },
    /// 压缩存储 (合并旧文件)
    Compact {
        /// 存储目录
        #[arg(short, long, default_value = "./data")]
        storage_dir: String,
    },
    /// 作为客户端连接
    Client {
        /// 要连接的管道名称或地址
        #[arg(short, long, default_value = "twinz_default")]
        name: String,
    },
}

pub struct SimplePlugin;

#[async_trait::async_trait]
impl Plugin for SimplePlugin {
    async fn handle_connection(
        &self,
        mut stream: Box<dyn TwinzStream>,
        storage: Arc<BitCask>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 使用 Duck Typing (Value) 处理
        // 示例: 客户端发送 "SET", "key", "value" (在数组中)
        // 使用 Values 实现一个简单的 Echo/Store 循环

        loop {
            // 读取 Value
            match ValueCodec::read_value(&mut *stream).await {
                Ok(Some(value)) => {
                    info!("Received Duck Value: {:?}", value);

                    // 如果数组 ["SET", k, v] -> 存储
                    // 如果数组 ["GET", k] -> 获取

                    match value {
                        Value::Array(ref args) => {
                            if args.len() >= 3 && args[0].as_str() == Some("SET") {
                                let key = args[1].to_string_lossy().as_bytes().to_vec();
                                // 存储 args[2] 的序列化
                                let val_bytes = serde_json::to_vec(&args[2])?;
                                storage.put(key, val_bytes).await?;
                                ValueCodec::write_value(
                                    &mut *stream,
                                    &Value::String("OK".to_string()),
                                )
                                .await?;
                            } else if args.len() >= 2 && args[0].as_str() == Some("GET") {
                                let key = args[1].to_string_lossy().as_bytes().to_vec();
                                match storage.get(&key).await {
                                    Ok(val_bytes) => {
                                        // 尝试反序列化回 Value，否则返回 Bytes
                                        let val: Value = serde_json::from_slice(&val_bytes)
                                            .unwrap_or(Value::Bytes(val_bytes));
                                        ValueCodec::write_value(&mut *stream, &val).await?;
                                    }
                                    Err(_) => {
                                        ValueCodec::write_value(&mut *stream, &Value::Null).await?;
                                    }
                                }
                            } else {
                                // 回显
                                ValueCodec::write_value(&mut *stream, &value).await?;
                            }
                        }
                        _ => {
                            // 回显
                            ValueCodec::write_value(&mut *stream, &value).await?;
                        }
                    }
                }
                Ok(None) => break, // EOF
                Err(e) => {
                    error!("Connection error: {}", e);
                    break;
                }
            }
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 默认开启 info 级别日志，除非环境变量 RUST_LOG 另有指定
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    env_logger::init();
    let args = Cli::parse();

    match args.command {
        Commands::Server {
            name,
            storage_dir,
            sync_mode,
            sync_interval,
        } => {
            info!("Starting Twinz Server...");
            info!("Storage: {}", storage_dir);
            info!("Address: twinz://{}", name);
            info!(
                "Sync Strategy: {:?} (Interval: {}s)",
                sync_mode, sync_interval
            );

            // 初始化存储
            let strategy = match sync_mode {
                CliSyncMode::Always => SyncStrategy::Always,
                CliSyncMode::Interval => SyncStrategy::Interval(sync_interval),
                CliSyncMode::Os => SyncStrategy::None,
            };

            let options = BitCaskOptions {
                sync_strategy: strategy,
            };
            let storage = BitCask::open(&storage_dir, options).await?;
            let storage = Arc::new(storage);

            // 初始化 Kernel
            let kernel = Kernel::new(storage.clone());

            // 初始化 Transport
            #[cfg(windows)]
            let transport = twinz_transport::windows::TwinzWindowsTransport;
            #[cfg(unix)]
            let transport = twinz_transport::unix::TwinzUnixTransport;

            // 地址
            let addr = TwinzAddress::Namespace(name);

            // 初始化 Plugin (默认)
            let plugin = Arc::new(SimplePlugin);

            // 运行
            kernel.run(transport, addr, plugin).await?;
        }
        Commands::Compact { storage_dir } => {
            info!("Starting Compaction...");
            info!("Storage: {}", storage_dir);
            // 默认选项 (OS 同步即可，我们只加载和重写)
            let options = BitCaskOptions::default();
            let storage = BitCask::open(&storage_dir, options).await?;
            storage.compact().await?;
            info!("Compaction Completed Successfully.");
        }
        Commands::Client { name } => {
            info!("正在连接到 Twinz Server: twinz://{}...", name);
            // 初始化 Transport
            #[cfg(windows)]
            let transport = twinz_transport::windows::TwinzWindowsTransport;
            #[cfg(unix)]
            let transport = twinz_transport::unix::TwinzUnixTransport;
            // let transport = TwinzTransport::default_windows(); // Error
            let addr = TwinzAddress::Namespace(name);

            let mut stream = transport.connect(&addr).await?;
            info!("已连接！发送 Duck Types...");

            // Test 1: Echo String
            info!("Sending String...");
            ValueCodec::write_value(&mut stream, &Value::String("Hello Duck".to_string()))
                .await
                .map_err(|e| {
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    )) as Box<dyn std::error::Error>
                })?;
            if let Some(resp) = ValueCodec::read_value(&mut stream).await.map_err(|e| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )) as Box<dyn std::error::Error>
            })? {
                info!("Response: {:?}", resp);
            }

            // Test 2: SET ["SET", "foo", "bar"]
            info!("Sending SET command...");
            let cmd = Value::Array(vec![
                Value::String("SET".to_string()),
                Value::String("foo".to_string()),
                Value::String("bar".to_string()),
            ]);
            ValueCodec::write_value(&mut stream, &cmd)
                .await
                .map_err(|e| {
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    )) as Box<dyn std::error::Error>
                })?;
            if let Some(resp) = ValueCodec::read_value(&mut stream).await.map_err(|e| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )) as Box<dyn std::error::Error>
            })? {
                info!("Response: {:?}", resp);
            }

            // Test 3: GET ["GET", "foo"]
            info!("Sending GET command...");
            let cmd = Value::Array(vec![
                Value::String("GET".to_string()),
                Value::String("foo".to_string()),
            ]);
            ValueCodec::write_value(&mut stream, &cmd)
                .await
                .map_err(|e| {
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    )) as Box<dyn std::error::Error>
                })?;

            if let Some(resp) = ValueCodec::read_value(&mut stream).await.map_err(|e| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )) as Box<dyn std::error::Error>
            })? {
                info!("Response: {:?}", resp);
            }
        }
    }

    Ok(())
}
