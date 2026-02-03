use bytes::{Buf, BufMut, Bytes, BytesMut};
use crc32fast::Hasher;
use thiserror::Error;

pub const HEADER_SIZE: usize = 4 + 8 + 4 + 4; // CRC(4) + TS(8) + KSz(4) + VSz(4)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryHeader {
    pub crc: u32,
    pub timestamp: u64,
    pub key_size: u32,
    pub value_size: u32,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub header: EntryHeader,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Error, Debug)]
pub enum FormatError {
    #[error("Incomplete header")]
    IncompleteHeader,
    #[error("CRC mismatch: expected {expected}, got {actual}")]
    CrcMismatch { expected: u32, actual: u32 },
}

impl Entry {
    pub fn new(key: Vec<u8>, value: Vec<u8>) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut entry = Entry {
            header: EntryHeader {
                crc: 0,
                timestamp,
                key_size: key.len() as u32,
                value_size: value.len() as u32,
            },
            key,
            value,
        };
        entry.update_crc();
        entry
    }

    pub fn update_crc(&mut self) {
        let mut hasher = Hasher::new();
        hasher.update(&self.header.timestamp.to_be_bytes());
        hasher.update(&self.header.key_size.to_be_bytes());
        hasher.update(&self.header.value_size.to_be_bytes());
        hasher.update(&self.key);
        hasher.update(&self.value);
        self.header.crc = hasher.finalize();
    }

    pub fn encode(&self) -> Bytes {
        let total_len = HEADER_SIZE + self.key.len() + self.value.len();
        let mut buf = BytesMut::with_capacity(total_len);

        buf.put_u32(self.header.crc);
        buf.put_u64(self.header.timestamp);
        buf.put_u32(self.header.key_size);
        buf.put_u32(self.header.value_size);
        buf.put_slice(&self.key);
        buf.put_slice(&self.value);

        buf.freeze()
    }
}
