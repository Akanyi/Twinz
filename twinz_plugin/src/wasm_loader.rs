use crate::loader::PluginLoader;
use crate::Plugin;
use async_trait::async_trait;
use log::{error, info};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use twinz_core::{Value, ValueCodec};
use twinz_storage::BitCask;
use twinz_transport::TwinzStream;
use wasmtime::*;

pub struct WasmPluginLoader {
    engine: Engine,
    plugin_dir: std::path::PathBuf,
}

impl WasmPluginLoader {
    pub fn new<P: AsRef<std::path::Path>>(
        plugin_dir: P,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut config = Config::new();
        config.async_support(true); // 启用异步支持
        let engine = Engine::new(&config)?;
        Ok(Self {
            engine,
            plugin_dir: plugin_dir.as_ref().to_path_buf(),
        })
    }
}

impl PluginLoader for WasmPluginLoader {
    fn can_load(&self, name: &str) -> bool {
        if name.ends_with(".wasm") {
            return true;
        }
        // Check if plugin_dir/name.wasm exists
        let path = self.plugin_dir.join(format!("{}.wasm", name));
        path.exists()
    }

    fn load(
        &self,
        name: &str,
    ) -> Result<Arc<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>> {
        let module_bytes = if name.ends_with(".wasm") {
            std::fs::read(name)?
        } else {
            let path = self.plugin_dir.join(format!("{}.wasm", name));
            std::fs::read(path)?
        };
        let module = Module::new(&self.engine, &module_bytes)?;

        let plugin: Arc<dyn Plugin> = Arc::new(WasmPlugin {
            engine: self.engine.clone(),
            module,
        });
        Ok(plugin)
    }
}

struct WasmState {
    storage: Arc<BitCask>,
    wasi_ctx: wasmtime_wasi::WasiCtx,
    // 保护流的所有权，允许协议插件 "接管" (Take) 或者由 Host Functions 借用
    stream: Arc<tokio::sync::Mutex<Option<Box<dyn TwinzStream>>>>,
}

pub struct WasmPlugin {
    engine: Engine,
    module: Module,
}

#[async_trait]
impl Plugin for WasmPlugin {
    async fn handle_connection(
        &self,
        stream: Box<dyn TwinzStream>,
        storage: Arc<BitCask>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 准备 WASI 上下文
        let wasi = wasmtime_wasi::WasiCtxBuilder::new().inherit_stdio().build();

        // 包装流
        let stream_arc = Arc::new(tokio::sync::Mutex::new(Some(stream)));

        let state = WasmState {
            storage: storage.clone(),
            wasi_ctx: wasi,
            stream: stream_arc.clone(),
        };

        // 创建 Store
        let mut store = Store::new(&self.engine, state);

        // 创建 Linker 并绑定宿主函数
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::add_to_linker(&mut linker, |s: &mut WasmState| &mut s.wasi_ctx)?;

        // 宿主函数：db_put
        linker.func_wrap4_async(
            "env",
            "db_put",
            |mut caller: Caller<'_, WasmState>,
             key_ptr: i32,
             key_len: i32,
             val_ptr: i32,
             val_len: i32|
             -> Box<dyn std::future::Future<Output = i32> + Send> {
                let memory = match caller.get_export("memory") {
                    Some(Extern::Memory(mem)) => mem,
                    _ => return Box::new(async { -1 }),
                };

                // 读取 Key
                let mut key_buf = vec![0u8; key_len as usize];
                if let Err(_) = memory.read(&caller, key_ptr as usize, &mut key_buf) {
                    return Box::new(async { -2 });
                }

                // 读取 Value
                let mut val_buf = vec![0u8; val_len as usize];
                if let Err(_) = memory.read(&caller, val_ptr as usize, &mut val_buf) {
                    return Box::new(async { -3 });
                }

                let storage = caller.data().storage.clone();

                Box::new(async move {
                    match storage.put(key_buf, val_buf).await {
                        Ok(_) => 0,
                        Err(_) => -4,
                    }
                })
            },
        )?;

        // 宿主函数：console_log (调试用)
        linker.func_wrap(
            "env",
            "console_log",
            |mut caller: Caller<'_, WasmState>, ptr: i32, len: i32| {
                let memory = match caller.get_export("memory") {
                    Some(Extern::Memory(mem)) => mem,
                    _ => return,
                };
                let mut buf = vec![0u8; len as usize];
                if let Ok(_) = memory.read(&caller, ptr as usize, &mut buf) {
                    if let Ok(s) = std::str::from_utf8(&buf) {
                        info!("[WASM LOG]: {}", s);
                    }
                }
            },
        )?;

        // 宿主函数：raw_read (同步 + block_in_place)
        // env.raw_read(ptr, len) -> bytes_read
        linker.func_wrap(
            "env",
            "raw_read",
            |mut caller: Caller<'_, WasmState>, ptr: i32, len: i32| -> i32 {
                let memory = match caller.get_export("memory") {
                    Some(Extern::Memory(mem)) => mem,
                    _ => return -1,
                };

                let stream_mutex = caller.data().stream.clone();
                let buf = vec![0u8; len as usize];

                let (read_count, buf) = tokio::task::block_in_place(move || {
                    let rt = tokio::runtime::Handle::current();
                    rt.block_on(async move {
                        let mut guard = stream_mutex.lock().await;
                        let mut buf = buf;
                        let mut n = 0;
                        if let Some(stream) = guard.as_mut() {
                            if let Ok(bytes) = stream.read(&mut buf).await {
                                n = bytes;
                            }
                        }
                        (n, buf)
                    })
                });

                if read_count > 0 {
                    if let Err(_) = memory.write(&mut caller, ptr as usize, &buf[0..read_count]) {
                        return -2;
                    }
                    read_count as i32
                } else {
                    0
                }
            },
        )?;

        // 宿主函数：raw_write (同步 + block_in_place)
        // env.raw_write(ptr, len) -> bytes_written
        linker.func_wrap(
            "env",
            "raw_write",
            |mut caller: Caller<'_, WasmState>, ptr: i32, len: i32| -> i32 {
                let memory = match caller.get_export("memory") {
                    Some(Extern::Memory(mem)) => mem,
                    _ => return -1,
                };

                let mut buf = vec![0u8; len as usize];
                if let Err(_) = memory.read(&caller, ptr as usize, &mut buf) {
                    return -2;
                }

                let stream_mutex = caller.data().stream.clone();

                let written_count = tokio::task::block_in_place(move || {
                    let rt = tokio::runtime::Handle::current();
                    rt.block_on(async move {
                        let mut guard = stream_mutex.lock().await;
                        if let Some(stream) = guard.as_mut() {
                            if let Ok(_) = stream.write_all(&buf).await {
                                let _ = stream.flush().await;
                                return buf.len() as i32;
                            }
                        }
                        0
                    })
                });

                written_count
            },
        )?;

        // 实例化 Wasm 模块
        let instance = linker.instantiate_async(&mut store, &self.module).await?;

        // 混合模式检测 (Hybrid Mode)
        // 检测是否导出 `handle_raw_connection`
        if let Some(func) = instance.get_func(&mut store, "handle_raw_connection") {
            info!("检测到协议层插件 (Protocol Plugin)，移交 Raw Socket 控制权...");
            // 调用 handle_raw_connection()
            let func = func.typed::<(), ()>(&mut store)?;
            func.call_async(&mut store, ()).await?;
            info!("协议插件执行结束。");
        } else {
            // 逻辑层插件 (Logic Plugin) - 走旧的 ValueCodec 循环
            info!("检测到逻辑层插件 (Logic Plugin)，使用 ValueCodec 封装...");

            let alloc_func = instance.get_typed_func::<i32, i32>(&mut store, "alloc")?;
            let dealloc_func = instance.get_typed_func::<(i32, i32), ()>(&mut store, "dealloc")?;
            let handle_func =
                instance.get_typed_func::<(i32, i32), i64>(&mut store, "handle_command")?;
            let memory = instance
                .get_memory(&mut store, "memory")
                .ok_or("Memory not exported")?;

            loop {
                // 读取逻辑：获取锁，读 Packet，释放锁
                let pkt = {
                    let mut guard = stream_arc.lock().await;
                    match guard.as_mut() {
                        Some(s) => ValueCodec::read_value(&mut *s).await,
                        None => break,
                    }
                };

                match pkt {
                    Ok(Some(value)) => {
                        let input_bytes = serde_json::to_vec(&value)?;
                        let input_len = input_bytes.len() as i32;

                        let input_ptr = alloc_func.call_async(&mut store, input_len).await?;
                        memory.write(&mut store, input_ptr as usize, &input_bytes)?;

                        let packed_res = handle_func
                            .call_async(&mut store, (input_ptr, input_len))
                            .await?;

                        let res_ptr = (packed_res >> 32) as i32;
                        let res_len = (packed_res & 0xFFFFFFFF) as i32;

                        if res_len > 0 {
                            let mut res_buf = vec![0u8; res_len as usize];
                            memory.read(&store, res_ptr as usize, &mut res_buf)?;
                            dealloc_func
                                .call_async(&mut store, (res_ptr, res_len))
                                .await?;

                            let resp_value: Value =
                                serde_json::from_slice(&res_buf).unwrap_or(Value::Null);

                            // 写回响应
                            let mut guard = stream_arc.lock().await;
                            if let Some(s) = guard.as_mut() {
                                ValueCodec::write_value(&mut *s, &resp_value).await?;
                            }
                        } else {
                            let mut guard = stream_arc.lock().await;
                            if let Some(s) = guard.as_mut() {
                                ValueCodec::write_value(&mut *s, &Value::Null).await?;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        error!("Connection Error: {}", e);
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}
