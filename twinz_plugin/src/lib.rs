// 移除导入，因为 Plugin trait 现在从 twinz_core 再导出

pub mod builtin_kv;
pub mod loader;
pub mod static_loader;
pub mod wasm_loader;

// 再导出插件的 trait from twinz_core
pub use twinz_core::Plugin;
