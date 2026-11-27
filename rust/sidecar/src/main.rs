#![windows_subsystem = "windows"]

mod asset_cache;

use asset_cache::{AssetCache, AssetFlight, AssetStart};
use seraphine_automation::write_guard::{confirm_write, validate_write};
use seraphine_automation::{AutomationState, DryRunAutomation};
use seraphine_domain::catalog::CatalogBuilder;
use seraphine_domain::community::{
    aram_info, parse_champion_aliases, validate_cached, CHAMPION_ALIAS_URL,
};
use seraphine_domain::opgg::{
    parse_arena_champion_build, parse_other_champion_build, parse_other_tier_list,
    parse_ranked_tier_list,
};
use seraphine_lcu::cache::{CacheFlight, JsonCache, Start};
use seraphine_lcu::websocket::{
    run_websocket, LcuEvent, WsStats, CHAMP_SELECT_TOPIC, ENTITLEMENT_TOPIC, GAMEFLOW_TOPIC,
    SUMMONER_TOPIC,
};
use seraphine_lcu::{
    should_retry_sgp, AssetClient, ConnectionInfo, ExternalClient, LcuClient, LcuError, SgpClient,
};
use seraphine_platform::{
    atomic_replace_file, current_process_memory, list_league_client_pids, list_processes,
    lock_file, read_process_command_line,
};
use seraphine_protocol::{
    decode, decode_length_prefix, encode, Message, MessageKind, Metrics, RpcError, MAX_EVENT_QUEUE,
    MAX_IPC_INFLIGHT_BYTES, MAX_PENDING_REQUESTS, PROTOCOL_VERSION,
};
use seraphine_resource_scheduler::{GameState, Priority, ResourceScheduler};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex, OwnedSemaphorePermit, Semaphore};
use zeroize::Zeroizing;

mod capabilities {
    pub const ALL: &[&str] = &[
        "health",
        "process.list",
        "process.command_line",
        "lcu.summoner_by_pid",
        "diagnostics",
        "scheduler.set_game_state",
        "lcu.connect",
        "lcu.get",
        "lcu.catalog",
        "lcu.websocket",
        "sgp.get",
        "asset.get",
        "external.get",
        "automation.configure",
        "automation.dry_run",
        "opgg.positions",
        "opgg.tier_list",
        "opgg.champion_build",
        "champion_alias.refresh",
        "champion_alias.search",
        "aram_buff.refresh",
        "aram_buff.get",
    ];
}

fn writes_enabled() -> bool {
    env::var("SERAPHINE_ENABLE_LCU_WRITES")
        .map(|value| value != "0")
        .unwrap_or(true)
}

fn active_capabilities() -> Vec<&'static str> {
    let mut capabilities = capabilities::ALL.to_vec();
    if writes_enabled() {
        capabilities.push("lcu.write");
    }
    capabilities
}

#[derive(Clone, Default)]
struct ChampionAliasCache {
    version: String,
    champions: HashMap<String, String>,
}

impl ChampionAliasCache {
    fn from_value(data: &Value) -> Option<Self> {
        let version = data.get("version")?.as_str()?.to_string();
        let champions = data
            .get("champions")?
            .as_object()?
            .iter()
            .filter_map(|(id, aliases)| Some((id.clone(), aliases.as_str()?.to_string())))
            .collect();
        Some(Self { version, champions })
    }

    fn to_value(&self) -> Value {
        json!({ "version": self.version, "champions": self.champions })
    }
}

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
enum AssetSource {
    Lcu(String),
    External(String),
}

impl AssetSource {
    fn cache_key(&self, destination: &Path, max_bytes: usize) -> String {
        let source = match self {
            Self::Lcu(path) => format!("lcu:{path}"),
            Self::External(url) => format!("url:{url}"),
        };
        format!("{}\0{source}\0{max_bytes}", destination.to_string_lossy())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct WriteLedgerKey {
    pid: u32,
    epoch: u64,
    session_id: String,
    action_id: u64,
    action_type: String,
    config_revision: u64,
    operation_id: String,
}

#[derive(Clone)]
struct WriteLedgerEntry {
    request_hash: [u8; 32],
    state: AutomationState,
}

fn write_request_hash(method: &str, path: &str, data: Option<&Value>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update([0]);
    hasher.update(path.as_bytes());
    hasher.update([0]);
    hasher.update([u8::from(data.is_some())]);
    if let Some(data) = data {
        if let Ok(payload) = serde_json::to_vec(data) {
            hasher.update(payload);
        }
    }
    hasher.finalize().into()
}

const MAX_PERSISTED_WRITE_GUARDS: usize = 512;
const MAX_WRITE_GUARD_BYTES: u64 = 256 * 1024;
const WRITE_GUARD_RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Clone, Copy)]
struct PersistedWriteGuard {
    request_hash: [u8; 32],
    completed: bool,
    completed_at_ms: u64,
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

fn hex_decode(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut result = [0_u8; 32];
    let bytes = value.as_bytes();
    for index in 0..32 {
        let high = (bytes[index * 2] as char).to_digit(16)?;
        let low = (bytes[index * 2 + 1] as char).to_digit(16)?;
        result[index] = ((high << 4) | low) as u8;
    }
    Some(result)
}

fn operation_guard_key(pid: u32, operation_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pid.to_le_bytes());
    hasher.update([0]);
    hasher.update(operation_id.as_bytes());
    hex_encode(&hasher.finalize().into())
}

fn load_write_guards(path: &Path) -> (HashMap<String, PersistedWriteGuard>, Option<&'static str>) {
    let length = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (HashMap::new(), None),
        Err(_) => {
            return (
                HashMap::new(),
                Some("persistent write guards are unavailable"),
            )
        }
    };
    if length == 0 || length > MAX_WRITE_GUARD_BYTES {
        return (HashMap::new(), Some("persistent write guards are invalid"));
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                HashMap::new(),
                Some("persistent write guards are unavailable"),
            )
        }
    };
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return (HashMap::new(), Some("persistent write guards are invalid")),
    };
    if value.get("version").and_then(Value::as_u64) != Some(1) {
        return (HashMap::new(), Some("persistent write guards are invalid"));
    }
    let Some(entries) = value.get("entries").and_then(Value::as_array) else {
        return (HashMap::new(), Some("persistent write guards are invalid"));
    };
    if entries.len() > MAX_PERSISTED_WRITE_GUARDS {
        return (HashMap::new(), Some("persistent write guards are invalid"));
    }
    let mut guards = HashMap::new();
    for entry in entries {
        let operation = entry
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let request_hash = entry
            .get("requestHash")
            .and_then(Value::as_str)
            .and_then(hex_decode);
        let completed = match entry.get("state").and_then(Value::as_str) {
            Some("completed") => true,
            Some("outcome_unknown") => false,
            _ => return (HashMap::new(), Some("persistent write guards are invalid")),
        };
        let completed_at_ms = entry
            .get("completedAt")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if operation.len() != 64 || hex_decode(operation).is_none() {
            return (HashMap::new(), Some("persistent write guards are invalid"));
        }
        let Some(request_hash) = request_hash else {
            return (HashMap::new(), Some("persistent write guards are invalid"));
        };
        if guards
            .insert(
                operation.to_string(),
                PersistedWriteGuard {
                    request_hash,
                    completed,
                    completed_at_ms,
                },
            )
            .is_some()
        {
            return (HashMap::new(), Some("persistent write guards are invalid"));
        }
    }
    (guards, None)
}

fn write_guard_payload(guards: &HashMap<String, PersistedWriteGuard>) -> Result<Vec<u8>, RpcError> {
    let entries = guards
        .iter()
        .map(|(operation, guard)| {
            json!({
                "operation": operation,
                "requestHash": hex_encode(&guard.request_hash),
                "state": if guard.completed { "completed" } else { "outcome_unknown" },
                "completedAt": guard.completed_at_ms,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({"version": 1, "entries": entries}))
        .map_err(|_| RpcError::new("PersistenceError", "failed to serialize write guards"))
}

fn prune_expired_completed_guards(guards: &mut HashMap<String, PersistedWriteGuard>, now_ms: u64) {
    guards.retain(|_, guard| {
        !guard.completed
            || guard.completed_at_ms == 0
            || now_ms.saturating_sub(guard.completed_at_ms) < WRITE_GUARD_RETENTION_MS
    });
}

fn save_write_guards(state: &mut State) -> Result<(), RpcError> {
    let _lock = lock_file(&state.write_guard_path.with_extension("lock"))
        .map_err(|_| RpcError::new("PersistenceError", "failed to lock write guards"))?;
    let (mut merged, error) = load_write_guards(&state.write_guard_path);
    if let Some(error) = error {
        return Err(RpcError::new("PersistenceError", error));
    }
    for (operation, incoming) in &state.write_guards {
        match merged.get(operation) {
            None => {
                merged.insert(operation.clone(), *incoming);
            }
            Some(existing) if existing.request_hash != incoming.request_hash => {
                return Err(RpcError::new(
                    "OperationConflict",
                    "operationId was already used for a different write request",
                ));
            }
            Some(existing) if incoming.completed && !existing.completed => {
                merged.insert(operation.clone(), *incoming);
            }
            Some(_) => {}
        }
    }
    prune_expired_completed_guards(&mut merged, unix_millis());
    if merged.len() > MAX_PERSISTED_WRITE_GUARDS {
        return Err(RpcError::new(
            "ResourceDeferred",
            "persistent write guard capacity reached",
        )
        .retryable(true));
    }
    let payload = write_guard_payload(&merged)?;
    write_file_atomic(&state.write_guard_path, &payload)
        .map_err(|_| RpcError::new("PersistenceError", "failed to persist write guards"))?;
    state.write_guards = merged;
    Ok(())
}

enum WriteGuardClaim {
    Leader,
    Existing(bool),
}

fn claim_write_guard(
    state: &mut State,
    operation: String,
    request_hash: [u8; 32],
) -> Result<WriteGuardClaim, RpcError> {
    if state.write_guard_error.is_some() {
        return Err(RpcError::new(
            "PersistenceError",
            "persistent write guards are unavailable",
        ));
    }
    let _lock = lock_file(&state.write_guard_path.with_extension("lock"))
        .map_err(|_| RpcError::new("PersistenceError", "failed to lock write guards"))?;
    let (mut merged, error) = load_write_guards(&state.write_guard_path);
    if let Some(error) = error {
        return Err(RpcError::new("PersistenceError", error));
    }
    if let Some(existing) = merged.get(&operation) {
        if existing.request_hash != request_hash {
            return Err(RpcError::new(
                "OperationConflict",
                "operationId was already used for a different write request",
            ));
        }
        let completed = existing.completed;
        state.write_guards = merged;
        return Ok(WriteGuardClaim::Existing(completed));
    }
    prune_expired_completed_guards(&mut merged, unix_millis());
    if merged.len() >= MAX_PERSISTED_WRITE_GUARDS {
        return Err(RpcError::new(
            "ResourceDeferred",
            "persistent write guard capacity reached",
        )
        .retryable(true));
    }
    merged.insert(
        operation,
        PersistedWriteGuard {
            request_hash,
            completed: false,
            completed_at_ms: 0,
        },
    );
    let payload = write_guard_payload(&merged)?;
    write_file_atomic(&state.write_guard_path, &payload)
        .map_err(|_| RpcError::new("PersistenceError", "failed to persist write guards"))?;
    state.write_guards = merged;
    Ok(WriteGuardClaim::Leader)
}

fn persist_write_guard(
    state: &mut State,
    operation: String,
    request_hash: [u8; 32],
    completed: bool,
) -> Result<(), RpcError> {
    if state.write_guard_error.is_some() {
        return Err(RpcError::new(
            "PersistenceError",
            "persistent write guards are unavailable",
        ));
    }
    let previous = state.write_guards.insert(
        operation.clone(),
        PersistedWriteGuard {
            request_hash,
            completed,
            completed_at_ms: if completed { unix_millis() } else { 0 },
        },
    );
    if let Err(error) = save_write_guards(state) {
        match previous {
            Some(previous) => {
                state.write_guards.insert(operation, previous);
            }
            None => {
                state.write_guards.remove(&operation);
            }
        }
        return Err(error);
    }
    Ok(())
}

fn finalize_write_guard(
    state: &mut State,
    operation: String,
    request_hash: [u8; 32],
    result: Result<Value, RpcError>,
) -> Result<Value, RpcError> {
    if result.is_ok() {
        if let Err(error) = persist_write_guard(state, operation, request_hash, true) {
            return Err(RpcError::new(
                "OutcomeUnknown",
                format!(
                    "write completed but guard persistence failed: {}",
                    error.message
                ),
            ));
        }
    }
    result
}

fn write_ledger_key(
    state: &State,
    epoch: u64,
    operation_id: &str,
    method: &str,
    path: &str,
    data: Option<&Value>,
) -> WriteLedgerKey {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let action_id = segments
        .windows(2)
        .find(|parts| matches!(parts[0], "actions" | "trades" | "swaps"))
        .and_then(|parts| parts[1].parse().ok())
        .unwrap_or(0);
    let action_type = if path.contains("/actions/") {
        data.and_then(|data| data.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("action")
            .to_string()
    } else if path.contains("/trades/") {
        "trade".to_string()
    } else if path.contains("/swaps/") {
        "swap".to_string()
    } else if path.ends_with("/my-selection") {
        "selection".to_string()
    } else {
        method.to_string()
    };
    let (session_id, config_revision) = state
        .automation
        .as_ref()
        .map(|automation| (automation.session_id(), automation.config_revision()))
        .unwrap_or(("", 0));
    WriteLedgerKey {
        pid: state
            .connection
            .as_ref()
            .map(|connection| connection.pid)
            .unwrap_or(0),
        epoch,
        session_id: session_id.to_string(),
        action_id,
        action_type,
        config_revision,
        operation_id: operation_id.to_string(),
    }
}

fn write_file_atomic(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let name = destination
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cache filename is missing"))?
        .to_string_lossy();
    let temporary = destination.with_file_name(format!(
        "{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let result =
        fs::write(&temporary, bytes).and_then(|_| atomic_replace_file(&temporary, destination));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

struct State {
    epoch: u64,
    started_at: Instant,
    metrics: Metrics,
    scheduler: ResourceScheduler,
    connection: Option<ConnectionInfo>,
    lcu: Option<LcuClient>,
    sgp: Option<SgpClient>,
    external: Option<ExternalClient>,
    opgg_cache: Arc<JsonCache>,
    asset: Option<AssetClient>,
    asset_cache: Arc<AssetCache>,
    ws_task: Option<tokio::task::JoinHandle<()>>,
    ws_shutdown: Option<mpsc::Sender<()>>,
    ws_event_rx_tx: mpsc::UnboundedSender<(u64, mpsc::Receiver<LcuEvent>)>,
    ws_event_rx_rx: Option<mpsc::UnboundedReceiver<(u64, mpsc::Receiver<LcuEvent>)>>,
    ws_queue_len: usize,
    ws_stats: Arc<WsStats>,
    sgp_token: Option<Zeroizing<String>>,
    sgp_token_superseded: Option<Zeroizing<String>>,
    sgp_refresh_lock: Arc<tokio::sync::Mutex<()>>,
    sgp_refresh_count: u64,
    cache_root: PathBuf,
    data_root: PathBuf,
    catalog_root: PathBuf,
    champion_aliases: Option<ChampionAliasCache>,
    external_proxy: Option<String>,
    automation: Option<DryRunAutomation>,
    write_ledger: HashMap<WriteLedgerKey, WriteLedgerEntry>,
    write_request_hashes: HashMap<(u32, String), [u8; 32]>,
    write_guards: HashMap<String, PersistedWriteGuard>,
    write_guard_path: PathBuf,
    write_guard_error: Option<&'static str>,
    catalog_champs: Arc<HashMap<String, String>>,
    catalog_augments: Arc<HashMap<String, String>>,
    catalog_paths: Arc<HashMap<String, String>>,
}

impl State {
    fn new() -> Self {
        let data_root = env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("Seraphine");
        let catalog_root = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("Seraphine");
        let write_guard_path = data_root.join("write-guards.json");
        let (write_guards, write_guard_error) = load_write_guards(&write_guard_path);
        let (ws_event_rx_tx, ws_event_rx_rx) = mpsc::unbounded_channel();
        Self {
            epoch: 0,
            started_at: Instant::now(),
            metrics: Metrics {
                started_at_ms: unix_millis(),
                ..Metrics::default()
            },
            scheduler: ResourceScheduler::new(),
            connection: None,
            lcu: None,
            sgp: None,
            external: None,
            opgg_cache: Arc::new(JsonCache::default()),
            asset: None,
            asset_cache: Arc::new(AssetCache::default()),
            ws_task: None,
            ws_shutdown: None,
            ws_event_rx_tx,
            ws_event_rx_rx: Some(ws_event_rx_rx),
            ws_queue_len: 0,
            ws_stats: Arc::new(WsStats::default()),
            sgp_token: None,
            sgp_token_superseded: None,
            sgp_refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            sgp_refresh_count: 0,
            cache_root: env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(env::temp_dir)
                .join("Seraphine")
                .join("Cache")
                .join("game"),
            data_root,
            catalog_root,
            champion_aliases: None,
            external_proxy: None,
            automation: None,
            write_ledger: HashMap::new(),
            write_request_hashes: HashMap::new(),
            write_guards,
            write_guard_path,
            write_guard_error,
            catalog_champs: Arc::new(HashMap::new()),
            catalog_augments: Arc::new(HashMap::new()),
            catalog_paths: Arc::new(HashMap::new()),
        }
    }

    fn cache_champion_aliases(&mut self, data: &Value) {
        self.champion_aliases = ChampionAliasCache::from_value(data);
    }

    fn restore_catalog_indexes(&mut self) {
        let path = self.catalog_root.join("catalog.json");
        let Ok(length) = fs::metadata(&path).map(|metadata| metadata.len()) else {
            return;
        };
        if length == 0 || length > 8 * 1024 * 1024 {
            return;
        }
        if let Ok(bytes) = fs::read(path) {
            if let Ok(snapshot) = serde_json::from_slice::<Value>(&bytes) {
                self.index_catalog(&snapshot);
            }
        }
    }

    fn touch_metrics(&mut self) {
        self.metrics.uptime_ms = self.started_at.elapsed().as_millis() as u64;
    }

    fn index_catalog(&mut self, snapshot: &Value) {
        self.catalog_champs = Arc::new(
            snapshot
                .get("champs")
                .and_then(Value::as_object)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|(id, name)| {
                            name.as_str().map(|name| (id.clone(), name.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        );
        self.catalog_augments = Arc::new(
            snapshot
                .get("cherryAugments")
                .and_then(Value::as_object)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|(id, item)| {
                            item.get("nameTRA")
                                .and_then(Value::as_str)
                                .map(|name| (id.clone(), name.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        );
        let mut paths = HashMap::new();
        for (kind, key, path_key) in [
            ("item", "items", "iconPath"),
            ("spell", "summonerSpells", "iconPath"),
            ("rune", "runes", "icon"),
            ("augment", "cherryAugments", "augmentSmallIconPath"),
        ] {
            if let Some(items) = snapshot.get(key).and_then(Value::as_object) {
                for (id, item) in items {
                    if let Some(path) = item.get(path_key).and_then(Value::as_str) {
                        paths.insert(format!("{kind}:{id}"), path.to_string());
                    }
                }
            }
        }
        self.catalog_paths = Arc::new(paths);
    }
}

#[derive(Clone)]
struct NetworkSnapshot {
    shared: Arc<Mutex<State>>,
    epoch: u64,
    connection: Option<ConnectionInfo>,
    scheduler: ResourceScheduler,
    lcu: Option<LcuClient>,
    sgp: Option<SgpClient>,
    sgp_token: Option<Zeroizing<String>>,
    sgp_refresh_lock: Arc<tokio::sync::Mutex<()>>,
    external: Option<ExternalClient>,
    asset: Option<AssetClient>,
    asset_cache: Arc<AssetCache>,
    opgg_cache: Arc<JsonCache>,
    cache_root: PathBuf,
    data_root: PathBuf,
    catalog_root: PathBuf,
    champion_aliases: Option<ChampionAliasCache>,
    catalog_champs: Arc<HashMap<String, String>>,
    catalog_augments: Arc<HashMap<String, String>>,
    catalog_paths: Arc<HashMap<String, String>>,
}

impl State {
    fn network_snapshot(&self, shared: Arc<Mutex<State>>) -> NetworkSnapshot {
        NetworkSnapshot {
            shared,
            epoch: self.epoch,
            connection: self.connection.clone(),
            scheduler: self.scheduler.clone(),
            lcu: self.lcu.clone(),
            sgp: self.sgp.clone(),
            sgp_token: self.sgp_token.clone(),
            sgp_refresh_lock: Arc::clone(&self.sgp_refresh_lock),
            external: self.external.clone(),
            asset: self.asset.clone(),
            asset_cache: Arc::clone(&self.asset_cache),
            opgg_cache: Arc::clone(&self.opgg_cache),
            cache_root: self.cache_root.clone(),
            data_root: self.data_root.clone(),
            catalog_root: self.catalog_root.clone(),
            champion_aliases: self.champion_aliases.clone(),
            catalog_champs: Arc::clone(&self.catalog_champs),
            catalog_augments: Arc::clone(&self.catalog_augments),
            catalog_paths: Arc::clone(&self.catalog_paths),
        }
    }
}

impl NetworkSnapshot {
    async fn external_client(&self) -> Result<ExternalClient, RpcError> {
        if let Some(client) = &self.external {
            return Ok(client.clone());
        }
        let mut state = self.shared.lock().await;
        if state.external.is_none() {
            state.external = Some(
                ExternalClient::new(state.external_proxy.as_deref())
                    .map_err(|error| RpcError::new("ConnectionError", error.to_string()))?,
            );
        }
        Ok(state.external.clone().unwrap())
    }

    async fn asset_client(&self) -> Result<AssetClient, RpcError> {
        if let Some(client) = &self.asset {
            return Ok(client.clone());
        }
        let mut state = self.shared.lock().await;
        if state.asset.is_none() {
            state.asset = Some(
                AssetClient::new(
                    state.external_proxy.as_deref(),
                    vec![
                        "raw.communitydragon.org".to_string(),
                        "lol-api-champion.op.gg".to_string(),
                        "game.gtimg.cn".to_string(),
                    ],
                )
                .map_err(|error| RpcError::new("AssetError", error.to_string()))?,
            );
        }
        Ok(state.asset.clone().unwrap())
    }

    async fn sgp_client(&self) -> Result<SgpClient, RpcError> {
        if self.shared.lock().await.epoch != self.epoch {
            return Err(RpcError::new("InvalidParams", "stale connection epoch"));
        }
        if let Some(client) = &self.sgp {
            return Ok(client.clone());
        }
        let connection = self
            .connection
            .clone()
            .ok_or_else(|| RpcError::new("NotConnected", "LCU is not connected"))?;
        let mut state = self.shared.lock().await;
        if state.epoch != self.epoch {
            return Err(RpcError::new("InvalidParams", "stale connection epoch"));
        }
        if state.sgp.is_none() {
            state.sgp = Some(
                SgpClient::new(&connection.server)
                    .map_err(|error| RpcError::new("ConnectionError", error.to_string()))?,
            );
        }
        Ok(state.sgp.clone().unwrap())
    }

    async fn refresh_sgp_token(
        &self,
        previous_token: &Zeroizing<String>,
    ) -> Result<Zeroizing<String>, RpcError> {
        let _refresh_guard = self.sgp_refresh_lock.lock().await;
        {
            let state = self.shared.lock().await;
            if state.epoch != self.epoch {
                return Err(RpcError::new("InvalidParams", "stale connection epoch"));
            }
            if state.sgp_token.as_ref() != Some(previous_token) {
                return state
                    .sgp_token
                    .clone()
                    .ok_or_else(|| RpcError::new("NotConnected", "SGP token is unavailable"));
            }
        }
        let client = self
            .lcu
            .clone()
            .ok_or_else(|| RpcError::new("NotConnected", "LCU is not connected"))?;
        let data = client
            .get_json_fresh("/entitlements/v1/token")
            .await
            .map_err(|error| RpcError::new("LcuError", error.to_string()))?;
        let token = parse_sgp_token(&data)
            .ok_or_else(|| RpcError::new("NotConnected", "SGP token is unavailable"))?;
        if token == previous_token.as_str() {
            return Err(RpcError::new(
                "SgpError",
                "SGP token did not change after refresh",
            ));
        }
        let mut state = self.shared.lock().await;
        if state.epoch != self.epoch {
            return Err(RpcError::new("InvalidParams", "stale connection epoch"));
        }
        if state.sgp_token.as_ref() == Some(previous_token) {
            state.sgp_token_superseded = Some(previous_token.clone());
            state.sgp_token = Some(Zeroizing::new(token.clone()));
            state.sgp_refresh_count = state.sgp_refresh_count.saturating_add(1);
        }
        state
            .sgp_token
            .clone()
            .ok_or_else(|| RpcError::new("NotConnected", "SGP token is unavailable"))
    }

    async fn sgp_get(&self, path: &str, params: Option<Value>) -> Result<Value, RpcError> {
        if self.shared.lock().await.epoch != self.epoch {
            return Err(RpcError::new("InvalidParams", "stale connection epoch"));
        }
        let mut current_token = self
            .sgp_token
            .clone()
            .ok_or_else(|| RpcError::new("NotConnected", "SGP token is unavailable"))?;
        let sgp = self.sgp_client().await?;
        let _permit = self
            .scheduler
            .try_acquire(Priority::P2, 128 * 1024)
            .map_err(|error| {
                RpcError::new("ResourceDeferred", format!("{error:?}")).retryable(true)
            })?;
        let mut retried = false;
        let mut refreshed = false;
        let result = loop {
            match sgp
                .get_json_once(path, current_token.as_str(), params.as_ref())
                .await
            {
                Ok(data) => break Ok(data),
                Err(LcuError::Unauthorized) if !refreshed => {
                    current_token = self.refresh_sgp_token(&current_token).await?;
                    refreshed = true;
                }
                Err(error) if !retried && should_retry_sgp(&error) => {
                    retried = true;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(error) => break Err(sgp_rpc_error(error)),
            }
        }?;
        if self.shared.lock().await.epoch != self.epoch {
            return Err(RpcError::new("InvalidParams", "stale connection epoch"));
        }
        Ok(result)
    }
}

struct WriteSnapshot {
    shared: Arc<Mutex<State>>,
    epoch: u64,
    working: State,
}

impl WriteSnapshot {
    fn new(shared: Arc<Mutex<State>>, state: &State) -> Self {
        let mut working = State::new();
        working.epoch = state.epoch;
        working.connection = state.connection.clone();
        working.lcu = state.lcu.clone();
        working.scheduler = state.scheduler.clone();
        working.automation = state.automation.clone();
        working.write_ledger = state.write_ledger.clone();
        working.write_request_hashes = state.write_request_hashes.clone();
        working.write_guards = state.write_guards.clone();
        working.write_guard_path = state.write_guard_path.clone();
        working.write_guard_error = state.write_guard_error;
        Self {
            shared,
            epoch: state.epoch,
            working,
        }
    }

    async fn commit(self) -> Result<(), RpcError> {
        let mut state = self.shared.lock().await;
        if state.epoch != self.epoch {
            return Err(RpcError::new("InvalidParams", "stale connection epoch"));
        }
        state.write_ledger = self.working.write_ledger;
        state.write_request_hashes = self.working.write_request_hashes;
        state.write_guards = self.working.write_guards;
        state.write_guard_error = self.working.write_guard_error;
        Ok(())
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

struct FrameReader<R> {
    reader: R,
    buffer: Vec<u8>,
    expected: Option<usize>,
}

impl<R> FrameReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            expected: None,
        }
    }
}

impl<R> FrameReader<R>
where
    R: AsyncRead + Unpin,
{
    async fn read_more(&mut self) -> io::Result<bool> {
        let mut chunk = [0_u8; 8192];
        match self.reader.read(&mut chunk).await {
            Ok(0) => Ok(false),
            Ok(length) => {
                self.buffer.extend_from_slice(&chunk[..length]);
                Ok(true)
            }
            Err(error) => Err(error),
        }
    }

    async fn next_message(&mut self) -> io::Result<Option<Message>> {
        if self.expected.is_none() {
            while self.buffer.len() < 4 {
                if !self.read_more().await? {
                    if self.buffer.is_empty() {
                        return Ok(None);
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "partial IPC frame prefix",
                    ));
                }
            }
            let prefix: Vec<u8> = self.buffer.drain(..4).collect();
            let length = u32::from_le_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]) as usize;
            let expected = decode_length_prefix(&prefix)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            if expected != Some(length) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid frame length",
                ));
            }
            self.expected = Some(length);
        }

        let length = self.expected.unwrap();
        while self.buffer.len() < length {
            if !self.read_more().await? {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "partial IPC frame payload",
                ));
            }
        }
        let payload: Vec<u8> = self.buffer.drain(..length).collect();
        self.expected = None;
        decode(&payload)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
    }
}

struct OutgoingFrame {
    frame: Vec<u8>,
    _permit: OwnedSemaphorePermit,
}

async fn queue_message(
    outgoing: &mpsc::Sender<OutgoingFrame>,
    byte_budget: &Arc<Semaphore>,
    message: Message,
) -> io::Result<()> {
    let frame = encode(&message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let permit = Arc::clone(byte_budget)
        .acquire_many_owned(frame.len() as u32)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "IPC writer closed"))?;
    outgoing
        .send(OutgoingFrame {
            frame,
            _permit: permit,
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "IPC writer closed"))
}

async fn writer_loop<W>(
    mut writer: W,
    mut outgoing: mpsc::Receiver<OutgoingFrame>,
) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while let Some(message) = outgoing.recv().await {
        writer.write_all(&message.frame).await?;
        writer.flush().await?;
    }
    Ok(())
}

fn memory_json() -> Value {
    match current_process_memory() {
        Ok(memory) => json!({
            "workingSetBytes": memory.working_set_bytes,
            "peakWorkingSetBytes": memory.peak_working_set_bytes,
            "privateUsageBytes": memory.private_usage_bytes,
            "peakPagefileUsageBytes": memory.peak_pagefile_usage_bytes,
        }),
        Err(_) => Value::Null,
    }
}

fn safe_cache_path(root: &Path, folder: &str, name: &str) -> Result<PathBuf, RpcError> {
    for value in [folder, name] {
        let path = Path::new(value);
        if value.is_empty()
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(RpcError::new("InvalidParams", "invalid cache path"));
        }
    }
    Ok(root.join(folder).join(name))
}

fn load_community_cache(path: &Path, champions_key: &str) -> Option<Value> {
    let length = fs::metadata(path).ok()?.len();
    if length == 0 || length > 4 * 1024 * 1024 {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let data = serde_json::from_slice(&bytes).ok()?;
    validate_cached(&data, champions_key).ok()?;
    Some(data)
}

fn store_community_cache(path: &Path, data: &Value) -> Result<(), RpcError> {
    let payload = serde_json::to_vec(data)
        .map_err(|error| RpcError::new("SerializeError", error.to_string()))?;
    write_file_atomic(path, &payload).map_err(|error| RpcError::new("IoError", error.to_string()))
}

fn opgg_params(message: &Message) -> Result<(String, String, String, String), RpcError> {
    let args = message
        .args
        .as_ref()
        .ok_or_else(|| RpcError::new("InvalidParams", "args are required"))?;
    let region = args.get("region").and_then(Value::as_str).unwrap_or("kr");
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or("ranked");
    let tier = args.get("tier").and_then(Value::as_str).unwrap_or("all");
    let position = args
        .get("position")
        .and_then(Value::as_str)
        .unwrap_or("none");
    if !matches!(region, "kr" | "global")
        || !matches!(
            mode,
            "ranked" | "aram" | "arena" | "urf" | "nexus_blitz" | "classic" | "aram_mayhem_classic"
        )
        || !matches!(
            tier,
            "all"
                | "ibsg"
                | "gold_plus"
                | "platinum_plus"
                | "emerald_plus"
                | "diamond_plus"
                | "master"
                | "master_plus"
                | "grandmaster"
                | "challenger"
        )
        || !matches!(
            position,
            "none" | "TOP" | "JUNGLE" | "MID" | "ADC" | "SUPPORT"
        )
    {
        return Err(RpcError::new("InvalidParams", "invalid OP.GG parameters"));
    }
    Ok((
        region.to_string(),
        mode.to_string(),
        tier.to_string(),
        position.to_string(),
    ))
}

async fn handle_request(message: &Message, state: &mut State) -> Message {
    state.metrics.requests += 1;
    let id = message.id.unwrap_or(0);
    let epoch = message.epoch.unwrap_or(state.epoch);
    let op = message.op.as_deref().unwrap_or_default();

    let result = match op {
        "health" => Ok(json!({
            "pid": std::process::id(),
            "epoch": state.epoch,
            "protocol": PROTOCOL_VERSION,
            "uptimeMs": state.started_at.elapsed().as_millis(),
            "memory": memory_json(),
            "connection": state.connection.as_ref().map(|connection| json!({
                "pid": connection.pid,
                "port": connection.port,
                "server": connection.server,
                "epoch": connection.epoch,
            })),
        })),
        "process.list" => match list_processes() {
            Ok(processes) => Ok(json!({
                "processes": processes,
                "leagueClientPids": list_league_client_pids().unwrap_or_default(),
            })),
            Err(error) => Err(RpcError::new("PlatformError", error.to_string()).retryable(true)),
        },
        "process.command_line" => {
            let Some(pid) = message
                .args
                .as_ref()
                .and_then(|args| args.get("pid"))
                .and_then(Value::as_u64)
            else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "pid is required"),
                );
            };
            match read_process_command_line(pid as u32) {
                Ok(command_line) => Ok(json!({
                    "pid": command_line.pid,
                    "port": command_line.port,
                    "server": command_line.server,
                    "hasToken": command_line.token.is_some(),
                })),
                Err(error) => Err(RpcError::new("PlatformError", error.to_string())),
            }
        }
        "lcu.summoner_by_pid" => {
            let Some(pid) = message
                .args
                .as_ref()
                .and_then(|args| args.get("pid"))
                .and_then(Value::as_u64)
            else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "pid is required"),
                );
            };
            let command_line = match read_process_command_line(pid as u32) {
                Ok(command_line) => command_line,
                Err(error) => {
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("PlatformError", error.to_string()),
                    )
                }
            };
            let (Some(port), Some(token)) = (command_line.port, command_line.token.as_ref()) else {
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "LCU credentials not found"),
                );
            };
            let client = match LcuClient::new(port, token.as_str()) {
                Ok(client) => client,
                Err(error) => {
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("ConnectionError", error.to_string()),
                    )
                }
            };
            let permit = match state.scheduler.try_acquire(Priority::P2, 64 * 1024) {
                Ok(permit) => permit,
                Err(error) => {
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("ResourceDeferred", format!("{error:?}")).retryable(true),
                    )
                }
            };
            match client.get_json("/lol-summoner/v1/current-summoner").await {
                Ok(data) => {
                    drop(permit);
                    Ok(data)
                }
                Err(error) => {
                    drop(permit);
                    Err(RpcError::new("LcuError", error.to_string()).retryable(true))
                }
            }
        }
        "scheduler.set_game_state" => {
            let Some(game_state) = message
                .args
                .as_ref()
                .and_then(|args| args.get("state"))
                .and_then(Value::as_str)
            else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "state is required"),
                );
            };
            let game_state = match game_state {
                "not_running" => Some(GameState::NotRunning),
                "client_starting" => Some(GameState::ClientStarting),
                "lobby" => Some(GameState::Lobby),
                "champ_select" => Some(GameState::ChampSelect),
                "in_game" => Some(GameState::InGame),
                "end_of_game" => Some(GameState::EndOfGame),
                "reconnect" => Some(GameState::Reconnect),
                _ => None,
            };
            let Some(game_state) = game_state else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "unknown game state"),
                );
            };
            state.scheduler.set_game_state(game_state);
            Ok(json!({ "scheduler": state.scheduler.snapshot() }))
        }
        "lcu.connect" => {
            let Some(pid) = message
                .args
                .as_ref()
                .and_then(|args| args.get("pid"))
                .and_then(Value::as_u64)
            else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "pid is required"),
                );
            };
            let command_line = match read_process_command_line(pid as u32) {
                Ok(command_line) => command_line,
                Err(error) => {
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("PlatformError", error.to_string()),
                    );
                }
            };
            let Some(port) = command_line.port else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "LCU port not found"),
                );
            };
            let Some(token) = command_line.token.as_ref() else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "LCU token not found"),
                );
            };
            let server = command_line.server.unwrap_or_default();
            if let Some(cache_root) = message
                .args
                .as_ref()
                .and_then(|args| args.get("cacheRoot"))
                .and_then(Value::as_str)
            {
                state.cache_root = PathBuf::from(cache_root);
            }
            state.external_proxy = message
                .args
                .as_ref()
                .and_then(|args| args.get("proxy"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let lcu = match LcuClient::new(port, token.as_str()) {
                Ok(client) => client,
                Err(error) => {
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("ConnectionError", error.to_string()),
                    );
                }
            };
            state.sgp_token = lcu
                .get_json("/entitlements/v1/token")
                .await
                .ok()
                .and_then(|data| parse_sgp_token(&data))
                .map(Zeroizing::new);
            state.sgp_token_superseded = None;
            state.epoch = state.epoch.wrapping_add(1);
            state.write_ledger.clear();
            let connection = ConnectionInfo {
                pid: pid as u32,
                port,
                server,
                epoch: state.epoch,
            };
            state.lcu = Some(lcu);
            state.sgp = None;
            state.external = None;
            state.asset = None;
            state.asset_cache.clear_failures();
            if let Some(shutdown) = state.ws_shutdown.take() {
                let _ = shutdown.send(()).await;
            }
            if let Some(task) = state.ws_task.take() {
                task.abort();
            }
            let (event_tx, event_rx) = mpsc::channel(MAX_EVENT_QUEUE);
            let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
            let stats = Arc::new(WsStats::default());
            let topics = vec![
                GAMEFLOW_TOPIC.to_string(),
                CHAMP_SELECT_TOPIC.to_string(),
                SUMMONER_TOPIC.to_string(),
                ENTITLEMENT_TOPIC.to_string(),
            ];
            state.ws_task = Some(tokio::spawn(run_websocket(
                port,
                (*token).clone(),
                state.lcu.clone().unwrap(),
                topics,
                event_tx,
                shutdown_rx,
                state.scheduler.clone(),
                Arc::clone(&stats),
            )));
            state.ws_shutdown = Some(shutdown_tx);
            let _ = state.ws_event_rx_tx.send((connection.epoch, event_rx));
            state.ws_stats = stats;
            state.connection = Some(connection.clone());
            state.restore_catalog_indexes();
            state.scheduler.set_game_state(GameState::ClientStarting);
            Ok(json!({
                "pid": connection.pid,
                "port": connection.port,
                "server": connection.server,
                "epoch": connection.epoch,
            }))
        }
        "lcu.get" => {
            let Some(path) = message
                .args
                .as_ref()
                .and_then(|args| args.get("path"))
                .and_then(Value::as_str)
            else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "path is required"),
                );
            };
            if !path.starts_with('/') || path.contains("://") {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "only LCU paths are allowed"),
                );
            }
            let Some(client) = state.lcu.clone() else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("NotConnected", "LCU is not connected"),
                );
            };
            let permit = match state.scheduler.try_acquire(Priority::P1, 64 * 1024) {
                Ok(permit) => permit,
                Err(error) => {
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("ResourceDeferred", format!("{error:?}")).retryable(true),
                    );
                }
            };
            match client.get_json(path).await {
                Ok(data) => {
                    drop(permit);
                    Ok(data)
                }
                Err(error) => {
                    drop(permit);
                    Err(RpcError::new("LcuError", error.to_string()).retryable(true))
                }
            }
        }
        "external.get" => {
            let Some(url) = message
                .args
                .as_ref()
                .and_then(|args| args.get("url"))
                .and_then(Value::as_str)
            else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "url is required"),
                );
            };
            let parsed = match reqwest::Url::parse(url) {
                Ok(url) => url,
                Err(error) => {
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("InvalidParams", error.to_string()),
                    );
                }
            };
            let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
            let allowed = [
                "lol-api-champion.op.gg",
                "raw.communitydragon.org",
                "game.gtimg.cn",
            ];
            if parsed.scheme() != "https"
                || !allowed
                    .iter()
                    .any(|item| host == *item || host.ends_with(&format!(".{item}")))
            {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "external host is not allowed"),
                );
            }
            if state.external.is_none() {
                match ExternalClient::new(state.external_proxy.as_deref()) {
                    Ok(client) => state.external = Some(client),
                    Err(error) => {
                        state.metrics.errors += 1;
                        return Message::response_error(
                            id,
                            epoch,
                            RpcError::new("ConnectionError", error.to_string()),
                        );
                    }
                }
            }
            let permit = match state.scheduler.try_acquire(Priority::P2, 512 * 1024) {
                Ok(permit) => permit,
                Err(error) => {
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("ResourceDeferred", format!("{error:?}")).retryable(true),
                    );
                }
            };
            match state.external.as_ref().unwrap().get_json(url).await {
                Ok(data) => {
                    drop(permit);
                    Ok(data)
                }
                Err(error) => {
                    drop(permit);
                    Err(RpcError::new("ExternalError", error.to_string()).retryable(true))
                }
            }
        }
        "automation.configure" => {
            if message.epoch != Some(state.epoch) {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "stale automation epoch"),
                );
            }
            let args = message.args.clone().unwrap_or(Value::Null);
            let Some(session_id) = args.get("sessionId").and_then(Value::as_str) else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "sessionId is required"),
                );
            };
            if session_id.is_empty() || session_id.len() > 128 {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "invalid sessionId"),
                );
            }
            let config = args.get("config").cloned().unwrap_or(Value::Null);
            let replace = state
                .automation
                .as_ref()
                .map(|automation| automation.epoch() != state.epoch)
                .unwrap_or(true);
            if replace {
                state.automation = Some(DryRunAutomation::new(state.epoch, session_id, config));
            } else if let Some(automation) = state.automation.as_mut() {
                automation.reset_session(session_id);
                automation.update_config(config);
            }
            if let Some(queue_id) = args.get("queueId").and_then(Value::as_u64) {
                if let Some(automation) = state.automation.as_mut() {
                    automation.set_queue_id(Some(queue_id));
                }
            }
            Ok(json!({ "configured": true }))
        }
        "automation.dry_run" => {
            if message.epoch != Some(state.epoch) {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "stale automation epoch"),
                );
            }
            let Some(automation) = state.automation.as_mut() else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "automation is not configured"),
                );
            };
            let event = message.args.clone().unwrap_or(Value::Null);
            let plans = automation.handle_event(&event);
            Ok(json!({
                "plans": plans,
                "snapshot": automation.snapshot(),
            }))
        }
        "aram_buff.refresh" => {
            let data = load_community_cache(&state.data_root.join("AramBuff.json"), "champions");
            match data {
                Some(data) => Ok(json!({
                    "available": true,
                    "updated": false,
                    "data": data,
                    "warning": Value::Null
                })),
                None => Ok(json!({
                    "available": false,
                    "updated": false,
                    "data": Value::Null,
                    "warning": "ARAM data is not installed; run the data update"
                })),
            }
        }
        "champion_alias.search" => {
            if state.champion_aliases.is_none() {
                let cached =
                    load_community_cache(&state.data_root.join("ChampionAlias.json"), "champions");
                state.champion_aliases = cached.as_ref().and_then(ChampionAliasCache::from_value);
            }
            let Some(data) = state.champion_aliases.as_ref() else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("NotFound", "champion aliases are unavailable"),
                );
            };
            let query = message
                .args
                .as_ref()
                .and_then(|args| args.get("query"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            let mut ids = data
                .champions
                .iter()
                .filter(|(_, aliases)| query.is_empty() || aliases.to_lowercase().contains(&query))
                .filter_map(|(id, _)| id.parse::<u64>().ok())
                .collect::<Vec<_>>();
            ids.sort_unstable();
            Ok(json!(ids))
        }
        "aram_buff.get" => {
            let Some(champion_id) = message
                .args
                .as_ref()
                .and_then(|args| args.get("championId"))
                .and_then(Value::as_u64)
            else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "championId is required"),
                );
            };
            let data = load_community_cache(&state.data_root.join("AramBuff.json"), "champions");
            Ok(json!({
                "available": data.is_some(),
                "version": data.as_ref().and_then(|data| data.get("version")).cloned(),
                "info": data.as_ref().and_then(|data| aram_info(data, champion_id)),
            }))
        }
        "diagnostics" => {
            state.touch_metrics();
            let uptime_seconds = (state.metrics.uptime_ms as f64 / 1000.0).max(0.001);
            let requests_per_second = state.metrics.requests as f64 / uptime_seconds;
            let error_rate = if state.metrics.requests == 0 {
                0.0
            } else {
                state.metrics.errors as f64 / state.metrics.requests as f64
            };
            if let Some(cache) = state.lcu.as_ref().map(LcuClient::cache_metrics) {
                state.metrics.cache_hits = cache.hits;
                state.metrics.cache_coalesced = cache.coalesced;
            }
            let opgg_cache = state.opgg_cache.metrics();
            let asset_cache = state.asset_cache.metrics();
            Ok(json!({
                "metrics": state.metrics,
                "opggCache": {
                    "hits": opgg_cache.hits,
                    "coalesced": opgg_cache.coalesced,
                },
                "assetCache": {
                    "inflight": asset_cache.inflight,
                    "negativeEntries": asset_cache.negative_entries,
                    "coalesced": asset_cache.coalesced,
                    "negativeHits": asset_cache.negative_hits,
                    "failures": asset_cache.failures,
                },
                "rates": {
                    "requestsPerSecond": requests_per_second,
                    "errorRate": error_rate,
                },
                "latencyUs": {
                    "p50": state.metrics.percentile_us(0.50),
                    "p95": state.metrics.percentile_us(0.95),
                    "p99": state.metrics.percentile_us(0.99),
                    "max": state.metrics.latency_max_us,
                    "mean": state.metrics.latency_sum_us.checked_div(state.metrics.requests).unwrap_or(0),
                },
                "memory": memory_json(),
                "capabilities": active_capabilities(),
                "scheduler": state.scheduler.snapshot(),
                "writes": json!({
                    "enabled": active_capabilities().contains(&"lcu.write"),
                    "ledgerEntries": state.write_ledger.len(),
                    "persistedGuards": state.write_guards.len(),
                    "persistenceError": state.write_guard_error,
                }),
                "clients": json!({
                    "lcu": state.lcu.is_some(),
                    "sgp": state.sgp.is_some(),
                    "external": state.external.is_some(),
                    "asset": state.asset.is_some(),
                }),
                "sgpRefreshCount": state.sgp_refresh_count,
                "websocket": {
                    "received": state.ws_stats.received.load(Ordering::Relaxed),
                    "dropped": state.ws_stats.dropped.load(Ordering::Relaxed),
                    "reconnects": state.ws_stats.reconnects.load(Ordering::Relaxed),
                    "snapshots": state.ws_stats.snapshots.load(Ordering::Relaxed),
                    "queued": state.ws_queue_len,
                },
            }))
        }
        "echo" => Ok(message.args.clone().unwrap_or(Value::Null)),
        _ => Err(RpcError::new(
            "MethodNotFound",
            format!("unknown operation: {op}"),
        )),
    };

    match result {
        Ok(data) => Message::response(id, epoch, data),
        Err(error) => {
            state.metrics.errors += 1;
            Message::response_error(id, epoch, error)
        }
    }
}

fn is_snapshot_network_op(op: &str) -> bool {
    matches!(
        op,
        "asset.get"
            | "opgg.positions"
            | "opgg.tier_list"
            | "opgg.champion_build"
            | "lcu.catalog"
            | "champion_alias.refresh"
            | "sgp.get"
    )
}

async fn handle_snapshot_network_request(message: &Message, shared: Arc<Mutex<State>>) -> Message {
    let id = message.id.unwrap_or(0);
    let (epoch, mut snapshot) = {
        let mut state = shared.lock().await;
        state.metrics.requests += 1;
        let epoch = message.epoch.unwrap_or(state.epoch);
        (epoch, state.network_snapshot(shared.clone()))
    };
    let result: Result<Value, RpcError> = async {
        match message.op.as_deref().unwrap_or_default() {
        "asset.get" => {
            let args = message.args.clone().unwrap_or(Value::Null);
            let folder = args
                .get("folder")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
            let destination = safe_cache_path(&snapshot.cache_root, folder, name)?;
            let max_bytes = usize::try_from(
                args.get("maxBytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(2 * 1024 * 1024),
            )
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| RpcError::new("InvalidParams", "invalid asset byte budget"))?;
            let source = if let Some(path) = args.get("path").and_then(Value::as_str) {
                if !path.starts_with('/') || path.contains("://") {
                    Err(RpcError::new(
                        "InvalidParams",
                        "only LCU asset paths are allowed",
                    ))
                } else {
                    Ok(AssetSource::Lcu(path.to_string()))
                }
            } else if let Some(url) = args.get("url").and_then(Value::as_str) {
                Ok(AssetSource::External(url.to_string()))
            } else {
                Err(RpcError::new("InvalidParams", "path or url is required"))
            };
            let source = source?;
            let priority = if args.get("force").and_then(Value::as_bool).unwrap_or(false) {
                Priority::P2
            } else {
                Priority::P3
            };
            let path = snapshot
                .cached_asset(destination, source, max_bytes, priority)
                .await?;
            Ok(json!({ "path": path }))
        }
        "opgg.positions" => {
            let (region, mode, tier, _) = opgg_params(message)?;
            let champion_id = message
                .args
                .as_ref()
                .and_then(|args| args.get("championId"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let url =
                format!("https://lol-api-champion.op.gg/api/{region}/champions/{mode}?tier={tier}");
            let raw = snapshot.fetch_opgg(url).await?;
            let positions = raw
                .get("data")
                .and_then(Value::as_array)
                .unwrap_or(&Vec::new())
                .iter()
                .find(|item| item.get("id").and_then(Value::as_u64) == Some(champion_id))
                .map(|item| {
                    item.get("positions")
                        .and_then(Value::as_array)
                        .unwrap_or(&Vec::new())
                        .iter()
                        .filter_map(|position| position.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(json!(positions))
        }
        "opgg.tier_list" => {
            let (region, mode, tier, _) = opgg_params(message)?;
            let url =
                format!("https://lol-api-champion.op.gg/api/{region}/champions/{mode}?tier={tier}");
            let raw = snapshot.fetch_opgg(url).await?;
            let data = if mode == "ranked" {
                parse_ranked_tier_list(&raw)
            } else {
                parse_other_tier_list(&raw)
            };
            Ok(json!({
                "data": data,
                "version": raw.get("meta").and_then(|meta| meta.get("version")).cloned().unwrap_or(Value::Null),
            }))
        }
        "opgg.champion_build" => {
            let (region, mode, tier, mut position) = opgg_params(message)?;
            let champion_id = message
                .args
                .as_ref()
                .and_then(|args| args.get("championId"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let tier_url =
                format!("https://lol-api-champion.op.gg/api/{region}/champions/{mode}?tier={tier}");
            let tier_raw = snapshot.fetch_opgg(tier_url).await?;
            let positions = tier_raw
                .get("data")
                .and_then(Value::as_array)
                .unwrap_or(&Vec::new())
                .iter()
                .find(|item| item.get("id").and_then(Value::as_u64) == Some(champion_id))
                .map(|item| {
                    item.get("positions")
                        .and_then(Value::as_array)
                        .unwrap_or(&Vec::new())
                        .iter()
                        .filter_map(|position| position.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !positions.contains(&position)
                && !positions.is_empty()
                && matches!(mode.as_str(), "ranked" | "classic")
            {
                position = positions[0].clone();
            }
            let url = if mode == "arena" {
                format!(
                    "https://lol-api-champion.op.gg/api/{region}/champions/{mode}/{champion_id}?tier={tier}"
                )
            } else {
                format!(
                    "https://lol-api-champion.op.gg/api/{region}/champions/{mode}/{champion_id}/{position}?tier={tier}"
                )
            };
            let raw = snapshot.fetch_opgg(url).await?;
            let mut data = if mode == "arena" {
                parse_arena_champion_build(&raw)
            } else {
                parse_other_champion_build(&raw, &position)
            };
            snapshot.resolve_opgg_icons(&mut data).await;
            Ok(json!({
                "data": data,
                "version": raw.get("meta").and_then(|meta| meta.get("version")).cloned().unwrap_or(Value::Null),
                "mode": mode,
            }))
        }
        "lcu.catalog" => {
            let force = message
                .args
                .as_ref()
                .and_then(|args| args.get("force"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            snapshot.build_catalog(force).await
        }
        "champion_alias.refresh" => {
            let force = message
                .args
                .as_ref()
                .and_then(|args| args.get("force"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let game_version = message
                .args
                .as_ref()
                .and_then(|args| args.get("gameVersion"))
                .and_then(Value::as_str);
            snapshot.refresh_champion_alias(force, game_version).await
        }
        "sgp.get" => {
            let path = message
                .args
                .as_ref()
                .and_then(|args| args.get("path"))
                .and_then(Value::as_str)
                .ok_or_else(|| RpcError::new("InvalidParams", "path is required"))?;
            if !path.starts_with('/') || path.contains("://") {
                Err(RpcError::new("InvalidParams", "only SGP paths are allowed"))
            } else {
                let params = message
                    .args
                    .as_ref()
                    .and_then(|args| args.get("params"))
                    .cloned();
                snapshot.sgp_get(path, params).await
            }
        }
        _ => Err(RpcError::new(
            "MethodNotFound",
            format!(
                "unknown operation: {}",
                message.op.as_deref().unwrap_or_default()
            ),
        )),
        }
    }
    .await;
    if result.is_err() {
        shared.lock().await.metrics.errors += 1;
    }
    match result {
        Ok(data) => Message::response(id, epoch, data),
        Err(error) => Message::response_error(id, epoch, error),
    }
}

async fn handle_write_request(message: &Message, shared: Arc<Mutex<State>>) -> Message {
    let id = message.id.unwrap_or(0);
    let epoch = message.epoch.unwrap_or(0);
    shared.lock().await.metrics.requests += 1;
    let result: Result<Value, RpcError> = async {
        if !writes_enabled() {
            return Err(RpcError::new("WriteDisabled", "LCU writes are disabled"));
        }
        let current_epoch = shared.lock().await.epoch;
        if message.epoch != Some(current_epoch) {
            return Err(RpcError::new("InvalidParams", "stale connection epoch"));
        }
        let args = message.args.clone().unwrap_or(Value::Null);
        let method = args
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_uppercase();
        let path = args.get("path").and_then(Value::as_str).unwrap_or_default();
        let operation_id = args
            .get("operationId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let data = args.get("data").cloned();
        let lowercase_path = path.to_ascii_lowercase();
        if !matches!(method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE")
            || !path.starts_with('/')
            || path.len() <= 1
            || path == "/lol-champ-select/v1/session"
            || path.ends_with('/')
            || path.contains("://")
            || path.contains('?')
            || path.contains('#')
            || path.split('/').any(|segment| matches!(segment, "." | ".."))
            || lowercase_path.contains("%2e")
            || lowercase_path.contains("%2f")
            || lowercase_path.contains("%5c")
            || path.contains('\r')
            || path.contains('\n')
            || path.len() > 2048
            || operation_id.is_empty()
            || operation_id.len() > 128
        {
            return Err(RpcError::new(
                "InvalidParams",
                "invalid LCU write parameters",
            ));
        }
        if let Some(body) = data.as_ref() {
            if serde_json::to_vec(body)
                .map(|bytes| bytes.len() > 256 * 1024)
                .unwrap_or(true)
            {
                return Err(RpcError::new(
                    "InvalidParams",
                    "LCU write body is too large",
                ));
            }
        }
        let mut snapshot = {
            let state = shared.lock().await;
            WriteSnapshot::new(shared.clone(), &state)
        };
        let result = write_lcu(
            &mut snapshot.working,
            current_epoch,
            &method,
            path,
            operation_id,
            data,
        )
        .await;
        if let Err(error) = snapshot.commit().await {
            return Err(RpcError::new(
                "OutcomeUnknown",
                format!(
                    "connection changed while write was in flight: {}",
                    error.message
                ),
            ));
        }
        result
    }
    .await;
    if result.is_err() {
        shared.lock().await.metrics.errors += 1;
    }
    match result {
        Ok(data) => Message::response(id, epoch, data),
        Err(error) => Message::response_error(id, epoch, error),
    }
}

fn is_concurrent_read_op(op: &str) -> bool {
    matches!(
        op,
        "health"
            | "process.list"
            | "process.command_line"
            | "lcu.summoner_by_pid"
            | "lcu.get"
            | "external.get"
            | "aram_buff.get"
            | "champion_alias.search"
            | "echo"
    )
}

async fn handle_concurrent_read_request(message: &Message, shared: &Arc<Mutex<State>>) -> Message {
    let id = message.id.unwrap_or(0);
    let op = message.op.as_deref().unwrap_or_default();
    let epoch = {
        let mut state = shared.lock().await;
        state.metrics.requests += 1;
        message.epoch.unwrap_or(state.epoch)
    };
    let result = match op {
        "health" => {
            let state = shared.lock().await;
            Ok(json!({
                "pid": std::process::id(),
                "epoch": state.epoch,
                "protocol": PROTOCOL_VERSION,
                "uptimeMs": state.started_at.elapsed().as_millis(),
                "memory": memory_json(),
                "connection": state.connection.as_ref().map(|connection| json!({
                    "pid": connection.pid,
                    "port": connection.port,
                    "server": connection.server,
                    "epoch": connection.epoch,
                })),
            }))
        }
        "process.list" => match list_processes() {
            Ok(processes) => Ok(json!({
                "processes": processes,
                "leagueClientPids": list_league_client_pids().unwrap_or_default(),
            })),
            Err(error) => Err(RpcError::new("PlatformError", error.to_string()).retryable(true)),
        },
        "process.command_line" => {
            let Some(pid) = message
                .args
                .as_ref()
                .and_then(|args| args.get("pid"))
                .and_then(Value::as_u64)
            else {
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "pid is required"),
                );
            };
            match read_process_command_line(pid as u32) {
                Ok(command_line) => Ok(json!({
                    "pid": command_line.pid,
                    "port": command_line.port,
                    "server": command_line.server,
                    "hasToken": command_line.token.is_some(),
                })),
                Err(error) => Err(RpcError::new("PlatformError", error.to_string())),
            }
        }
        "lcu.summoner_by_pid" => {
            let Some(pid) = message
                .args
                .as_ref()
                .and_then(|args| args.get("pid"))
                .and_then(Value::as_u64)
            else {
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "pid is required"),
                );
            };
            let scheduler = shared.lock().await.scheduler.clone();
            async {
                let command_line = read_process_command_line(pid as u32)
                    .map_err(|error| RpcError::new("PlatformError", error.to_string()))?;
                let (Some(port), Some(token)) = (command_line.port, command_line.token.as_ref())
                else {
                    return Err(RpcError::new("InvalidParams", "LCU credentials not found"));
                };
                let client = LcuClient::new(port, token.as_str())
                    .map_err(|error| RpcError::new("ConnectionError", error.to_string()))?;
                let _permit = scheduler
                    .try_acquire(Priority::P2, 64 * 1024)
                    .map_err(|error| {
                        RpcError::new("ResourceDeferred", format!("{error:?}")).retryable(true)
                    })?;
                client
                    .get_json("/lol-summoner/v1/current-summoner")
                    .await
                    .map_err(|error| RpcError::new("LcuError", error.to_string()).retryable(true))
            }
            .await
        }
        "lcu.get" => {
            let Some(path) = message
                .args
                .as_ref()
                .and_then(|args| args.get("path"))
                .and_then(Value::as_str)
            else {
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "path is required"),
                );
            };
            if !path.starts_with('/') || path.contains("://") {
                Err(RpcError::new("InvalidParams", "only LCU paths are allowed"))
            } else {
                async {
                    let (client, scheduler, current_epoch) = {
                        let state = shared.lock().await;
                        (state.lcu.clone(), state.scheduler.clone(), state.epoch)
                    };
                    if current_epoch != epoch {
                        return Err(RpcError::new("InvalidParams", "stale connection epoch"));
                    }
                    let client = client
                        .ok_or_else(|| RpcError::new("NotConnected", "LCU is not connected"))?;
                    let _permit =
                        scheduler
                            .try_acquire(Priority::P1, 64 * 1024)
                            .map_err(|error| {
                                RpcError::new("ResourceDeferred", format!("{error:?}"))
                                    .retryable(true)
                            })?;
                    client.get_json(path).await.map_err(|error| {
                        RpcError::new("LcuError", error.to_string()).retryable(true)
                    })
                }
                .await
            }
        }
        "external.get" => {
            let Some(url) = message
                .args
                .as_ref()
                .and_then(|args| args.get("url"))
                .and_then(Value::as_str)
            else {
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "url is required"),
                );
            };
            let parsed = match reqwest::Url::parse(url) {
                Ok(url) => url,
                Err(error) => {
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("InvalidParams", error.to_string()),
                    )
                }
            };
            let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
            let allowed = [
                "lol-api-champion.op.gg",
                "raw.communitydragon.org",
                "game.gtimg.cn",
            ];
            if parsed.scheme() != "https"
                || !allowed
                    .iter()
                    .any(|item| host == *item || host.ends_with(&format!(".{item}")))
            {
                Err(RpcError::new(
                    "InvalidParams",
                    "external host is not allowed",
                ))
            } else {
                async {
                    let (client, scheduler) = {
                        let mut state = shared.lock().await;
                        if state.external.is_none() {
                            state.external = Some(
                                ExternalClient::new(state.external_proxy.as_deref()).map_err(
                                    |error| RpcError::new("ConnectionError", error.to_string()),
                                )?,
                            );
                        }
                        (state.external.clone().unwrap(), state.scheduler.clone())
                    };
                    let _permit =
                        scheduler
                            .try_acquire(Priority::P2, 512 * 1024)
                            .map_err(|error| {
                                RpcError::new("ResourceDeferred", format!("{error:?}"))
                                    .retryable(true)
                            })?;
                    client.get_json(url).await.map_err(|error| {
                        RpcError::new("ExternalError", error.to_string()).retryable(true)
                    })
                }
                .await
            }
        }
        "aram_buff.get" => {
            let Some(champion_id) = message
                .args
                .as_ref()
                .and_then(|args| args.get("championId"))
                .and_then(Value::as_u64)
            else {
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "championId is required"),
                );
            };
            let path = shared.lock().await.data_root.join("AramBuff.json");
            let data = load_community_cache(&path, "champions");
            Ok(json!({
                "available": data.is_some(),
                "version": data.as_ref().and_then(|data| data.get("version")).cloned(),
                "info": data.as_ref().and_then(|data| aram_info(data, champion_id)),
            }))
        }
        "champion_alias.search" => {
            let aliases = {
                let mut state = shared.lock().await;
                if state.champion_aliases.is_none() {
                    let cached = load_community_cache(
                        &state.data_root.join("ChampionAlias.json"),
                        "champions",
                    );
                    state.champion_aliases =
                        cached.as_ref().and_then(ChampionAliasCache::from_value);
                }
                state.champion_aliases.clone()
            };
            let data = match aliases {
                Some(data) => data,
                None => {
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("NotFound", "champion aliases are unavailable"),
                    )
                }
            };
            let query = message
                .args
                .as_ref()
                .and_then(|args| args.get("query"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            let mut ids = data
                .champions
                .iter()
                .filter(|(_, aliases)| query.is_empty() || aliases.to_lowercase().contains(&query))
                .filter_map(|(id, _)| id.parse::<u64>().ok())
                .collect::<Vec<_>>();
            ids.sort_unstable();
            Ok(json!(ids))
        }
        "echo" => Ok(message.args.clone().unwrap_or(Value::Null)),
        _ => Err(RpcError::new(
            "MethodNotFound",
            format!("unknown operation: {op}"),
        )),
    };
    match result {
        Ok(data) => Message::response(id, epoch, data),
        Err(error) => {
            shared.lock().await.metrics.errors += 1;
            Message::response_error(id, epoch, error)
        }
    }
}

fn parse_sgp_token(data: &Value) -> Option<String> {
    data.get("accessToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

fn apply_sgp_token_event(state: &mut State, token: String) {
    if state.sgp_token_superseded.as_deref() == Some(&token) {
        return;
    }
    state.sgp_token = Some(Zeroizing::new(token));
    state.sgp_token_superseded = None;
}

fn sgp_rpc_error(error: LcuError) -> RpcError {
    RpcError::new("SgpError", error.to_string()).retryable(should_retry_sgp(&error))
}

fn requires_champ_select_guard(path: &str) -> bool {
    path.starts_with("/lol-champ-select/v1/session/")
        && (path.contains("/actions/")
            || path.ends_with("/my-selection")
            || ((path.contains("/trades/") || path.contains("/swaps/"))
                && path.ends_with("/accept")))
}

async fn read_write_confirmation(
    state: &State,
    client: &LcuClient,
    path: &str,
    data: Option<&Value>,
) -> Result<(), RpcError> {
    let read_permit = state
        .scheduler
        .try_acquire(Priority::P1, 64 * 1024)
        .map_err(|error| RpcError::new("ResourceDeferred", format!("{error:?}")))?;
    let confirmation = client
        .get_json_fresh("/lol-champ-select/v1/session")
        .await
        .map_err(|error| RpcError::new("LcuError", error.to_string()))
        .and_then(|session| {
            confirm_write(&session, path, data)
                .map_err(|error| RpcError::new("InvalidState", error.as_str()))
        });
    drop(read_permit);
    confirmation
}

fn finish_write(
    ledger: &mut HashMap<WriteLedgerKey, WriteLedgerEntry>,
    key: WriteLedgerKey,
    request_hash: [u8; 32],
    confirmation: Result<(), RpcError>,
    response_data: Value,
    write_error: Option<&str>,
) -> Result<Value, RpcError> {
    match confirmation {
        Ok(()) => {
            ledger.insert(
                key,
                WriteLedgerEntry {
                    request_hash,
                    state: AutomationState::Completed,
                },
            );
            let mut result = json!({
                "replayed": false,
                "state": "completed",
                "data": response_data,
            });
            if write_error.is_some() {
                result["confirmedAfterError"] = Value::Bool(true);
            }
            Ok(result)
        }
        Err(error) => {
            ledger.insert(
                key,
                WriteLedgerEntry {
                    request_hash,
                    state: AutomationState::OutcomeUnknown,
                },
            );
            let context = write_error
                .map(|write_error| {
                    format!(
                        "write returned error ({write_error}); confirmation failed: {}",
                        error.message
                    )
                })
                .unwrap_or_else(|| format!("write needs inspection: {}", error.message));
            Err(RpcError::new("OutcomeUnknown", context))
        }
    }
}

fn ensure_write_capacity(state: &mut State) -> Result<(), RpcError> {
    prune_expired_completed_guards(&mut state.write_guards, unix_millis());
    if state.write_guards.len() >= MAX_PERSISTED_WRITE_GUARDS {
        return Err(RpcError::new(
            "ResourceDeferred",
            "persistent write guard capacity reached",
        )
        .retryable(true));
    }
    if state.write_ledger.len() >= 256 || state.write_request_hashes.len() >= 512 {
        let completed = state
            .write_ledger
            .iter()
            .find(|(_, entry)| entry.state == AutomationState::Completed)
            .map(|(key, _)| key.clone());
        if let Some(key) = completed {
            state.write_ledger.remove(&key);
            state
                .write_request_hashes
                .remove(&(key.pid, key.operation_id.clone()));
        }
    }
    if state.write_ledger.len() < 256 && state.write_request_hashes.len() < 512 {
        Ok(())
    } else {
        Err(RpcError::new("ResourceDeferred", "write ledger is full").retryable(true))
    }
}

async fn write_lcu(
    state: &mut State,
    epoch: u64,
    method: &str,
    path: &str,
    operation_id: &str,
    data: Option<Value>,
) -> Result<Value, RpcError> {
    let key = write_ledger_key(state, epoch, operation_id, method, path, data.as_ref());
    let request_hash = write_request_hash(method, path, data.as_ref());
    let operation_scope = (key.pid, key.operation_id.clone());
    let guard_key = operation_guard_key(key.pid, &key.operation_id);
    if let Some(guard) = state.write_guards.get(&guard_key) {
        if guard.request_hash != request_hash {
            return Err(RpcError::new(
                "OperationConflict",
                "operationId was already used for a different write request",
            ));
        }
        if guard.completed {
            return Ok(json!({
                "replayed": true,
                "state": "completed",
                "data": Value::Null
            }));
        }
        return Err(RpcError::new(
            "OutcomeUnknown",
            "previous write outcome is unknown; inspect LCU state before retrying",
        ));
    }
    if let Some(previous_hash) = state.write_request_hashes.get(&operation_scope) {
        if *previous_hash != request_hash {
            return Err(RpcError::new(
                "OperationConflict",
                "operationId was already used for a different write request",
            ));
        }
        if !state.write_ledger.contains_key(&key) {
            return Err(RpcError::new(
                "OutcomeUnknown",
                "operationId was used in a previous connection; inspect LCU state before retrying",
            ));
        }
    }
    if let Some(entry) = state.write_ledger.get(&key) {
        if entry.request_hash != request_hash {
            return Err(RpcError::new(
                "OperationConflict",
                "operationId was already used for a different write request",
            ));
        }
        return match entry.state {
            AutomationState::Completed => Ok(json!({
                "replayed": true,
                "state": "completed",
                "data": Value::Null
            })),
            AutomationState::Failed | AutomationState::OutcomeUnknown => Err(RpcError::new(
                "OutcomeUnknown",
                "previous write outcome is unknown; inspect LCU state before retrying",
            )),
            _ => Err(RpcError::new(
                "OutcomeUnknown",
                "previous write outcome is unknown",
            )),
        };
    }
    let Some(client) = state.lcu.clone() else {
        return Err(RpcError::new("NotConnected", "LCU is not connected"));
    };
    let guarded = requires_champ_select_guard(path);
    if guarded {
        let read_permit = state
            .scheduler
            .try_acquire(Priority::P1, 64 * 1024)
            .map_err(|error| RpcError::new("ResourceDeferred", format!("{error:?}")))?;
        let session = client
            .get_json_fresh("/lol-champ-select/v1/session")
            .await
            .map_err(|error| RpcError::new("LcuError", error.to_string()))?;
        drop(read_permit);
        validate_write(&session, path, data.as_ref())
            .map_err(|error| RpcError::new("InvalidState", error.as_str()))?;
    }
    let _permit = state
        .scheduler
        .try_acquire(Priority::P0, 256 * 1024)
        .map_err(|error| RpcError::new("ResourceDeferred", format!("{error:?}")))?;
    ensure_write_capacity(state)?;
    match claim_write_guard(state, guard_key.clone(), request_hash)? {
        WriteGuardClaim::Leader => {}
        WriteGuardClaim::Existing(true) => {
            return Ok(json!({
                "replayed": true,
                "state": "completed",
                "data": Value::Null
            }));
        }
        WriteGuardClaim::Existing(false) => {
            return Err(RpcError::new(
                "OutcomeUnknown",
                "previous write outcome is unknown; inspect LCU state before retrying",
            ));
        }
    }
    state
        .write_request_hashes
        .insert(operation_scope, request_hash);
    state.write_ledger.insert(
        key.clone(),
        WriteLedgerEntry {
            request_hash,
            state: AutomationState::Writing,
        },
    );
    let write_data = data.clone();
    let result = match method {
        "POST" => client.post_json(path, write_data).await,
        "PUT" => client.put_json(path, write_data).await,
        "PATCH" => client.patch_json(path, write_data).await,
        "DELETE" => client.delete(path).await,
        _ => unreachable!(),
    };
    match result {
        Ok(response_data) if guarded => {
            let confirmation = read_write_confirmation(state, &client, path, data.as_ref()).await;
            let result = finish_write(
                &mut state.write_ledger,
                key,
                request_hash,
                confirmation,
                response_data,
                None,
            );
            finalize_write_guard(state, guard_key, request_hash, result)
        }
        Ok(response_data) => {
            state.write_ledger.insert(
                key,
                WriteLedgerEntry {
                    request_hash,
                    state: AutomationState::Completed,
                },
            );
            let result =
                Ok(json!({ "replayed": false, "state": "completed", "data": response_data }));
            finalize_write_guard(state, guard_key, request_hash, result)
        }
        Err(write_error) if guarded => {
            let write_error = write_error.to_string();
            let confirmation = read_write_confirmation(state, &client, path, data.as_ref()).await;
            let result = finish_write(
                &mut state.write_ledger,
                key,
                request_hash,
                confirmation,
                Value::Null,
                Some(&write_error),
            );
            finalize_write_guard(state, guard_key, request_hash, result)
        }
        Err(write_error) => {
            state.write_ledger.insert(
                key,
                WriteLedgerEntry {
                    request_hash,
                    state: AutomationState::OutcomeUnknown,
                },
            );
            Err(RpcError::new(
                "OutcomeUnknown",
                format!("write failed without readback: {write_error}"),
            ))
        }
    }
}

fn event_topic(event: &LcuEvent) -> &'static str {
    if event.uri.ends_with("gameflow-phase") {
        "gameflow.changed"
    } else if event.uri.ends_with("champ-select/v1/session") {
        "champselect.changed"
    } else if event.uri.ends_with("current-summoner") {
        "summoner.changed"
    } else if event.uri.ends_with("entitlements/v1/token") {
        "entitlements.changed"
    } else {
        "lcu.changed"
    }
}

impl NetworkSnapshot {
    async fn cached_asset(
        &self,
        destination: PathBuf,
        source: AssetSource,
        max_bytes: usize,
        priority: Priority,
    ) -> Result<PathBuf, RpcError> {
        let key = source.cache_key(&destination, max_bytes);
        loop {
            if destination.is_file() {
                return Ok(destination);
            }
            match self.asset_cache.start(&key) {
                AssetStart::Leader => break,
                AssetStart::Wait(mut receiver) => {
                    receiver
                        .changed()
                        .await
                        .map_err(|_| RpcError::new("AssetError", "asset download task closed"))?;
                }
                AssetStart::Deferred => {
                    return Err(
                        RpcError::new("AssetDeferred", "asset download failed recently")
                            .retryable(true),
                    );
                }
            }
        }

        let flight = AssetFlight::new(Arc::clone(&self.asset_cache), key);
        let result = self
            .fetch_asset_bytes(&source, max_bytes, priority)
            .await
            .and_then(|bytes| {
                write_file_atomic(&destination, &bytes)
                    .map_err(|error| RpcError::new("IoError", error.to_string()))
            });
        match result {
            Ok(()) => {
                flight.complete();
                Ok(destination)
            }
            Err(error) => {
                if error.code == "ResourceDeferred" {
                    flight.complete();
                }
                Err(error)
            }
        }
    }

    async fn fetch_asset_bytes(
        &self,
        source: &AssetSource,
        max_bytes: usize,
        priority: Priority,
    ) -> Result<Vec<u8>, RpcError> {
        let permit = self
            .scheduler
            .try_acquire(priority, max_bytes)
            .map_err(|error| {
                RpcError::new("ResourceDeferred", format!("{error:?}")).retryable(true)
            })?;
        let result = match source {
            AssetSource::Lcu(path) => {
                let client = self
                    .lcu
                    .clone()
                    .ok_or_else(|| RpcError::new("NotConnected", "LCU is not connected"))?;
                client
                    .get_bytes(path, max_bytes)
                    .await
                    .map_err(|error| RpcError::new("AssetError", error.to_string()))
            }
            AssetSource::External(url) => {
                let client = self.asset_client().await?;
                client
                    .download(url, max_bytes)
                    .await
                    .map_err(|error| RpcError::new("AssetError", error.to_string()))
            }
        };
        drop(permit);
        result
    }

    async fn cache_icon(
        &self,
        folder: &str,
        name: &str,
        path: Option<&str>,
        url: Option<&str>,
        max_bytes: usize,
    ) -> Result<String, RpcError> {
        let destination = safe_cache_path(&self.cache_root, folder, name)?;
        let source = if let Some(path) = path {
            AssetSource::Lcu(path.to_string())
        } else if let Some(url) = url {
            AssetSource::External(url.to_string())
        } else {
            return Err(RpcError::new("InvalidParams", "icon source is missing"));
        };
        self.cached_asset(destination, source, max_bytes, Priority::P2)
            .await
            .map(|path| path.to_string_lossy().to_string())
    }

    async fn resolve_icon(&self, spec: String) -> String {
        let Some((kind, value)) = spec
            .strip_prefix('@')
            .and_then(|value| value.split_once(':'))
        else {
            return spec;
        };
        if kind == "champion_name" {
            return self
                .catalog_champs
                .get(value)
                .cloned()
                .unwrap_or_else(|| value.to_string());
        }
        if kind == "augment_name" {
            return self
                .catalog_augments
                .get(value)
                .cloned()
                .unwrap_or_else(|| value.to_string());
        }
        if kind == "classic_rune" || kind == "legacy_mastery" {
            let Some((url, key)) = value.rsplit_once('|') else {
                return spec;
            };
            let folder = if kind == "classic_rune" {
                "classic rune icons"
            } else {
                "legacy mastery icons"
            };
            return self
                .cache_icon(
                    folder,
                    &format!("{key}.png"),
                    None,
                    Some(url),
                    2 * 1024 * 1024,
                )
                .await
                .unwrap_or(spec);
        }

        let Ok(id) = value.parse::<u64>() else {
            return spec;
        };
        let (folder, extension, fallback) = match kind {
            "champion" => (
                "champion icons",
                "png",
                "app/resource/images/champion-0.png",
            ),
            "item" => ("item icons", "png", "app/resource/images/item-0.png"),
            "spell" => (
                "summoner spell icons",
                "png",
                "app/resource/images/summoner_empty.png",
            ),
            "rune" => ("rune icons", "png", "app/resource/images/rune-0.png"),
            "augment" => ("augment icons", "png", "app/resource/images/item-0.png"),
            _ => return spec,
        };
        let path = if kind == "champion" {
            Some(format!("/lol-game-data/assets/v1/champion-icons/{id}.png"))
        } else {
            self.catalog_paths.get(&format!("{kind}:{id}")).cloned()
        };
        let Some(path) = path else {
            return fallback.to_string();
        };
        self.cache_icon(
            folder,
            &format!("{id}.{extension}"),
            Some(&path),
            None,
            2 * 1024 * 1024,
        )
        .await
        .unwrap_or_else(|_| fallback.to_string())
    }

    async fn resolve_opgg_icons(&mut self, value: &mut Value) {
        seraphine_domain::opgg::resolve_icons(value, self).await;
    }
}

impl NetworkSnapshot {
    async fn build_catalog(&self, force: bool) -> Result<Value, RpcError> {
        let client = self
            .lcu
            .clone()
            .ok_or_else(|| RpcError::new("NotConnected", "LCU is not connected"))?;
        let endpoints = [
            "/lol-game-data/assets/v1/items.json",
            "/lol-game-data/assets/v1/summoner-spells.json",
            "/lol-game-data/assets/v1/perks.json",
            "/lol-game-queues/v1/queues",
            "/lol-game-data/assets/v1/champion-summary.json",
            "/lol-game-data/assets/v1/skins.json",
            "/lol-game-data/assets/v1/perkstyles.json",
            "/lol-game-data/assets/v1/cherry-augments.json",
        ];
        let mut builder = CatalogBuilder::default();
        let priority = if force { Priority::P1 } else { Priority::P4 };
        for (index, endpoint) in endpoints.into_iter().enumerate() {
            let permit = self
                .scheduler
                .try_acquire(priority, 512 * 1024)
                .map_err(|error| {
                    RpcError::new("ResourceDeferred", format!("{error:?}")).retryable(true)
                })?;
            let value = match client.get_json(endpoint).await {
                Ok(value) => value,
                Err(error) => {
                    drop(permit);
                    return Err(
                        RpcError::new("LcuError", format!("{endpoint}: {error}")).retryable(true)
                    );
                }
            };
            drop(permit);
            match index {
                0 => builder.add_items(value),
                1 => builder.add_spells(value),
                2 => builder.add_runes(value),
                3 => builder.add_queues(value),
                4 => builder.add_champions(value),
                5 => builder.add_skins(value),
                6 => builder.add_perks(value),
                7 => builder.add_augments(value),
                _ => unreachable!(),
            }
        }
        let snapshot = builder.build();
        let payload = serde_json::to_vec(&snapshot)
            .map_err(|error| RpcError::new("SerializeError", error.to_string()))?;
        fs::create_dir_all(&self.catalog_root)
            .map_err(|error| RpcError::new("IoError", error.to_string()))?;
        let path = self.catalog_root.join("catalog.json");
        write_file_atomic(&path, &payload)
            .map_err(|error| RpcError::new("IoError", error.to_string()))?;
        let mut state = self.shared.lock().await;
        if state.epoch != self.epoch {
            return Err(RpcError::new("InvalidParams", "stale connection epoch"));
        }
        state.index_catalog(&snapshot);
        Ok(json!({
            "path": path,
            "bytes": payload.len(),
            "epoch": self.epoch,
        }))
    }

    async fn commit_champion_aliases(&self, data: &Value) {
        self.shared.lock().await.cache_champion_aliases(data);
    }

    async fn refresh_champion_alias(
        &self,
        force: bool,
        game_version: Option<&str>,
    ) -> Result<Value, RpcError> {
        let path = self.data_root.join("ChampionAlias.json");
        let mut cached = load_community_cache(&path, "champions");
        if cached.is_none() {
            cached = self
                .champion_aliases
                .as_ref()
                .map(ChampionAliasCache::to_value);
        }
        if !force {
            if let (Some(game_version), Some(data)) = (game_version, cached.as_ref()) {
                if data.get("version").and_then(Value::as_str) == Some(game_version) {
                    self.commit_champion_aliases(data).await;
                    return Ok(json!({
                        "available": true, "updated": false, "data": data, "warning": Value::Null
                    }));
                }
            }
        }

        let client = self.external_client().await?;
        let max_bytes = 1024 * 1024;
        let permit = self
            .scheduler
            .try_acquire(Priority::P3, max_bytes)
            .map_err(|error| RpcError::new("ResourceDeferred", format!("{error:?}")))?;
        let fetched = client
            .get_legacy_json(CHAMPION_ALIAS_URL, "game.gtimg.cn", &json!({}), max_bytes)
            .await
            .map_err(|_| "upstream request failed")
            .and_then(|raw| parse_champion_aliases(&raw).map_err(|_| "invalid alias data"));
        drop(permit);

        let data = match fetched {
            Ok(data) => data,
            Err(error) => {
                if let Some(data) = cached {
                    self.commit_champion_aliases(&data).await;
                    return Ok(json!({
                        "available": true,
                        "updated": false,
                        "data": data,
                        "warning": "upstream update failed; cached data retained"
                    }));
                }
                return Ok(json!({
                    "available": false,
                    "updated": false,
                    "data": Value::Null,
                    "warning": error
                }));
            }
        };
        if let Err(error) = store_community_cache(&path, &data) {
            if let Some(data) = cached {
                return Ok(json!({
                    "available": true,
                    "updated": false,
                    "data": data,
                    "warning": format!("cache update failed: {}", error.message)
                }));
            }
            return Err(error);
        }
        self.commit_champion_aliases(&data).await;
        Ok(json!({
            "available": true, "updated": true, "data": data, "warning": Value::Null
        }))
    }
}

impl NetworkSnapshot {
    async fn fetch_opgg(&self, url: String) -> Result<Value, RpcError> {
        loop {
            match self.opgg_cache.start(&url) {
                Start::Cached(value) => return Ok(value),
                Start::Wait(notify) => {
                    notify.notified().await;
                }
                Start::Leader => break,
            }
        }
        let flight = CacheFlight::new(Arc::clone(&self.opgg_cache), url.clone());
        let client = self.external_client().await?;
        let max_bytes = 2 * 1024 * 1024;
        let _permit = self
            .scheduler
            .acquire(Priority::P2, max_bytes)
            .await
            .map_err(|error| RpcError::new("ResourceDeferred", format!("{error:?}")))?;
        let result = client
            .get_legacy_json(&url, "lol-api-champion.op.gg", &json!({}), max_bytes)
            .await
            .map_err(|error| RpcError::new("ExternalError", error.to_string()));
        match result {
            Ok(value) => {
                let size = serde_json::to_vec(&value)
                    .map(|bytes| bytes.len())
                    .unwrap_or(max_bytes);
                self.opgg_cache
                    .complete(&url, value.clone(), size, Duration::from_secs(60));
                flight.disarm();
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }
}

#[async_trait::async_trait]
impl seraphine_domain::opgg::IconResolver for NetworkSnapshot {
    async fn resolve(&mut self, spec: String) -> String {
        self.resolve_icon(spec).await
    }
}

const MAX_CONCURRENT_READS: usize = 8;

struct InFlightRequest {
    task: tokio::task::JoinHandle<()>,
    token: u64,
    started: Arc<AtomicBool>,
    deadline: Option<tokio::time::Instant>,
    abandoned: bool,
    cancel_safe: bool,
    started_at: Instant,
    epoch: u64,
}

async fn execute_request(
    message: Message,
    shared: Arc<Mutex<State>>,
    read_slots: Arc<Semaphore>,
    control_slots: Arc<Semaphore>,
    completion: mpsc::Sender<(u64, u64, Message)>,
    started: Arc<AtomicBool>,
    token: u64,
) {
    let cancel_safe = is_concurrent_read_op(message.op.as_deref().unwrap_or_default());
    let slots = if cancel_safe {
        read_slots
    } else {
        control_slots
    };
    let _permit = Arc::clone(&slots).acquire_owned().await;
    started.store(true, Ordering::Release);
    let response = if cancel_safe {
        handle_concurrent_read_request(&message, &shared).await
    } else if message.op.as_deref() == Some("lcu.write") {
        handle_write_request(&message, shared.clone()).await
    } else if is_snapshot_network_op(message.op.as_deref().unwrap_or_default()) {
        handle_snapshot_network_request(&message, shared.clone()).await
    } else {
        let mut state = shared.lock().await;
        handle_request(&message, &mut state).await
    };
    if let Some(id) = message.id {
        let _ = completion.send((token, id, response)).await;
    }
}

fn next_request_deadline(requests: &HashMap<u64, InFlightRequest>) -> Option<tokio::time::Instant> {
    requests
        .iter()
        .filter(|(_, request)| !request.abandoned)
        .filter_map(|(_, request)| request.deadline)
        .min()
}

async fn abandon_request(
    request_id: u64,
    deadline_exceeded: bool,
    requests: &mut HashMap<u64, InFlightRequest>,
    outgoing: &mpsc::Sender<OutgoingFrame>,
    byte_budget: &Arc<Semaphore>,
) -> io::Result<()> {
    let Some(mut request) = requests.remove(&request_id) else {
        return Ok(());
    };
    let response_epoch = request.epoch;
    let can_abort = request.cancel_safe || !request.started.load(Ordering::Acquire);
    if can_abort {
        request.task.abort();
    } else {
        request.abandoned = true;
        requests.insert(request_id, request);
    }
    let error = if can_abort {
        if deadline_exceeded {
            RpcError::new("DeadlineExceeded", "request deadline exceeded").retryable(true)
        } else {
            RpcError::new("Canceled", "request canceled")
        }
    } else {
        RpcError::new(
            "OutcomeUnknown",
            "request was abandoned after execution started; inspect the result before retrying",
        )
    };
    queue_message(
        outgoing,
        byte_budget,
        Message::response_error(request_id, response_epoch, error),
    )
    .await
}

fn hello_message(epoch: u64) -> Message {
    let mut message = Message::hello(
        active_capabilities()
            .into_iter()
            .map(str::to_string)
            .collect(),
        env!("CARGO_PKG_VERSION"),
    );
    message.epoch = Some(epoch);
    message
}

async fn run() -> io::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut frame_reader = FrameReader::new(stdin);
    let (outgoing_tx, outgoing_rx) = mpsc::channel(MAX_EVENT_QUEUE);
    let byte_budget = Arc::new(Semaphore::new(MAX_IPC_INFLIGHT_BYTES));
    let mut writer_task = tokio::spawn(writer_loop(stdout, outgoing_rx));
    let shared = Arc::new(Mutex::new(State::new()));
    let mut ws_event_commands = shared
        .lock()
        .await
        .ws_event_rx_rx
        .take()
        .expect("WS event command receiver missing");
    let mut ws_events: Option<(u64, mpsc::Receiver<LcuEvent>)> = None;
    let read_slots = Arc::new(Semaphore::new(MAX_CONCURRENT_READS));
    let control_slots = Arc::new(Semaphore::new(1));
    let (completion_tx, mut completion_rx) = mpsc::channel(MAX_PENDING_REQUESTS);
    let mut requests: HashMap<u64, InFlightRequest> = HashMap::new();
    let mut next_request_token = 0_u64;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;

    queue_message(&outgoing_tx, &byte_budget, hello_message(0)).await?;

    loop {
        let next_deadline = next_request_deadline(&requests);
        tokio::select! {
            writer_result = &mut writer_task => {
                writer_result??;
                return Ok(());
            }
            command = ws_event_commands.recv() => {
                if let Some((generation, receiver)) = command {
                    ws_events = Some((generation, receiver));
                }
            }
            event = async {
                match ws_events.as_mut() {
                    Some((_, receiver)) => receiver.recv().await,
                    None => std::future::pending().await,
                }
            }, if ws_events.is_some() => {
                if let Some(event) = event {
                    let mut messages = Vec::new();
                    {
                        let mut state = shared.lock().await;
                        state.ws_queue_len = ws_events.as_ref().map(|(_, events)| events.len()).unwrap_or(0);
                        let generation = ws_events.as_ref().map(|(generation, _)| *generation).unwrap_or(0);
                        if generation != state.epoch {
                            continue;
                        }
                        state.metrics.events += 1;
                        let topic = event_topic(&event);
                        if topic == "champselect.changed" {
                            if let Some(automation) = state.automation.as_mut() {
                                let plans = automation.handle_event(&event.data);
                                if !plans.is_empty() {
                                    messages.push(Message::event(
                                        state.epoch,
                                        event.seq,
                                        "automation.planned",
                                        json!({ "plans": plans }),
                                    ));
                                }
                            }
                        }
                        if topic == "entitlements.changed" {
                            if let Some(token) = parse_sgp_token(&event.data) {
                                apply_sgp_token_event(&mut state, token);
                            }
                        }
                        let data = if topic == "champselect.changed" {
                            json!({
                                "uri": event.uri,
                                "eventType": event.event_type,
                                "data": event.data,
                            })
                        } else {
                            event.data
                        };
                        messages.push(Message::event(state.epoch, event.seq, topic, data));
                    }
                    for message in messages {
                        queue_message(&outgoing_tx, &byte_budget, message).await?;
                    }
                }
            }
            completed = completion_rx.recv() => {
                let Some((token, request_id, response)) = completed else { break };
                if requests
                    .get(&request_id)
                    .is_some_and(|request| request.token == token)
                {
                    let request = requests.remove(&request_id).unwrap();
                    if !request.abandoned {
                        queue_message(&outgoing_tx, &byte_budget, response).await?;
                    }
                    shared
                        .lock()
                        .await
                        .metrics
                        .record_latency(request.started_at.elapsed().as_micros() as u64);
                }
            }
            _ = async {
                match next_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            } => {
                let now = tokio::time::Instant::now();
                let expired = requests
                    .iter()
                    .filter(|(_, request)| {
                        !request.abandoned && request.deadline.is_some_and(|deadline| deadline <= now)
                    })
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for request_id in expired {
                    abandon_request(
                        request_id,
                        true,
                        &mut requests,
                        &outgoing_tx,
                        &byte_budget,
                    )
                    .await?;
                }
            }
            message = frame_reader.next_message() => {
                let Some(message) = message? else { break };
                if let Err(error) = message.validate() {
                    let epoch = shared.lock().await.epoch;
                    let response = Message::response_error(
                        message.id.unwrap_or(0),
                        message.epoch.unwrap_or(epoch),
                        RpcError::new("ProtocolError", error.to_string()),
                    );
                    queue_message(&outgoing_tx, &byte_budget, response).await?;
                    continue;
                }

                match message.kind {
                    MessageKind::Hello => {
                        let epoch = {
                            let mut state = shared.lock().await;
                            state.epoch = state.epoch.max(message.epoch.unwrap_or(0));
                            state.epoch
                        };
                        queue_message(&outgoing_tx, &byte_budget, hello_message(epoch)).await?;
                    }
                    MessageKind::Request => {
                        let request_id = message.id.unwrap_or(0);
                        let current_epoch = shared.lock().await.epoch;
                        let request_epoch = message.epoch.unwrap_or(current_epoch);
                        if requests.contains_key(&request_id) {
                            queue_message(
                                &outgoing_tx,
                                &byte_budget,
                                Message::response_error(
                                    request_id,
                                    request_epoch,
                                    RpcError::new("InvalidParams", "duplicate request id"),
                                ),
                            )
                            .await?;
                            continue;
                        }
                        if requests.len() >= MAX_PENDING_REQUESTS {
                            queue_message(
                                &outgoing_tx,
                                &byte_budget,
                                Message::response_error(
                                    request_id,
                                    request_epoch,
                                    RpcError::new("ResourceDeferred", "too many pending requests")
                                        .retryable(true),
                                ),
                            )
                            .await?;
                            continue;
                        }
                        let deadline = message
                            .deadline_ms
                            .and_then(|milliseconds| tokio::time::Instant::now().checked_add(Duration::from_millis(milliseconds)));
                        if deadline.is_some_and(|deadline| deadline <= tokio::time::Instant::now()) {
                            queue_message(
                                &outgoing_tx,
                                &byte_budget,
                                Message::response_error(
                                    request_id,
                                    request_epoch,
                                    RpcError::new("DeadlineExceeded", "request deadline exceeded")
                                        .retryable(true),
                                ),
                            )
                            .await?;
                            continue;
                        }
                        let started = Arc::new(AtomicBool::new(false));
                        let cancel_safe = is_concurrent_read_op(message.op.as_deref().unwrap_or_default());
                        next_request_token = next_request_token.wrapping_add(1);
                        let token = next_request_token;
                        let task = tokio::spawn(execute_request(
                            message,
                            Arc::clone(&shared),
                            Arc::clone(&read_slots),
                            Arc::clone(&control_slots),
                            completion_tx.clone(),
                            Arc::clone(&started),
                            token,
                        ));
                        requests.insert(
                            request_id,
                            InFlightRequest {
                                task,
                                token,
                                started,
                                deadline,
                                abandoned: false,
                                cancel_safe,
                                started_at: Instant::now(),
                                epoch: request_epoch,
                            },
                        );
                    }
                    MessageKind::Ping => {
                        queue_message(
                            &outgoing_tx,
                            &byte_budget,
                            Message::pong(message.nonce.unwrap_or(0)),
                        )
                        .await?;
                    }
                    MessageKind::Cancel => {
                        if let Some(request_id) = message.id {
                            abandon_request(
                                request_id,
                                false,
                                &mut requests,
                                &outgoing_tx,
                                &byte_budget,
                            )
                            .await?;
                        }
                    }
                    MessageKind::Shutdown => break,
                    _ => {}
                }
            }
            _ = heartbeat.tick() => {}
        }
    }

    for request in requests.values() {
        request.task.abort();
    }
    drop(outgoing_tx);
    writer_task.await??;

    let mut state = shared.lock().await;
    if let Some(shutdown) = state.ws_shutdown.take() {
        let _ = shutdown.send(()).await;
    }
    if let Some(task) = state.ws_task.take() {
        task.abort();
    }

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("seraphine-sidecar: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_reader_preserves_partial_frames_when_polling_is_canceled() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let first = encode(&Message::ping(1)).unwrap();
        let second = encode(&Message::ping(2)).unwrap();
        writer.write_all(&first[..2]).await.unwrap();
        let mut frames = FrameReader::new(reader);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), frames.next_message())
                .await
                .is_err()
        );
        writer.write_all(&first[2..]).await.unwrap();
        writer.write_all(&second).await.unwrap();
        assert_eq!(frames.next_message().await.unwrap(), Some(Message::ping(1)));
        assert_eq!(frames.next_message().await.unwrap(), Some(Message::ping(2)));
    }

    #[tokio::test]
    async fn outgoing_writer_preserves_frame_order() {
        let (mut writer, mut reader) = tokio::io::duplex(4096);
        let (outgoing, receiver) = mpsc::channel(2);
        let budget = Arc::new(Semaphore::new(MAX_IPC_INFLIGHT_BYTES));
        queue_message(&outgoing, &budget, Message::ping(1))
            .await
            .unwrap();
        queue_message(&outgoing, &budget, Message::ping(2))
            .await
            .unwrap();
        drop(outgoing);
        writer_loop(&mut writer, receiver).await.unwrap();
        drop(writer);
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        let first_length = decode_length_prefix(&bytes).unwrap().unwrap();
        assert_eq!(
            decode(&bytes[4..4 + first_length]).unwrap(),
            Message::ping(1)
        );
        let offset = 4 + first_length;
        let second_length = decode_length_prefix(&bytes[offset..]).unwrap().unwrap();
        assert_eq!(
            decode(&bytes[offset + 4..offset + 4 + second_length]).unwrap(),
            Message::ping(2)
        );
        assert_eq!(bytes.len(), offset + 4 + second_length);
    }

    #[tokio::test]
    async fn outgoing_byte_budget_applies_backpressure() {
        let frame_size = encode(&Message::ping(1)).unwrap().len() as u32;
        let budget = Arc::new(Semaphore::new(frame_size as usize));
        let (outgoing, receiver) = mpsc::channel(2);
        queue_message(&outgoing, &budget, Message::ping(1))
            .await
            .unwrap();
        let (second_tx, second_budget) = (outgoing.clone(), Arc::clone(&budget));
        let second = tokio::spawn(async move {
            queue_message(&second_tx, &second_budget, Message::ping(2)).await
        });
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
        assert!(!second.is_finished());
        drop(receiver);
        assert!(second.await.unwrap().is_err());
    }

    #[test]
    fn cache_writes_atomically_replace_existing_files() {
        let directory = env::temp_dir().join(format!(
            "seraphine-cache-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("asset.png");
        fs::write(&destination, b"old").unwrap();
        write_file_atomic(&destination, b"new").unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_cache_replacement_preserves_destination() {
        let directory = env::temp_dir().join(format!(
            "seraphine-cache-failure-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("asset.png");
        fs::create_dir(&destination).unwrap();
        assert!(write_file_atomic(&destination, b"new").is_err());
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn asset_cache_keys_include_source_and_budget() {
        let destination = Path::new("cache/asset.png");
        let lcu = AssetSource::Lcu("/asset.png".to_string());
        let external = AssetSource::External("https://example.com/asset.png".to_string());
        assert_ne!(
            lcu.cache_key(destination, 1024),
            lcu.cache_key(destination, 2048)
        );
        assert_ne!(
            lcu.cache_key(destination, 1024),
            lcu.cache_key(Path::new("cache/other.png"), 1024)
        );
        assert_ne!(
            lcu.cache_key(destination, 1024),
            external.cache_key(destination, 1024)
        );
    }

    #[test]
    fn write_ledger_key_uses_full_operation_scope() {
        let mut state = State::new();
        state.connection = Some(ConnectionInfo {
            pid: 42,
            port: 55123,
            server: "HN1".to_string(),
            epoch: 7,
        });
        state.automation = Some(DryRunAutomation::new(7, "session-1", json!({})));
        let data = json!({"type": "pick", "championId": 1});
        let key = write_ledger_key(
            &state,
            7,
            "operation-1",
            "POST",
            "/lol-champ-select/v1/session/actions/10",
            Some(&data),
        );
        assert_eq!(key.pid, 42);
        assert_eq!(key.epoch, 7);
        assert_eq!(key.session_id, "session-1");
        assert_eq!(key.action_id, 10);
        assert_eq!(key.action_type, "pick");
        assert_eq!(key.config_revision, 0);
        assert_eq!(key.operation_id, "operation-1");

        state
            .automation
            .as_mut()
            .unwrap()
            .update_config(json!({"changed": true}));
        let changed = write_ledger_key(
            &state,
            7,
            "operation-1",
            "POST",
            "/lol-champ-select/v1/session/actions/10",
            Some(&data),
        );
        assert_ne!(key, changed);
        assert_eq!(changed.config_revision, 1);
    }

    #[test]
    fn sgp_token_parser_rejects_empty_values() {
        assert_eq!(
            parse_sgp_token(&json!({"accessToken": "token"})),
            Some("token".to_string())
        );
        assert_eq!(parse_sgp_token(&json!({"accessToken": ""})), None);
        assert_eq!(parse_sgp_token(&json!({})), None);
        assert!(!sgp_rpc_error(LcuError::Unauthorized).retryable);

        let mut state = State::new();
        state.sgp_token = Some(Zeroizing::new("new-token".to_string()));
        state.sgp_token_superseded = Some(Zeroizing::new("old-token".to_string()));
        apply_sgp_token_event(&mut state, "old-token".to_string());
        assert_eq!(
            state.sgp_token.as_deref().map(String::as_str),
            Some("new-token")
        );
        apply_sgp_token_event(&mut state, "newer-token".to_string());
        assert_eq!(
            state.sgp_token.as_deref().map(String::as_str),
            Some("newer-token")
        );
        assert!(state.sgp_token_superseded.is_none());
    }

    #[tokio::test]
    async fn sgp_refresh_skips_when_token_already_changed() {
        let shared = Arc::new(Mutex::new(State::new()));
        {
            let mut state = shared.lock().await;
            state.epoch = 1;
            state.sgp_token = Some(Zeroizing::new("new-token".to_string()));
        }
        let snapshot = {
            let state = shared.lock().await;
            state.network_snapshot(shared.clone())
        };
        let previous = Zeroizing::new("old-token".to_string());
        let token = snapshot.refresh_sgp_token(&previous).await.unwrap();
        assert_eq!(token.as_str(), "new-token");
        assert_eq!(shared.lock().await.sgp_refresh_count, 0);
    }

    #[tokio::test]
    async fn sgp_snapshot_rejects_stale_epoch_before_network() {
        let shared = Arc::new(Mutex::new(State::new()));
        {
            let mut state = shared.lock().await;
            state.epoch = 1;
            state.sgp_token = Some(Zeroizing::new("token".to_string()));
        }
        let snapshot = {
            let state = shared.lock().await;
            state.network_snapshot(shared.clone())
        };
        shared.lock().await.epoch = 2;
        let error = snapshot.sgp_get("/test", None).await.unwrap_err();
        assert_eq!(error.code, "InvalidParams");
    }

    #[test]
    fn write_request_hash_detects_operation_reuse_conflicts() {
        let first = write_request_hash(
            "POST",
            "/lol-champ-select/v1/session/actions/10",
            Some(&json!({"championId": 1})),
        );
        let changed_body = write_request_hash(
            "POST",
            "/lol-champ-select/v1/session/actions/10",
            Some(&json!({"championId": 2})),
        );
        let changed_method = write_request_hash(
            "PUT",
            "/lol-champ-select/v1/session/actions/10",
            Some(&json!({"championId": 1})),
        );
        assert_ne!(first, changed_body);
        assert_ne!(first, changed_method);
        assert_ne!(
            write_request_hash("POST", "/actions/10", None),
            write_request_hash("POST", "/actions/10", Some(&Value::Null))
        );
    }

    #[test]
    fn failed_http_write_can_complete_after_readback() {
        let key = WriteLedgerKey {
            pid: 1,
            epoch: 1,
            session_id: "session".to_string(),
            action_id: 10,
            action_type: "pick".to_string(),
            config_revision: 0,
            operation_id: "operation".to_string(),
        };
        let mut ledger = HashMap::new();
        let request_hash =
            write_request_hash("POST", "/actions/10", Some(&json!({"championId": 1})));
        let result = finish_write(
            &mut ledger,
            key.clone(),
            request_hash,
            Ok(()),
            Value::Null,
            Some("HTTP 500"),
        )
        .unwrap();
        assert_eq!(result["confirmedAfterError"], true);
        assert_eq!(
            ledger.get(&key).map(|entry| entry.state),
            Some(AutomationState::Completed)
        );
    }

    #[test]
    fn persistent_write_guards_round_trip_without_raw_ids() {
        let directory = env::temp_dir().join(format!(
            "seraphine-write-guards-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("write-guards.json");
        let mut state = State::new();
        state.write_guard_path = path.clone();
        state.write_guard_error = None;
        state.write_guards.clear();
        let operation = operation_guard_key(42, "operation-secret");
        let request_hash =
            write_request_hash("POST", "/actions/10", Some(&json!({"championId": 1})));
        persist_write_guard(&mut state, operation.clone(), request_hash, false).unwrap();
        let payload = fs::read_to_string(&path).unwrap();
        assert!(!payload.contains("operation-secret"));
        assert!(!payload.contains("session-1"));
        let (loaded, error) = load_write_guards(&path);
        assert!(error.is_none());
        assert!(!loaded[&operation].completed);

        persist_write_guard(&mut state, operation.clone(), request_hash, true).unwrap();
        let (loaded, error) = load_write_guards(&path);
        assert!(error.is_none());
        assert!(loaded[&operation].completed);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persistent_write_guards_merge_across_sidecar_instances() {
        let directory = env::temp_dir().join(format!(
            "seraphine-write-guards-merge-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("write-guards.json");
        let mut first = State::new();
        first.write_guard_path = path.clone();
        first.write_guard_error = None;
        first.write_guards.clear();
        let mut second = State::new();
        second.write_guard_path = path.clone();
        second.write_guard_error = None;
        second.write_guards.clear();
        let first_operation = operation_guard_key(1, "first");
        let second_operation = operation_guard_key(1, "second");
        let request_hash = write_request_hash("POST", "/actions/10", None);
        persist_write_guard(&mut first, first_operation.clone(), request_hash, false).unwrap();
        persist_write_guard(&mut second, second_operation.clone(), request_hash, false).unwrap();
        let (loaded, error) = load_write_guards(&path);
        assert!(error.is_none());
        assert_eq!(loaded.len(), 2);

        persist_write_guard(&mut first, first_operation.clone(), request_hash, true).unwrap();
        persist_write_guard(&mut second, first_operation.clone(), request_hash, false).unwrap();
        let (loaded, error) = load_write_guards(&path);
        assert!(error.is_none());
        assert!(loaded[&first_operation].completed);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn only_one_sidecar_can_claim_a_write() {
        let directory = env::temp_dir().join(format!(
            "seraphine-write-claim-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("write-guards.json");
        let mut first = State::new();
        first.write_guard_path = path.clone();
        first.write_guard_error = None;
        first.write_guards.clear();
        let mut second = State::new();
        second.write_guard_path = path;
        second.write_guard_error = None;
        second.write_guards.clear();
        let operation = operation_guard_key(1, "shared-operation");
        let request_hash = write_request_hash("POST", "/actions/10", None);
        assert!(matches!(
            claim_write_guard(&mut first, operation.clone(), request_hash),
            Ok(WriteGuardClaim::Leader)
        ));
        assert!(matches!(
            claim_write_guard(&mut second, operation.clone(), request_hash),
            Ok(WriteGuardClaim::Existing(false))
        ));
        persist_write_guard(&mut first, operation.clone(), request_hash, true).unwrap();
        assert!(matches!(
            claim_write_guard(&mut second, operation, request_hash),
            Ok(WriteGuardClaim::Existing(true))
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn corrupt_write_guards_fail_closed() {
        let directory = env::temp_dir().join(format!(
            "seraphine-write-guards-invalid-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("write-guards.json");
        fs::write(&path, b"not-json").unwrap();
        let (guards, error) = load_write_guards(&path);
        assert!(guards.is_empty());
        assert!(error.is_some());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_confirmation_is_outcome_unknown() {
        let key = WriteLedgerKey {
            pid: 1,
            epoch: 1,
            session_id: "session".to_string(),
            action_id: 10,
            action_type: "pick".to_string(),
            config_revision: 0,
            operation_id: "operation".to_string(),
        };
        let mut ledger = HashMap::new();
        let request_hash =
            write_request_hash("POST", "/actions/10", Some(&json!({"championId": 1})));
        let error = finish_write(
            &mut ledger,
            key.clone(),
            request_hash,
            Err(RpcError::new("InvalidState", "mismatch")),
            Value::Null,
            Some("HTTP 500"),
        )
        .unwrap_err();
        assert_eq!(error.code, "OutcomeUnknown");
        assert_eq!(
            ledger.get(&key).map(|entry| entry.state),
            Some(AutomationState::OutcomeUnknown)
        );
    }

    #[test]
    fn expired_completed_guards_are_pruned_without_losing_unknowns() {
        let now = unix_millis();
        let mut guards = HashMap::new();
        guards.insert(
            "expired".to_string(),
            PersistedWriteGuard {
                request_hash: [1; 32],
                completed: true,
                completed_at_ms: now - WRITE_GUARD_RETENTION_MS - 1,
            },
        );
        guards.insert(
            "recent".to_string(),
            PersistedWriteGuard {
                request_hash: [2; 32],
                completed: true,
                completed_at_ms: now,
            },
        );
        guards.insert(
            "unknown".to_string(),
            PersistedWriteGuard {
                request_hash: [3; 32],
                completed: false,
                completed_at_ms: 0,
            },
        );
        guards.insert(
            "legacy".to_string(),
            PersistedWriteGuard {
                request_hash: [4; 32],
                completed: true,
                completed_at_ms: 0,
            },
        );
        prune_expired_completed_guards(&mut guards, now);
        assert!(!guards.contains_key("expired"));
        assert!(guards.contains_key("recent"));
        assert!(guards.contains_key("unknown"));
        assert!(guards.contains_key("legacy"));
    }

    #[test]
    fn full_guard_capacity_unblocks_after_expired_completed_prune() {
        let directory = env::temp_dir().join(format!(
            "seraphine-write-guard-prune-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("write-guards.json");
        let now = unix_millis();
        let mut guards = HashMap::new();
        for index in 0..MAX_PERSISTED_WRITE_GUARDS {
            guards.insert(
                format!("{index:064x}"),
                PersistedWriteGuard {
                    request_hash: [index as u8; 32],
                    completed: true,
                    completed_at_ms: now - WRITE_GUARD_RETENTION_MS - 1,
                },
            );
        }
        let payload = write_guard_payload(&guards).unwrap();
        write_file_atomic(&path, &payload).unwrap();
        let mut state = State::new();
        state.write_guard_path = path.clone();
        state.write_guard_error = None;
        state.write_guards = guards;
        let operation = format!("{:064x}", MAX_PERSISTED_WRITE_GUARDS);
        assert!(matches!(
            claim_write_guard(&mut state, operation, [9; 32]),
            Ok(WriteGuardClaim::Leader)
        ));
        assert_eq!(state.write_guards.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn snapshot_network_ops_bypass_the_control_state_lock() {
        for op in [
            "asset.get",
            "opgg.positions",
            "opgg.tier_list",
            "opgg.champion_build",
            "lcu.catalog",
            "champion_alias.refresh",
        ] {
            assert!(is_snapshot_network_op(op), "{op}");
        }
        assert!(!is_snapshot_network_op("lcu.write"));
    }
}
