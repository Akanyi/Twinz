use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// 动态值类型 (Duck Typing Support)
/// 类似于 JSON 数据模型，用于插件间或客户端与服务器间交换数据
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)] // 让 serde 自动推断类型
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>), // JSON 不直接支持 Bytes，但在我们的协议中很重要
    Array(Vec<Value>),
    Map(HashMap<String, Value>),
}

// 宏 helper
#[macro_export]
macro_rules! value {
    ($($tt:tt)+) => {
        serde_json::json!($($tt)+)
    };
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    // Duck Typing: 尝试转换
    pub fn to_string_lossy(&self) -> String {
        match self {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::String(s) => s.clone(),
            Value::Bytes(b) => String::from_utf8_lossy(b).to_string(),
            Value::Array(arr) => format!("{:?}", arr),
            Value::Map(map) => format!("{:?}", map),
        }
    }
}

// 实现 From 转换以便于使用
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Integer(v as i64)
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Integer(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}
impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Value::Bytes(v)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string_lossy())
    }
}
