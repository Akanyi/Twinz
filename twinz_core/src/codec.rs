use crate::types::Value;
use bytes::{Buf, BufMut, BytesMut};
use std::io::Cursor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use twinz_transport::TwinzStream;

/// 简单的长度前缀协议 Codec (Length-Prefixed JSON/MsgPack)
/// Format: [Length: u32][Payload: Bytes]
pub struct ValueCodec;

impl ValueCodec {
    /// 从流中读取一个 Value
    pub async fn read_value(
        stream: &mut (dyn TwinzStream + Unpin),
    ) -> Result<Option<Value>, Box<dyn std::error::Error + Send + Sync>> {
        // 读取长度 (4 字节)
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(Box::new(e)),
        };

        // 解析长度
        let mut len_rdr = Cursor::new(&len_buf);
        let len = len_rdr.get_u32();

        // 读取负载
        let mut payload = vec![0u8; len as usize];
        stream.read_exact(&mut payload).await?;

        // 反序列化 (使用 JSON 以便调试，可以切换到 MsgPack)
        let value: Value = serde_json::from_slice(&payload)?;
        Ok(Some(value))
    }

    /// 写入一个 Value 到流
    pub async fn write_value(
        stream: &mut (dyn TwinzStream + Unpin),
        value: &Value,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::to_vec(value)?;
        let len = payload.len() as u32;

        let mut buf = BytesMut::with_capacity(4 + payload.len());
        buf.put_u32(len);
        buf.put_slice(&payload);

        stream.write_all(&buf).await?;
        Ok(()) // Flush?
    }
}
