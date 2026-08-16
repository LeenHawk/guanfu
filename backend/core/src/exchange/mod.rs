//! 外部交换格式的纯 codec 层。
//!
//! 只做格式 ↔ typed definition 的转换,不触碰数据库;落库在
//! [`crate::services::exchange`]。

pub mod ccv2;
pub mod entry;
pub mod png;
