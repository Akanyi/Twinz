use clap::{Parser, Subcommand, ValueEnum};
use log::{error, info};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use twinz_core::{Kernel, Value, ValueCodec};
use twinz_plugin::Plugin;
use twinz_storage::{BitCask, BitCaskOptions, SyncStrategy};
use twinz_transport::{TwinzAddress, TwinzStream, TwinzTransport};

mod config;

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
        #[arg(short, long)]
        name: Option<String>,

        /// 存储目录
        #[arg(short, long)]
        storage_dir: Option<String>,

        /// 同步模式: 'always', 'interval', 'os'
        #[arg(long, value_enum)]
        sync_mode: Option<CliSyncMode>,

        /// 同步间隔 (仅在 'interval' 模式下有效)
        #[arg(long)]
        sync_interval: Option<u64>,

        /// 插件目录
        #[arg(long)]
        plugin_dir: Option<String>,
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

// SimplePlugin removed (Moved to twinz_plugin::builtin_kv::TwinzKV)

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
            plugin_dir,
        } => {
            // 1. 加载 Config File
            let mut file_configs = config::parse_twinzfile("TwinzFile").unwrap_or_default();
            // 简单起见，我们取第一个 Server Block，或者如果没有 Block 就默认 default
            let file_config = if !file_configs.is_empty() {
                file_configs.remove(0)
            } else {
                config::ServerConfig::default()
            };

            // 2. Merge Config (CLI > File > Default)
            let final_name = name.or(Some(file_config.name)).unwrap();
            let final_storage = storage_dir
                .or(file_config.storage_dir)
                .unwrap_or("./data".to_string());
            let final_plugin_dir = plugin_dir
                .or(file_config.plugin_dir)
                .unwrap_or("./plugins".to_string());

            let final_sync_mode_str = match sync_mode {
                Some(m) => match m {
                    CliSyncMode::Always => "always".to_string(),
                    CliSyncMode::Interval => "interval".to_string(),
                    CliSyncMode::Os => "os".to_string(),
                },
                None => file_config.sync_mode.unwrap_or("os".to_string()),
            };

            let final_sync_interval = sync_interval.or(file_config.sync_interval).unwrap_or(1);

            info!("Starting Twinz Server...");
            info!("Storage: {}", final_storage);
            info!("Address: twinz://{}", final_name);
            info!("Plugin Dir: {}", final_plugin_dir);
            info!(
                "Sync Strategy: {} (Interval: {}s)",
                final_sync_mode_str, final_sync_interval
            );

            // 初始化存储
            let strategy = match final_sync_mode_str.as_str() {
                "always" => SyncStrategy::Always,
                "interval" => SyncStrategy::Interval(final_sync_interval),
                _ => SyncStrategy::None,
            };

            let options = BitCaskOptions {
                sync_strategy: strategy,
            };
            let storage = BitCask::open(&final_storage, options).await?;
            let storage = Arc::new(storage);

            // 初始化 Kernel
            let kernel = Kernel::new(storage.clone());

            // 初始化 Transport
            #[cfg(windows)]
            let transport = twinz_transport::windows::TwinzWindowsTransport;
            #[cfg(unix)]
            let transport = twinz_transport::unix::TwinzUnixTransport;

            // 地址
            let addr = TwinzAddress::Namespace(final_name);

            // 初始化 PluginManager
            let mut plugin_manager = twinz_plugin::loader::PluginManager::new();

            // 1. 注册 Wasm 加载器 (优先)
            let wasm_loader = twinz_plugin::wasm_loader::WasmPluginLoader::new(&final_plugin_dir);
            match wasm_loader {
                Ok(loader) => {
                    plugin_manager.register_loader(Box::new(loader));
                }
                Err(e) => {
                    error!("Failed to initialize WasmPluginLoader: {}", e);
                }
            }

            // 2. 注册静态加载器 (内置插件)
            let mut static_loader = twinz_plugin::static_loader::StaticPluginLoader::new();
            let static_loader = Box::new(static_loader);
            plugin_manager.register_loader(static_loader);

            // 加载默认插件
            let plugin_name = "kv";
            let plugin = plugin_manager
                .get_plugin(plugin_name)
                .expect("Failed to load default 'kv' plugin");

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

            // 地址
            let addr = TwinzAddress::Namespace(name);
            let mut stream = transport.connect(&addr).await?;
            info!("已连接！(输入 'EXIT' 或 'QUIT' 退出)");

            let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
            let mut input_buf = String::new();

            loop {
                use tokio::io::AsyncBufReadExt;
                use tokio::io::AsyncWriteExt;

                // 打印提示符 (由于 stdout 缓冲，需要 flush)
                let mut stdout = tokio::io::stdout();
                stdout.write_all(b"twinz> ").await?;
                stdout.flush().await?;

                input_buf.clear();
                let bytes_read = stdin.read_line(&mut input_buf).await?;
                if bytes_read == 0 {
                    break; // EOF
                }

                let line = input_buf.trim();
                if line.is_empty() {
                    continue;
                }

                if line.eq_ignore_ascii_case("EXIT") || line.eq_ignore_ascii_case("QUIT") {
                    println!("Bye!");
                    break;
                }

                // 简单的命令解析 (Split by whitespace)
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.is_empty() {
                    continue;
                }

                let cmd_str = parts[0].to_uppercase();
                let cmd_val = match cmd_str.as_str() {
                    "GET" if parts.len() == 2 => Value::Array(vec![
                        Value::String("GET".to_string()),
                        Value::String(parts[1].to_string()),
                    ]),
                    "SET" if parts.len() == 3 => Value::Array(vec![
                        Value::String("SET".to_string()),
                        Value::String(parts[1].to_string()),
                        Value::String(parts[2].to_string()),
                    ]),
                    "PING" => Value::String("PING".to_string()),
                    _ => {
                        // 默认为原始字符串发送，或者尝试构建 Array
                        // 如果用户输入了 "MYCMD arg1 arg2"，我们转为 ["MYCMD", "arg1", "arg2"]
                        let mut vec_cmd = Vec::new();
                        for part in parts {
                            vec_cmd.push(Value::String(part.to_string()));
                        }
                        Value::Array(vec_cmd)
                    }
                };

                // 发送请求
                if let Err(e) = ValueCodec::write_value(&mut stream, &cmd_val).await {
                    error!("发送失败: {}", e);
                    break;
                }

                // 等待响应
                match ValueCodec::read_value(&mut stream).await {
                    Ok(Some(resp)) => {
                        println!("{:?}", resp);
                    }
                    Ok(None) => {
                        info!("服务器关闭了连接");
                        break;
                    }
                    Err(e) => {
                        error!("读取响应失败: {}", e);
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
