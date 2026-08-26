use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::routing::post;
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GsiEndpoint {
    pub port: u16,
}

impl GsiEndpoint {
    #[must_use]
    pub const fn socket_addr(self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port)
    }
}

impl Default for GsiEndpoint {
    fn default() -> Self {
        Self { port: 21_987 }
    }
}

pub trait GsiPayloadHandler: Send + Sync + 'static {
    /// Handles one complete request body.
    ///
    /// # Errors
    ///
    /// Returns a parsing, adapter, or persistence error for HTTP status mapping.
    fn handle_payload(&self, data: &[u8]) -> openclip_core::Result<()>;
}

#[derive(Debug, Error)]
pub enum ListenerError {
    #[error("could not bind CS2 GSI listener at {address}: {source}")]
    Bind {
        address: SocketAddr,
        source: std::io::Error,
    },
    #[error("refusing to expose CS2 GSI listener on non-loopback address {0}")]
    NonLoopback(SocketAddr),
    #[error("CS2 GSI listener failed: {0}")]
    Serve(std::io::Error),
}

/// Binds the loopback endpoint and serves GSI requests until the task is cancelled.
///
/// # Errors
///
/// Returns a bind error when the port is unavailable, or a server error if HTTP serving fails.
pub async fn serve(
    endpoint: GsiEndpoint,
    handler: Arc<dyn GsiPayloadHandler>,
) -> Result<(), ListenerError> {
    let address = endpoint.socket_addr();
    let listener = TcpListener::bind(address)
        .await
        .map_err(|source| ListenerError::Bind { address, source })?;
    serve_on_listener(listener, handler).await
}

/// Serves on a pre-bound loopback listener. Exposed to support port-zero integration tests.
///
/// # Errors
///
/// Refuses non-loopback listeners and returns any HTTP server failure.
pub async fn serve_on_listener(
    listener: TcpListener,
    handler: Arc<dyn GsiPayloadHandler>,
) -> Result<(), ListenerError> {
    let address = listener.local_addr().map_err(ListenerError::Serve)?;
    if !address.ip().is_loopback() {
        return Err(ListenerError::NonLoopback(address));
    }
    let router = Router::new()
        .route("/", post(receive_payload))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(handler);
    info!(%address, "gsi listener started");
    axum::serve(listener, router)
        .await
        .map_err(ListenerError::Serve)
}

async fn receive_payload(
    State(handler): State<Arc<dyn GsiPayloadHandler>>,
    body: Bytes,
) -> StatusCode {
    match handler.handle_payload(&body) {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(openclip_core::OpenClipError::InvalidGameState(error)) => {
            warn!(%error, bytes = body.len(), "malformed gsi payload");
            StatusCode::BAD_REQUEST
        }
        Err(error) => {
            warn!(%error, "gsi payload processing failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
