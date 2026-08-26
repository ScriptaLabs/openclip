//! Counter-Strike 2 integration boundary.
//!
//! The GSI listener, config installer, and state-diff adapter belong here in Milestone 2. Keeping
//! this crate free of capture dependencies guarantees that gameplay detection cannot become
//! coupled to Windows frame acquisition.

/// Settings owned specifically by the CS2 Game State Integration adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GsiEndpoint {
    pub host: String,
    pub port: u16,
}

impl Default for GsiEndpoint {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 21_987,
        }
    }
}
