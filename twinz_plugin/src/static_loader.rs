use crate::builtin_kv::TwinzKV;
use crate::loader::{PluginError, PluginLoader};
use crate::Plugin;
use std::collections::HashMap;
use std::sync::Arc;

pub struct StaticPluginLoader {
    plugins: HashMap<String, Arc<dyn Plugin>>,
}

impl StaticPluginLoader {
    pub fn new() -> Self {
        let mut plugins: HashMap<String, Arc<dyn Plugin>> = HashMap::new();

        #[cfg(feature = "builtin-kv")]
        {
            plugins.insert("kv".to_string(), Arc::new(TwinzKV));
            // 默认
            plugins.insert("simple".to_string(), Arc::new(TwinzKV));
        }

        Self { plugins }
    }

    pub fn register(&mut self, name: &str, plugin: Arc<dyn Plugin>) {
        self.plugins.insert(name.to_string(), plugin);
    }
}

impl PluginLoader for StaticPluginLoader {
    fn can_load(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    fn load(
        &self,
        name: &str,
    ) -> Result<Arc<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>> {
        self.plugins
            .get(name)
            .cloned()
            .ok_or_else(|| PluginError::NotFound(name.to_string()).into())
    }
}
