//! Provider-neutral semantic IR for model operations.
//!
//! Provider wire types are decoded into this module at the protocol boundary.
//! Business services and transport adapters must not expose provider events.

pub mod audio;
mod common;
pub mod embeddings;
pub mod generation;
pub mod images;
pub mod models;
mod operation;
pub mod platform;
pub mod realtime;
pub mod search;
pub mod tokens;
pub mod video;

pub use common::*;
pub use operation::*;
