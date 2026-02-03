use std::path::PathBuf;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwinzAddress {
    /// 显式文件路径或管道名称
    /// 例如 `\\.\pipe\my_pipe` 或 `/tmp/my.sock`
    Path(PathBuf),

    /// 命名空间映射地址
    /// `twinz://name` -> `\\.\pipe\twinz.name` (Win) or `/tmp/twinz.name.sock` (Unix)
    Namespace(String),

    /// 多重 Twin 身份标识 (Multi-Twin Identity)
    /// `\\.\pipe\twinz.<name>.<id>`
    Identity { name: String, id: String },

    /// 这里的 PID 是用于私有进程间通信的
    /// `\\.\pipe\twinz.pid.<pid>`
    Pid(u32),
}

#[derive(Error, Debug)]
pub enum AddressError {
    #[error("Invalid address format")]
    InvalidFormat,
    #[error("Unknown scheme: {0}")]
    UnknownScheme(String),
}

impl TwinzAddress {
    pub fn resolve(&self) -> PathBuf {
        match self {
            TwinzAddress::Path(p) => p.clone(),
            TwinzAddress::Namespace(name) => {
                // Platform-specific resolution
                #[cfg(windows)]
                {
                    PathBuf::from(format!(r"\\.\pipe\twinz.{}", name))
                }
                #[cfg(unix)]
                {
                    PathBuf::from(format!("/tmp/twinz.{}.sock", name))
                }
                #[cfg(not(any(windows, unix)))]
                {
                    PathBuf::from(name)
                }
            }
            TwinzAddress::Identity { name, id } => {
                #[cfg(windows)]
                {
                    PathBuf::from(format!(r"\\.\pipe\twinz.{}.{}", name, id))
                }
                #[cfg(unix)]
                {
                    PathBuf::from(format!("/tmp/twinz.{}.{}.sock", name, id))
                }
                #[cfg(not(any(windows, unix)))]
                {
                    PathBuf::from(format!("{}.{}", name, id))
                }
            }
            TwinzAddress::Pid(pid) => {
                #[cfg(windows)]
                {
                    PathBuf::from(format!(r"\\.\pipe\twinz.pid.{}", pid))
                }
                #[cfg(unix)]
                {
                    PathBuf::from(format!("/tmp/twinz.pid.{}.sock", pid))
                }
                #[cfg(not(any(windows, unix)))]
                {
                    PathBuf::from(format!("pid.{}", pid))
                }
            }
        }
    }
}

impl FromStr for TwinzAddress {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("twinz://") {
            let name = s.trim_start_matches("twinz://");
            return Ok(TwinzAddress::Namespace(name.to_string()));
        }

        // 检查特定的管道模式
        if s.contains("twinz.pid.") {
            if let Some(pid_str) = s.rsplit('.').next() {
                if let Ok(pid) = pid_str.parse::<u32>() {
                    return Ok(TwinzAddress::Pid(pid));
                }
            }
        }

        // 简单解析，用户可以传递原始路径
        Ok(TwinzAddress::Path(PathBuf::from(s)))
    }
}
