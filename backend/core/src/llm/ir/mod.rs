//! Provider-neutral semantic IR for model operations.
//!
//! Provider wire types are decoded into this module at the protocol boundary.
//! Business services and transport adapters must not expose provider events.

mod common;
pub mod embeddings;
pub mod generation;
pub mod models;
mod operation;
pub mod tokens;

pub use common::*;
pub use operation::*;
