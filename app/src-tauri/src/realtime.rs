//! Push channel for updates.
//!
//! CI inserts a row into the Supabase `update` table for every release. Subscribing to those
//! inserts lets running copies of `OpenClip` react within seconds instead of waiting for the next
//! poll. The message is only a nudge: whatever gets installed is still verified against the
//! updater public key, so a spoofed or replayed event cannot install anything.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

const TOPIC: &str = "realtime:openclip-updates";
const HEARTBEAT: Duration = Duration::from_secs(25);
const MIN_BACKOFF: Duration = Duration::from_secs(3);
const MAX_BACKOFF: Duration = Duration::from_mins(2);

/// Turns the configured value into a Supabase Realtime websocket URL.
///
/// Accepts the websocket URL itself (`wss://<project>.supabase.co/realtime/v1/websocket?apikey=…`)
/// or the project's `https://` URL together with the public (anon) key.
pub(crate) fn websocket_url(configured: &str, anon_key: Option<&str>) -> Option<Url> {
    let mut url = Url::parse(configured.trim()).ok()?;
    match url.scheme() {
        "ws" | "wss" => {}
        "https" | "http" => {
            let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
            url.set_scheme(scheme).ok()?;
        }
        _ => return None,
    }
    if !url.path().contains("/realtime/") {
        url.set_path("/realtime/v1/websocket");
    }
    let has_key = url.query_pairs().any(|(name, _)| name == "apikey");
    let has_version = url.query_pairs().any(|(name, _)| name == "vsn");
    if !has_key {
        let key = anon_key?;
        url.query_pairs_mut().append_pair("apikey", key);
    }
    if !has_version {
        url.query_pairs_mut().append_pair("vsn", "1.0.0");
    }
    Some(url)
}

fn join_message() -> String {
    json!({
        "topic": TOPIC,
        "event": "phx_join",
        "payload": {
            "config": {
                "broadcast": { "self": false },
                "presence": { "key": "" },
                "postgres_changes": [
                    { "event": "INSERT", "schema": "public", "table": "update" }
                ]
            }
        },
        "ref": "1",
        "join_ref": "1"
    })
    .to_string()
}

fn heartbeat_message(reference: u64) -> String {
    json!({ "topic": "phoenix", "event": "heartbeat", "payload": {}, "ref": reference.to_string() })
        .to_string()
}

/// True when a Realtime frame announces a newly inserted release.
pub(crate) fn is_release_insert(frame: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(frame) else {
        return false;
    };
    if value.get("event").and_then(Value::as_str) != Some("postgres_changes") {
        return false;
    }
    let data = value.pointer("/payload/data");
    data.and_then(|data| data.get("type"))
        .and_then(Value::as_str)
        == Some("INSERT")
        && data
            .and_then(|data| data.get("table"))
            .and_then(Value::as_str)
            .is_none_or(|table| table == "update")
}

async fn run_once(url: &Url, nudge: &UnboundedSender<()>) -> Result<(), String> {
    let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .map_err(|error| error.to_string())?;
    socket
        .send(Message::text(join_message()))
        .await
        .map_err(|error| error.to_string())?;
    tracing::info!("update channel connected");

    let mut reference = 1_u64;
    let mut ticker = tokio::time::interval(HEARTBEAT);
    ticker.tick().await;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                reference += 1;
                socket
                    .send(Message::text(heartbeat_message(reference)))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            frame = socket.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    if is_release_insert(text.as_str()) {
                        let _ = nudge.send(());
                    }
                }
                Some(Ok(Message::Close(_))) | None => return Err("connection closed".to_owned()),
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error.to_string()),
            }
        }
    }
}

/// Keeps the channel connected, reconnecting with backoff, for the life of the app.
pub(crate) async fn watch(url: Url, nudge: UnboundedSender<()>) {
    let mut backoff = MIN_BACKOFF;
    loop {
        match run_once(&url, &nudge).await {
            Ok(()) => backoff = MIN_BACKOFF,
            Err(error) => tracing::debug!(%error, "update channel disconnected"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_websocket_url_is_used_as_given() {
        let url = websocket_url(
            "wss://abc.supabase.co/realtime/v1/websocket?apikey=KEY&vsn=1.0.0",
            None,
        )
        .expect("valid");
        assert_eq!(url.scheme(), "wss");
        assert_eq!(
            url.query_pairs()
                .filter(|(name, _)| name == "apikey")
                .count(),
            1
        );
    }

    #[test]
    fn a_project_url_becomes_a_realtime_url_with_the_anon_key() {
        let url = websocket_url("https://abc.supabase.co", Some("ANON")).expect("valid");
        assert_eq!(url.scheme(), "wss");
        assert_eq!(url.path(), "/realtime/v1/websocket");
        assert!(
            url.query_pairs()
                .any(|(name, value)| name == "apikey" && value == "ANON")
        );
        assert!(
            url.query_pairs()
                .any(|(name, value)| name == "vsn" && value == "1.0.0")
        );
    }

    #[test]
    fn a_project_url_without_a_key_is_rejected() {
        assert!(websocket_url("https://abc.supabase.co", None).is_none());
        assert!(websocket_url("ftp://abc.supabase.co", Some("ANON")).is_none());
        assert!(websocket_url("not a url", Some("ANON")).is_none());
    }

    #[test]
    fn only_inserts_into_the_update_table_count_as_releases() {
        let insert = r#"{"topic":"realtime:openclip-updates","event":"postgres_changes","payload":{"data":{"type":"INSERT","table":"update","record":{"id":7}}}}"#;
        let delete =
            r#"{"event":"postgres_changes","payload":{"data":{"type":"DELETE","table":"update"}}}"#;
        let other = r#"{"event":"postgres_changes","payload":{"data":{"type":"INSERT","table":"profiles"}}}"#;
        assert!(is_release_insert(insert));
        assert!(!is_release_insert(delete));
        assert!(!is_release_insert(other));
        assert!(!is_release_insert(r#"{"event":"phx_reply","payload":{}}"#));
        assert!(!is_release_insert("not json"));
    }

    #[test]
    fn the_join_message_subscribes_to_update_inserts() {
        let value: Value = serde_json::from_str(&join_message()).unwrap();
        assert_eq!(value["event"], "phx_join");
        assert_eq!(
            value["payload"]["config"]["postgres_changes"][0]["table"],
            "update"
        );
    }
}
