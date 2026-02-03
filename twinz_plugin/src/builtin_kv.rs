#[cfg(feature = "builtin-kv")]
use crate::Plugin;
#[cfg(feature = "builtin-kv")]
use async_trait::async_trait;
#[cfg(feature = "builtin-kv")]
use log::{error, info};
#[cfg(feature = "builtin-kv")]
use std::sync::Arc;
#[cfg(feature = "builtin-kv")]
use twinz_core::{Value, ValueCodec};
#[cfg(feature = "builtin-kv")]
use twinz_storage::BitCask;
#[cfg(feature = "builtin-kv")]
use twinz_transport::TwinzStream;

#[cfg(feature = "builtin-kv")]
pub struct TwinzKV;

#[cfg(feature = "builtin-kv")]
#[async_trait]
impl Plugin for TwinzKV {
    async fn handle_connection(
        &self,
        mut stream: Box<dyn TwinzStream>,
        storage: Arc<BitCask>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        loop {
            // 读取 Value
            match ValueCodec::read_value(&mut *stream).await {
                Ok(Some(value)) => {
                    info!("TwinzKV Received: {:?}", value);

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
