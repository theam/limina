//! Limina's research model: projects, the H → E → F evidence graph, steering, collaboration and
//! the knowledge views built on top of it. Ported from the Python runtime with the same rules,
//! error messages and persisted shapes.

pub mod common;
pub mod engines;
pub mod environment;
pub mod retry;
pub mod usage;

pub mod collaboration;
pub mod exporter;
pub mod import;
pub mod knowledge;
pub mod progress;
pub mod project;
pub mod research;
pub mod service;

pub use engines::Engine;
