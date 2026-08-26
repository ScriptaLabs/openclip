//! Counter-Strike 2 Game State Integration.
//!
//! This crate owns CS2 payload parsing, state interpretation, loopback HTTP transport, and GSI
//! configuration discovery. It depends only on generic `OpenClip` domain contracts and never on
//! capture or UI code.

pub mod adapter;
pub mod installer;
pub mod listener;
pub mod models;
pub mod service;

pub use adapter::Cs2Adapter;
pub use installer::{
    ConfigInstallAction, ConfigInstallResult, Cs2Install, GSI_CONFIG_FILE_NAME, SteamLocator,
    install_gsi_config, render_gsi_config,
};
pub use listener::{GsiEndpoint, GsiPayloadHandler, ListenerError, serve};
pub use models::{Cs2ConnectionState, Cs2Status, GsiPayload};
pub use service::GsiService;
