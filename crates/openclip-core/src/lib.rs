//! Domain types and ports shared by every `OpenClip` component.
//!
//! This crate deliberately contains no Tauri, `SQLite`, CS2, or Windows API code.

pub mod models;
pub mod ports;
pub mod settings;

pub use models::*;
pub use ports::*;
pub use settings::*;
