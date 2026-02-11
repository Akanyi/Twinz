use anyhow::{anyhow, Result};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub name: String,
    pub storage_dir: Option<String>,
    pub sync_mode: Option<String>, // "always", "interval", "os"
    pub sync_interval: Option<u64>,
    pub plugin_dir: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "twinz_default".to_string(),
            storage_dir: None,
            sync_mode: None,
            sync_interval: None,
            plugin_dir: None,
        }
    }
}

pub fn parse_twinzfile<P: AsRef<Path>>(path: P) -> Result<Vec<ServerConfig>> {
    let content = fs::read_to_string(path)?;
    let mut tokens = content.split_whitespace().peekable();
    let mut configs = Vec::new();

    while let Some(token) = tokens.next() {
        // Assume start of a server block: "name {"
        let name = token.to_string();

        let brace = tokens
            .next()
            .ok_or_else(|| anyhow!("Expected '{{' after server name"))?;
        if brace != "{" {
            return Err(anyhow!(
                "Expected '{{' after server name, found '{}'",
                brace
            ));
        }

        let mut config = ServerConfig {
            name,
            ..Default::default()
        };

        // Parse block content
        loop {
            let directive = match tokens.next() {
                Some("}") => break, // End of block
                Some(d) => d,
                None => return Err(anyhow!("Unexpected end of file, missing '}}'")),
            };

            match directive {
                "storage" => {
                    let val = tokens
                        .next()
                        .ok_or_else(|| anyhow!("Expected value for 'storage'"))?;
                    // Remove quotes if present
                    config.storage_dir = Some(val.trim_matches('"').to_string());
                }
                "sync" => {
                    let mode = tokens
                        .next()
                        .ok_or_else(|| anyhow!("Expected mode for 'sync'"))?;
                    config.sync_mode = Some(mode.to_string());

                    // Optional interval for "interval" mode
                    if mode == "interval" {
                        if let Some(next_token) = tokens.peek() {
                            // Simple check if next token is a number
                            if let Ok(interval) = next_token.parse::<u64>() {
                                config.sync_interval = Some(interval);
                                tokens.next(); // consume it
                            }
                        }
                    }
                }
                "plugin_dir" => {
                    let val = tokens
                        .next()
                        .ok_or_else(|| anyhow!("Expected value for 'plugin_dir'"))?;
                    config.plugin_dir = Some(val.trim_matches('"').to_string());
                }
                _ => {
                    return Err(anyhow!("Unknown directive: {}", directive));
                }
            }
        }
        configs.push(config);
    }

    Ok(configs)
}
