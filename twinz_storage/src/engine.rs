use crate::format::{Entry, HEADER_SIZE};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Clone)]
pub struct EntryLocation {
    pub file_id: u32,
    pub offset: u64,
    pub valid_len: u32, // 相关数据段的长度 (Header + Key + Value)
}

type KeyDir = HashMap<Vec<u8>, EntryLocation>;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Format Error: {0}")]
    Format(#[from] crate::format::FormatError),
    #[error("Key not found")]
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStrategy {
    /// 每次写入都完全同步 (metadata + data) - 最安全但最慢
    Always,
    /// 定时同步 (秒) - 类似 Redis AOF everysec策略
    Interval(u64),
    /// 依赖 OS 自动刷盘 (Write back cache) - 最快但可能丢数据
    None,
}

pub struct BitCaskOptions {
    pub sync_strategy: SyncStrategy,
}

impl Default for BitCaskOptions {
    fn default() -> Self {
        Self {
            sync_strategy: SyncStrategy::None,
        }
    }
}

pub struct BitCask {
    dir: PathBuf,
    active_file: Arc<RwLock<File>>,
    active_file_id: u32,
    keydir: Arc<RwLock<KeyDir>>,
    // 写入同步策略
    options: BitCaskOptions,
    // 文件句柄池：避免频繁打开关闭文件
    readers_pool: Arc<RwLock<HashMap<u32, Arc<Mutex<File>>>>>,
}

// 简单的文件迭代器
struct BitCaskFileIterator {
    file: File,
    file_id: u32,
    offset: u64,
    len: u64,
}

impl BitCaskFileIterator {
    async fn new(path: &Path, file_id: u32) -> Result<Self, StorageError> {
        let file = File::open(path).await?;
        let len = file.metadata().await?.len();
        Ok(Self {
            file,
            file_id,
            offset: 0,
            len,
        })
    }

    async fn next_entry(
        &mut self,
    ) -> Result<Option<(EntryLocation, Vec<u8>, Vec<u8>)>, StorageError> {
        if self.offset >= self.len {
            return Ok(None);
        }

        self.file
            .seek(tokio::io::SeekFrom::Start(self.offset))
            .await?;

        let mut header_buf = vec![0u8; HEADER_SIZE];
        if self.file.read_exact(&mut header_buf).await.is_err() {
            return Ok(None);
        }

        let mut buf = std::io::Cursor::new(&header_buf);
        use bytes::Buf;
        buf.advance(4 + 8); // Skip CRC, Timestamp
        let key_size = buf.get_u32();
        let value_size = buf.get_u32();

        let entry_len = HEADER_SIZE as u64 + key_size as u64 + value_size as u64;

        let mut key = vec![0u8; key_size as usize];
        if self.file.read_exact(&mut key).await.is_err() {
            return Ok(None);
        }

        let mut value = vec![0u8; value_size as usize];
        if self.file.read_exact(&mut value).await.is_err() {
            return Ok(None);
        }

        let location = EntryLocation {
            file_id: self.file_id,
            offset: self.offset,
            valid_len: entry_len as u32,
        };

        self.offset += entry_len;
        Ok(Some((location, key, value)))
    }
}

impl BitCask {
    /// 打开或创建 BitCask 存储
    pub async fn open(
        dir: impl AsRef<Path>,
        options: BitCaskOptions,
    ) -> Result<Self, StorageError> {
        let dir = dir.as_ref();
        if !dir.exists() {
            tokio::fs::create_dir_all(dir).await?;
        }

        let mut keydir = HashMap::new();
        let mut active_file_id = 0;

        let mut files = vec![];
        let mut read_dir = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "data" {
                    if let Some(stem) = path.file_stem() {
                        if let Ok(id) = stem.to_string_lossy().parse::<u32>() {
                            // 记录文件 ID，稍后用于排序和重放
                            files.push(id);
                            if id > active_file_id {
                                active_file_id = id;
                            }
                        }
                    }
                }
            }
        }
        files.sort();

        // 重放文件
        // 重放文件
        for id in files {
            let file_path = dir.join(format!("{}.data", id));
            let mut iter = BitCaskFileIterator::new(&file_path, id).await?;

            while let Some((location, key, _value)) = iter.next_entry().await? {
                keydir.insert(key, location);
            }
        }

        let file_path = dir.join(format!("{}.data", active_file_id));

        let active_file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(file_path)
            .await?;

        let active_file = Arc::new(RwLock::new(active_file));

        // 如果是定时同步策略，启动后台任务
        if let SyncStrategy::Interval(secs) = options.sync_strategy {
            if secs > 0 {
                let target_file = active_file.clone();
                tokio::spawn(async move {
                    let mut interval =
                        tokio::time::interval(tokio::time::Duration::from_secs(secs));
                    loop {
                        interval.tick().await;
                        // 获取读锁进行 Sync
                        // 注意：sync_data/all immutable borrow is sufficient for tokio File?
                        // Tokio File sync_all takes &self.
                        let file = target_file.read().await;
                        if let Err(e) = file.sync_data().await {
                            // Log error? using println for now as we might not have log setup here securely
                            eprintln!("Background sync failed: {}", e);
                        }
                    }
                });
            }
        }

        Ok(BitCask {
            dir: dir.to_path_buf(),
            active_file: active_file,
            active_file_id,
            keydir: Arc::new(RwLock::new(keydir)),
            options,
            readers_pool: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<(), StorageError> {
        let entry = Entry::new(key.clone(), value);
        let encoded = entry.encode();
        let len = encoded.len() as u32;

        let mut file = self.active_file.write().await;
        let offset = file.seek(std::io::SeekFrom::End(0)).await?;
        file.write_all(&encoded).await?;

        match self.options.sync_strategy {
            SyncStrategy::Always => file.sync_all().await?,
            SyncStrategy::Interval(_) | SyncStrategy::None => {}
        }

        let location = EntryLocation {
            file_id: self.active_file_id,
            offset,
            valid_len: len,
        };

        let mut keydir = self.keydir.write().await;
        keydir.insert(key, location);

        Ok(())
    }

    pub async fn get(&self, key: &[u8]) -> Result<Vec<u8>, StorageError> {
        let keydir = self.keydir.read().await;
        let location = keydir.get(key).ok_or(StorageError::NotFound)?;

        // 尝试从池中获取读取器
        let reader = {
            let pool = self.readers_pool.read().await;
            pool.get(&location.file_id).cloned()
        };

        let reader = match reader {
            Some(r) => r,
            None => {
                // Cache Miss: 打开并插入
                // 注意：这里可能存在竞争，多个读取者可能同时打开同一个文件
                // 但这对正确性没有影响，只是多打开了几次，最终Map只会存一个
                let file_path = self.dir.join(format!("{}.data", location.file_id));
                let file = File::open(file_path).await?;
                let reader = Arc::new(Mutex::new(file));

                let mut pool = self.readers_pool.write().await;
                pool.insert(location.file_id, reader.clone());
                reader
            }
        };

        // 锁定文件进行 Seek + Read
        // 这确保了对同一个文件句柄的操作是原子的
        let mut file = reader.lock().await;
        file.seek(std::io::SeekFrom::Start(location.offset)).await?;

        // 读取 Header
        let mut header_buf = vec![0u8; HEADER_SIZE];
        file.read_exact(&mut header_buf).await?;

        let mut buf = std::io::Cursor::new(&header_buf);
        use bytes::Buf;
        buf.advance(4); // Skip CRC
        buf.advance(8); // Timestamp
        let key_size = buf.get_u32();
        let value_size = buf.get_u32(); // u32

        // 读取 KV
        let mut kv_buf = vec![0u8; (key_size + value_size) as usize];
        file.read_exact(&mut kv_buf).await?;

        let value = kv_buf[key_size as usize..].to_vec();
        Ok(value)
    }
    /// 执行 Compaction 操作
    /// 找出所有旧文件
    /// 迭代读取所有条目
    /// 如果条目是最新的 (exists in KeyDir same location)，写入新的 merge 文件
    /// 更新 KeyDir 指向新位置
    /// 删除旧文件
    pub async fn compact(&self) -> Result<(), StorageError> {
        // 找出需要 Compact 的文件 (除了当前活跃文件)
        let files_to_compact: Vec<u32> = {
            let mut files = vec![];
            let mut read_dir = tokio::fs::read_dir(&self.dir).await?;
            while let Some(entry) = read_dir.next_entry().await? {
                let path = entry.path();
                if let Some(ext) = path.extension() {
                    if ext == "data" {
                        if let Some(stem) = path.file_stem() {
                            if let Ok(id) = stem.to_string_lossy().parse::<u32>() {
                                // 不 Compact 当前活跃文件
                                if id != self.active_file_id {
                                    files.push(id);
                                }
                            }
                        }
                    }
                }
            }
            files.sort();
            files
        };

        if files_to_compact.is_empty() {
            return Ok(());
        }

        for id in files_to_compact {
            let file_path = self.dir.join(format!("{}.data", id));
            let mut iter = BitCaskFileIterator::new(&file_path, id).await?;

            while let Some((location, key, value)) = iter.next_entry().await? {
                // 检查有效性
                let is_valid = {
                    let keydir = self.keydir.read().await;
                    match keydir.get(&key) {
                        Some(curr_loc) => {
                            // 只有当索引中的位置完全等于当前读取的位置时，才说明这是有效数据
                            curr_loc.file_id == location.file_id
                                && curr_loc.offset == location.offset
                        }
                        None => false,
                    }
                };

                if is_valid {
                    // 重新写入 (Compaction)
                    // put 会自动追加到 active_file 并更新 keydir
                    self.put(key, value).await?;
                }
            }

            // 文件压缩完成。删除它。
            // 先从 readers pool 中移除
            {
                let mut pool = self.readers_pool.write().await;
                pool.remove(&id);
            }
            tokio::fs::remove_file(file_path).await?;
        }

        Ok(())
    }
}
