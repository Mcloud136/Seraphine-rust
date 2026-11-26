use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use seraphine_lcu::websocket::{
    run_websocket, LcuEvent, WsStats, CHAMP_SELECT_TOPIC, ENTITLEMENT_TOPIC, GAMEFLOW_TOPIC,
    SUMMONER_TOPIC,
};
use seraphine_lcu::LcuClient;
use seraphine_lcu::SgpClient;
use seraphine_platform::{list_league_client_pids, read_process_command_line};
use seraphine_resource_scheduler::{Priority, ResourceScheduler};

use crate::error::CoreError;
use crate::paths::AppPaths;

pub const EVENT_CHANNEL_CAPACITY: usize = 256;
const WS_QUEUE_CAPACITY: usize = 128;
const REQUEST_BUDGET: usize = 128 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub pid: u32,
    pub port: u16,
    pub server: String,
    pub epoch: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub connected: bool,
    pub pid: u32,
    pub port: u16,
    pub server: String,
    pub epoch: u64,
    pub websocket_received: u64,
    pub websocket_dropped: u64,
    pub websocket_reconnects: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SessionEvent {
    Connected {
        info: SessionInfo,
    },
    Disconnected {
        reason: String,
    },
    Lcu {
        epoch: u64,
        seq: u64,
        uri: String,
        event_type: String,
        data: Value,
    },
    Error {
        message: String,
    },
}

struct ConnectedSession {
    info: SessionInfo,
    lcu: LcuClient,
    sgp: Option<SgpClient>,
    sgp_token: Option<Zeroizing<String>>,
    ws_shutdown: mpsc::Sender<()>,
    ws_task: JoinHandle<()>,
    forward_task: JoinHandle<()>,
    stats: Arc<WsStats>,
}

impl ConnectedSession {
    fn shutdown(&self) {
        let _ = self.ws_shutdown.try_send(());
        self.ws_task.abort();
        self.forward_task.abort();
    }
}

pub struct SessionManager {
    pub paths: AppPaths,
    scheduler: ResourceScheduler,
    events: broadcast::Sender<SessionEvent>,
    session: RwLock<Option<ConnectedSession>>,
    epoch: AtomicU64,
    auto_connect: AtomicBool,
}

impl SessionManager {
    pub fn new(paths: AppPaths) -> Arc<Self> {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Arc::new(Self {
            paths,
            scheduler: ResourceScheduler::new(),
            events,
            session: RwLock::new(None),
            epoch: AtomicU64::new(0),
            auto_connect: AtomicBool::new(true),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    pub fn scheduler(&self) -> ResourceScheduler {
        self.scheduler.clone()
    }

    pub fn set_auto_connect(&self, enabled: bool) {
        self.auto_connect.store(enabled, Ordering::SeqCst);
    }

    pub async fn status(&self) -> SessionStatus {
        let guard = self.session.read().await;
        match guard.as_ref() {
            Some(session) => SessionStatus {
                connected: true,
                pid: session.info.pid,
                port: session.info.port,
                server: session.info.server.clone(),
                epoch: session.info.epoch,
                websocket_received: session.stats.received.load(Ordering::Relaxed),
                websocket_dropped: session.stats.dropped.load(Ordering::Relaxed),
                websocket_reconnects: session.stats.reconnects.load(Ordering::Relaxed),
            },
            None => SessionStatus {
                connected: false,
                pid: 0,
                port: 0,
                server: String::new(),
                epoch: self.epoch.load(Ordering::SeqCst),
                websocket_received: 0,
                websocket_dropped: 0,
                websocket_reconnects: 0,
            },
        }
    }

    pub async fn lcu(&self) -> Option<LcuClient> {
        self.session
            .read()
            .await
            .as_ref()
            .map(|session| session.lcu.clone())
    }

    pub async fn sgp_token(&self) -> Option<Zeroizing<String>> {
        self.session
            .read()
            .await
            .as_ref()
            .and_then(|session| session.sgp_token.clone())
    }

    /// Connects to the first League client or to a specific process id.
    pub async fn connect(&self, pid: Option<u32>) -> Result<SessionInfo, CoreError> {
        let pids =
            list_league_client_pids().map_err(|error| CoreError::Platform(error.to_string()))?;
        let pid = match pid {
            Some(pid) => pid,
            None => pids
                .first()
                .copied()
                .ok_or_else(|| CoreError::Platform("League client not found".into()))?,
        };
        let command_line = read_process_command_line(pid)
            .map_err(|error| CoreError::Platform(error.to_string()))?;
        let port = command_line
            .port
            .ok_or_else(|| CoreError::InvalidParams("LCU port not found".into()))?;
        let token = command_line
            .token
            .clone()
            .ok_or_else(|| CoreError::InvalidParams("LCU token not found".into()))?;
        let server = command_line.server.clone().unwrap_or_default();

        let lcu = LcuClient::new(port, token.as_str())
            .map_err(|error| CoreError::Lcu(error.to_string()))?;
        let sgp_token = lcu
            .get_json("/entitlements/v1/token")
            .await
            .ok()
            .and_then(|data| {
                data.get("accessToken")
                    .and_then(Value::as_str)
                    .filter(|token| !token.is_empty())
                    .map(|token| Zeroizing::new(token.to_string()))
            });
        let sgp = SgpClient::new(&server).ok();

        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let info = SessionInfo {
            pid,
            port,
            server,
            epoch,
        };

        if let Some(previous) = self.session.write().await.take() {
            previous.shutdown();
        }

        let (event_tx, mut event_rx) = mpsc::channel::<LcuEvent>(WS_QUEUE_CAPACITY);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);
        let stats = Arc::new(WsStats::default());
        let ws_task = tokio::spawn(run_websocket(
            port,
            token,
            lcu.clone(),
            vec![
                GAMEFLOW_TOPIC.to_string(),
                CHAMP_SELECT_TOPIC.to_string(),
                SUMMONER_TOPIC.to_string(),
                ENTITLEMENT_TOPIC.to_string(),
            ],
            event_tx,
            shutdown_rx,
            self.scheduler.clone(),
            Arc::clone(&stats),
        ));

        let events = self.events.clone();
        let forward_task = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let _ = events.send(SessionEvent::Lcu {
                    epoch,
                    seq: event.seq,
                    uri: event.uri,
                    event_type: event.event_type,
                    data: event.data,
                });
            }
        });

        let session = ConnectedSession {
            info: info.clone(),
            lcu,
            sgp,
            sgp_token,
            ws_shutdown: shutdown_tx,
            ws_task,
            forward_task,
            stats,
        };
        *self.session.write().await = Some(session);
        let _ = self
            .events
            .send(SessionEvent::Connected { info: info.clone() });
        Ok(info)
    }

    pub async fn disconnect(&self, reason: &str) {
        if let Some(session) = self.session.write().await.take() {
            session.shutdown();
        }
        let _ = self.events.send(SessionEvent::Disconnected {
            reason: reason.to_string(),
        });
    }

    /// Starts the process monitor that keeps the session in sync with the client.
    /// The caller decides which async runtime owns the loop.
    pub async fn monitor(self: Arc<Self>) {
        let mut interval = tokio::time::interval(Duration::from_millis(1500));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let pids = list_league_client_pids().unwrap_or_default();
            let connected = self.status().await;
            if connected.connected && !pids.contains(&connected.pid) {
                self.disconnect("League client exited").await;
                continue;
            }
            if !connected.connected && self.auto_connect.load(Ordering::SeqCst) && !pids.is_empty()
            {
                if let Err(error) = self.connect(None).await {
                    let _ = self.events.send(SessionEvent::Error {
                        message: error.to_string(),
                    });
                }
            }
        }
    }

    /// Fetches an LCU resource through the shared scheduler.
    pub async fn lcu_get(&self, path: &str) -> Result<Value, CoreError> {
        let lcu = self.lcu().await.ok_or(CoreError::NotConnected)?;
        let _permit = self
            .scheduler
            .acquire(Priority::P2, REQUEST_BUDGET)
            .await
            .map_err(|error| CoreError::Deferred(format!("{error:?}")))?;
        lcu.get_json(path)
            .await
            .map_err(|error| CoreError::Lcu(error.to_string()))
    }

    /// Fetches a fresh LCU resource, bypassing the short-lived read cache.
    pub async fn lcu_get_fresh(&self, path: &str) -> Result<Value, CoreError> {
        let lcu = self.lcu().await.ok_or(CoreError::NotConnected)?;
        let _permit = self
            .scheduler
            .acquire(Priority::P1, REQUEST_BUDGET)
            .await
            .map_err(|error| CoreError::Deferred(format!("{error:?}")))?;
        lcu.get_json_fresh(path)
            .await
            .map_err(|error| CoreError::Lcu(error.to_string()))
    }

    /// Reads an optional resource, treating HTTP 404 as an empty result.
    pub async fn lcu_get_optional(&self, path: &str) -> Result<Option<Value>, CoreError> {
        let lcu = self.lcu().await.ok_or(CoreError::NotConnected)?;
        let _permit = self
            .scheduler
            .acquire(Priority::P2, REQUEST_BUDGET)
            .await
            .map_err(|error| CoreError::Deferred(format!("{error:?}")))?;
        match lcu.get_json(path).await {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.status().map(|status| status.as_u16()) == Some(404) => Ok(None),
            Err(error) => Err(CoreError::Lcu(error.to_string())),
        }
    }

    /// Fetches a Tencent SGP resource with the current entitlement token.
    pub async fn sgp_get(&self, path: &str, params: Option<Value>) -> Result<Value, CoreError> {
        let (client, token) = {
            let guard = self.session.read().await;
            let session = guard.as_ref().ok_or(CoreError::NotConnected)?;
            let client = session
                .sgp
                .clone()
                .ok_or_else(|| CoreError::Lcu("SGP client is unavailable".into()))?;
            let token = session
                .sgp_token
                .clone()
                .ok_or_else(|| CoreError::Lcu("SGP token is unavailable".into()))?;
            (client, token)
        };
        let _permit = self
            .scheduler
            .acquire(Priority::P2, REQUEST_BUDGET)
            .await
            .map_err(|error| CoreError::Deferred(format!("{error:?}")))?;
        client
            .get_json(path, token.as_str(), params.as_ref())
            .await
            .map_err(|error| CoreError::Lcu(error.to_string()))
    }
}

pub fn is_windows_11() -> bool {
    seraphine_platform::windows_build().is_some_and(|build| build >= 22000)
}
