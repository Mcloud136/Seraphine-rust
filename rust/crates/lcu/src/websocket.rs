use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async_tls_with_config, Connector};
use zeroize::Zeroizing;

use crate::LcuClient;
use seraphine_resource_scheduler::{GameState, ResourceScheduler};

pub const GAMEFLOW_TOPIC: &str = "OnJsonApiEvent_lol-gameflow_v1_gameflow-phase";
pub const CHAMP_SELECT_TOPIC: &str = "OnJsonApiEvent_lol-champ-select_v1_session";
pub const SUMMONER_TOPIC: &str = "OnJsonApiEvent_lol-summoner_v1_current-summoner";
pub const ENTITLEMENT_TOPIC: &str = "OnJsonApiEvent_entitlements_v1_token";

#[derive(Clone, Debug)]
pub struct LcuEvent {
    pub seq: u64,
    pub uri: String,
    pub event_type: String,
    pub data: Value,
    pub snapshot: bool,
}

#[derive(Debug, Default)]
pub struct WsStats {
    pub received: AtomicU64,
    pub dropped: AtomicU64,
    pub reconnects: AtomicU64,
    pub snapshots: AtomicU64,
}

struct BufferedEvent {
    source_seq: Option<u64>,
    order: u64,
    event: LcuEvent,
}

pub fn game_state_from_phase(phase: &str) -> GameState {
    match phase {
        "None" => GameState::NotRunning,
        "Lobby" => GameState::Lobby,
        "ChampSelect" => GameState::ChampSelect,
        "GameStart" | "InProgress" => GameState::InGame,
        "EndOfGame" => GameState::EndOfGame,
        "Reconnect" => GameState::Reconnect,
        _ => GameState::ClientStarting,
    }
}

fn accept_source_seq(last: &mut Option<u64>, source_seq: Option<u64>) -> bool {
    let Some(source_seq) = source_seq else {
        return true;
    };
    if last.is_some_and(|last| source_seq <= last) {
        return false;
    }
    *last = Some(source_seq);
    true
}

fn emit(sender: &mpsc::Sender<LcuEvent>, event: LcuEvent, stats: &Arc<WsStats>) -> bool {
    if sender.try_send(event).is_ok() {
        true
    } else {
        stats.dropped.fetch_add(1, Ordering::Relaxed);
        false
    }
}

fn update_scheduler(event: &LcuEvent, scheduler: &ResourceScheduler) {
    if event.uri.ends_with("gameflow-phase") {
        if let Some(phase) = event.data.as_str() {
            scheduler.set_game_state(game_state_from_phase(phase));
        }
    }
}

fn parse_event(text: &str) -> Option<(Option<u64>, LcuEvent)> {
    let frame = serde_json::from_str::<Value>(text).ok()?;
    let frame = frame.as_array()?;
    if frame.len() < 3 || frame[0].as_u64() != Some(8) {
        return None;
    }
    let (source_seq, payload) = if let Some(event) = frame[2].as_array() {
        (event.first().and_then(Value::as_u64), event.get(1)?)
    } else {
        (frame[2].get("seq").and_then(Value::as_u64), &frame[2])
    };
    let data = payload.as_object()?;
    let uri = data.get("uri").and_then(Value::as_str)?;
    let event_type = data
        .get("eventType")
        .and_then(Value::as_str)
        .unwrap_or("Update")
        .to_string();
    let payload = data.get("data").cloned().unwrap_or(Value::Null);
    Some((
        source_seq,
        LcuEvent {
            seq: 0,
            uri: uri.to_string(),
            event_type,
            data: payload,
            snapshot: false,
        },
    ))
}

async fn connect(
    port: u16,
    token: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("riot:{token}"));
    let mut request = format!("wss://127.0.0.1:{port}/").into_client_request()?;
    request
        .headers_mut()
        .insert(header::AUTHORIZATION, format!("Basic {encoded}").parse()?);
    request
        .headers_mut()
        .insert(header::SEC_WEBSOCKET_PROTOCOL, "wamp".parse()?);
    let (socket, _) = connect_async_tls_with_config(
        request,
        None,
        true,
        Some(Connector::NativeTls(
            native_tls::TlsConnector::builder()
                .danger_accept_invalid_certs(true)
                .build()?,
        )),
    )
    .await?;
    Ok(socket)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_websocket(
    port: u16,
    token: Zeroizing<String>,
    client: LcuClient,
    topics: Vec<String>,
    sender: mpsc::Sender<LcuEvent>,
    mut shutdown: mpsc::Receiver<()>,
    scheduler: ResourceScheduler,
    stats: Arc<WsStats>,
) {
    let mut backoff = Duration::from_millis(250);
    let sequence = AtomicU64::new(1);

    'reconnect: loop {
        match shutdown.try_recv() {
            Ok(_) | Err(mpsc::error::TryRecvError::Disconnected) => break,
            Err(mpsc::error::TryRecvError::Empty) => {}
        }
        let mut socket = match connect(port, token.as_str()).await {
            Ok(socket) => socket,
            Err(error) => {
                eprintln!("seraphine-sidecar: websocket connect failed: {error}");
                stats.reconnects.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(8));
                continue;
            }
        };
        backoff = Duration::from_millis(250);
        // WAMP sequence numbers are scoped to a connection and may reset.
        let mut last_source_seq = None;

        let mut subscribed = true;
        for topic in &topics {
            if socket
                .send(Message::Text(
                    serde_json::to_string(&serde_json::json!([5, topic]))
                        .unwrap()
                        .into(),
                ))
                .await
                .is_err()
            {
                subscribed = false;
                break;
            }
        }
        if !subscribed {
            stats.reconnects.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(8));
            continue 'reconnect;
        }

        let snapshot_future = async {
            let (gameflow, champ_select, summoner) = tokio::join!(
                client.get_json_fresh("/lol-gameflow/v1/gameflow-phase"),
                client.get_json_fresh("/lol-champ-select/v1/session"),
                client.get_json_fresh("/lol-summoner/v1/current-summoner"),
            );
            [
                ("/lol-gameflow/v1/gameflow-phase", gameflow),
                ("/lol-champ-select/v1/session", champ_select),
                ("/lol-summoner/v1/current-summoner", summoner),
            ]
            .into_iter()
            .filter_map(|(uri, result)| {
                result.ok().map(|data| LcuEvent {
                    seq: sequence.fetch_add(1, Ordering::Relaxed),
                    uri: uri.to_string(),
                    event_type: "Update".to_string(),
                    data,
                    snapshot: true,
                })
            })
            .collect::<Vec<_>>()
        };
        tokio::pin!(snapshot_future);
        let mut buffered = Vec::new();
        let mut socket_failed = false;
        let snapshots = loop {
            tokio::select! {
                _ = shutdown.recv() => return,
                snapshots = &mut snapshot_future => break snapshots,
                message = socket.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            if let Some((source_seq, event)) = parse_event(&text) {
                                stats.received.fetch_add(1, Ordering::Relaxed);
                                buffered.push(BufferedEvent {
                                    source_seq,
                                    order: buffered.len() as u64,
                                    event,
                                });
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = socket.send(Message::Pong(payload)).await;
                        }
                        Some(Ok(Message::Close(_))) | None => {
                            socket_failed = true;
                            break Vec::new();
                        }
                        _ => {}
                    }
                }
            }
        };
        if socket_failed {
            stats.reconnects.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(8));
            continue 'reconnect;
        }

        for event in &snapshots {
            update_scheduler(event, &scheduler);
            stats.snapshots.fetch_add(1, Ordering::Relaxed);
            emit(&sender, event.clone(), &stats);
        }

        buffered.sort_by_key(|event| (event.source_seq.unwrap_or(u64::MAX), event.order));
        let mut seen = HashSet::new();
        for buffered in buffered {
            if let Some(source_seq) = buffered.source_seq {
                if !seen.insert(source_seq) {
                    continue;
                }
                last_source_seq = Some(source_seq);
            }
            if snapshots.iter().any(|snapshot| {
                snapshot.uri == buffered.event.uri && snapshot.data == buffered.event.data
            }) {
                continue;
            }
            let mut event = buffered.event;
            event.seq = sequence.fetch_add(1, Ordering::Relaxed);
            update_scheduler(&event, &scheduler);
            emit(&sender, event, &stats);
        }

        let (gameflow, champ_select, summoner) = tokio::join!(
            client.get_json_fresh("/lol-gameflow/v1/gameflow-phase"),
            client.get_json_fresh("/lol-champ-select/v1/session"),
            client.get_json_fresh("/lol-summoner/v1/current-summoner"),
        );
        for (uri, result) in [
            ("/lol-gameflow/v1/gameflow-phase", gameflow),
            ("/lol-champ-select/v1/session", champ_select),
            ("/lol-summoner/v1/current-summoner", summoner),
        ] {
            if let Ok(data) = result {
                let event = LcuEvent {
                    seq: sequence.fetch_add(1, Ordering::Relaxed),
                    uri: uri.to_string(),
                    event_type: "Update".to_string(),
                    data,
                    snapshot: true,
                };
                update_scheduler(&event, &scheduler);
                stats.snapshots.fetch_add(1, Ordering::Relaxed);
                emit(&sender, event, &stats);
            }
        }

        loop {
            tokio::select! {
                _ = shutdown.recv() => return,
                message = socket.next() => {
                    let Some(Ok(Message::Text(text))) = message else {
                        break;
                    };
                    let Some((source_seq, mut event)) = parse_event(&text) else {
                        continue;
                    };
                    if let Some(source_seq) = source_seq {
                        if !accept_source_seq(&mut last_source_seq, Some(source_seq)) {
                            continue;
                        }
                    }
                    event.seq = sequence.fetch_add(1, Ordering::Relaxed);
                    stats.received.fetch_add(1, Ordering::Relaxed);
                    update_scheduler(&event, &scheduler);
                    emit(&sender, event, &stats);
                }
            }
        }
        stats.reconnects.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_sequence_watermark_rejects_replays() {
        let mut last = None;
        assert!(accept_source_seq(&mut last, Some(10)));
        assert!(!accept_source_seq(&mut last, Some(10)));
        assert!(!accept_source_seq(&mut last, Some(9)));
        assert!(accept_source_seq(&mut last, Some(11)));
        assert!(accept_source_seq(&mut last, None));
    }

    #[test]
    fn source_sequence_watermark_resets_for_new_connection() {
        let mut old_connection = Some(100);
        assert!(!accept_source_seq(&mut old_connection, Some(1)));
        let mut new_connection = None;
        assert!(accept_source_seq(&mut new_connection, Some(1)));
    }

    #[test]
    fn maps_gameflow_phases() {
        assert_eq!(game_state_from_phase("Lobby"), GameState::Lobby);
        assert_eq!(game_state_from_phase("ChampSelect"), GameState::ChampSelect);
        assert_eq!(game_state_from_phase("InProgress"), GameState::InGame);
    }

    #[test]
    fn parses_optional_wamp_sequence() {
        let event = parse_event(
            r#"[8,"topic",{"seq":42,"uri":"/lol-gameflow/v1/gameflow-phase","eventType":"Update","data":"Lobby"}]"#,
        )
        .unwrap();
        assert_eq!(event.0, Some(42));
        assert_eq!(event.1.data, "Lobby");
        let event = parse_event(
            r#"[8,"topic",[43,{"uri":"/lol-gameflow/v1/gameflow-phase","eventType":"Update","data":"Lobby"}]]"#,
        )
        .unwrap();
        assert_eq!(event.0, Some(43));
    }
}
