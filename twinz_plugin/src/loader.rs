use crate::Plugin;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PluginError {
    #[error("Plugin not found: {0}")]
    NotFound(String),
    #[error("Load error: {0}")]
    LoadError(String),
}

/// 插件加载器接口
pub trait PluginLoader: Send + Sync {
    /// 检查是否支持加载该名称
    fn can_load(&self, name: &str) -> bool;

    /// 加载并返回插件实例
    fn load(&self, name: &str)
        -> Result<Arc<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>>;
}

/// 插件管理器
pub struct PluginManager {
    loaders: Vec<Box<dyn PluginLoader>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self { loaders: vec![] }
    }

    pub fn register_loader(&mut self, loader: Box<dyn PluginLoader>) {
        self.loaders.push(loader);
    }

    pub fn get_plugin(
        &self,
        name: &str,
    ) -> Result<Arc<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>> {
        for loader in &self.loaders {
            if loader.can_load(name) {
                return loader.load(name);
            }
        }
        Err(PluginError::NotFound(name.to_string()).into())
    }
}
