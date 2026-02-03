use crate::loader::{PluginError, PluginLoader};
use crate::Plugin;
use async_trait::async_trait;
use log::{error, info};
use std::sync::Arc;
use twinz_core::{Value, ValueCodec};
use twinz_storage::BitCask;
use twinz_transport::TwinzStream;
use wasmtime::*;

pub struct WasmPluginLoader {
    engine: Engine,
}

impl WasmPluginLoader {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut config = Config::new();
        config.async_support(true); // 启用异步支持
        let engine = Engine::new(&config)?;
        Ok(Self { engine })
    }
}

impl PluginLoader for WasmPluginLoader {
    fn can_load(&self, name: &str) -> bool {
        name.ends_with(".wasm")
    }

    fn load(
        &self,
        name: &str,
    ) -> Result<Arc<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>> {
        let module_bytes = std::fs::read(name)?;
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
}

pub struct WasmPlugin {
    engine: Engine,
    module: Module,
}

#[async_trait]
impl Plugin for WasmPlugin {
    async fn handle_connection(
        &self,
        mut stream: Box<dyn TwinzStream>,
        storage: Arc<BitCask>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 准备 WASI 上下文
        let wasi = wasmtime_wasi::WasiCtxBuilder::new().inherit_stdio().build();

        let state = WasmState {
            storage: storage.clone(),
            wasi_ctx: wasi,
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
                // 同步读取参数到缓冲区，避免在 async 块中持有 caller 引用

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

        // 实例化 Wasm 模块
        let instance = linker.instantiate_async(&mut store, &self.module).await?;

        // 获取导出函数 (alloc, dealloc, handle_command)

        let alloc_func = instance.get_typed_func::<i32, i32>(&mut store, "alloc")?;
        let dealloc_func = instance.get_typed_func::<(i32, i32), ()>(&mut store, "dealloc")?;
        let handle_func =
            instance.get_typed_func::<(i32, i32), i64>(&mut store, "handle_command")?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or("Memory not exported")?;

        // 主循环：处理流消息

        loop {
            match ValueCodec::read_value(&mut *stream).await {
                Ok(Some(value)) => {
                    // 序列化输入值
                    let input_bytes = serde_json::to_vec(&value)?;
                    let input_len = input_bytes.len() as i32;

                    // Wasm 分配内存
                    let input_ptr = alloc_func.call_async(&mut store, input_len).await?;

                    // 写入内存
                    memory.write(&mut store, input_ptr as usize, &input_bytes)?;

                    // 调用处理函数
                    let packed_res = handle_func
                        .call_async(&mut store, (input_ptr, input_len))
                        .await?;

                    // 解包返回值 (ptr << 32 | len)
                    let res_ptr = (packed_res >> 32) as i32;
                    let res_len = (packed_res & 0xFFFFFFFF) as i32;

                    if res_len > 0 {
                        // 读取返回值
                        let mut res_buf = vec![0u8; res_len as usize];
                        memory.read(&store, res_ptr as usize, &mut res_buf)?;

                        // 释放 Wasm 内存
                        dealloc_func
                            .call_async(&mut store, (res_ptr, res_len))
                            .await?;

                        // 解析并写回响应
                        let resp_value: Value =
                            serde_json::from_slice(&res_buf).unwrap_or(Value::Null);
                        ValueCodec::write_value(&mut *stream, &resp_value).await?;
                    } else {
                        ValueCodec::write_value(&mut *stream, &Value::Null).await?;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    error!("连接错误: {}", e);

                    break;
                }
            }
        }

        Ok(())
    }
}
