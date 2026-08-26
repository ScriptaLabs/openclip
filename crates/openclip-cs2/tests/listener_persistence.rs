use std::sync::{Arc, Mutex};

use openclip_core::MatchRepository;
use openclip_cs2::listener::serve_on_listener;
use openclip_cs2::{GsiPayloadHandler, GsiService};
use openclip_storage::SqliteRepository;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const ROUND_LIVE: &[u8] = include_bytes!("fixtures/round_live.json");
const KILL_1: &[u8] = include_bytes!("fixtures/kill_1.json");
const MALFORMED: &[u8] = include_bytes!("fixtures/malformed.json");

#[tokio::test]
async fn loopback_listener_accepts_payloads_and_persists_match_events() {
    let repository = Arc::new(Mutex::new(
        SqliteRepository::in_memory().expect("in-memory database"),
    ));
    let service = Arc::new(GsiService::new(Arc::clone(&repository)));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let handler: Arc<dyn GsiPayloadHandler> = service;
    let task = tokio::spawn(serve_on_listener(listener, handler));

    assert!(post(address, ROUND_LIVE).await.starts_with("HTTP/1.1 204"));
    assert!(post(address, KILL_1).await.starts_with("HTTP/1.1 204"));
    assert!(post(address, MALFORMED).await.starts_with("HTTP/1.1 400"));

    let matches = repository
        .lock()
        .expect("database lock")
        .list_matches()
        .expect("matches query");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].map.as_deref(), Some("de_mirage"));
    assert_eq!(matches[0].rounds.len(), 1);
    assert!(
        matches[0].rounds[0]
            .events
            .iter()
            .any(|event| event.event_type == openclip_core::GameEventType::Kill)
    );

    task.abort();
}

async fn post(address: std::net::SocketAddr, body: &[u8]) -> String {
    let mut stream = TcpStream::connect(address).await.expect("connects");
    let request = format!(
        "POST / HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("request headers");
    stream.write_all(body).await.expect("request body");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("response");
    String::from_utf8(response).expect("HTTP response is UTF-8")
}
