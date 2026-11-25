#![windows_subsystem = "windows"]

use seraphine_automation::write_guard::{confirm_write, validate_write};
use seraphine_automation::{AutomationState, DryRunAutomation};
use seraphine_domain::catalog::CatalogBuilder;
use seraphine_domain::community::{
    aram_info, parse_aram_buffs, parse_champion_aliases, validate_cached, ARAM_URL,
    CHAMPION_ALIAS_URL,
};
use seraphine_domain::opgg::{
    parse_arena_champion_build, parse_other_champion_build, parse_other_tier_list,
    parse_ranked_tier_list,
};
use seraphine_lcu::cache::{JsonCache, Start};
use seraphine_lcu::websocket::{
    run_websocket, LcuEvent, WsStats, CHAMP_SELECT_TOPIC, ENTITLEMENT_TOPIC, GAMEFLOW_TOPIC,
    SUMMONER_TOPIC,
};
use seraphine_lcu::{AssetClient, ConnectionInfo, ExternalClient, LcuClient, SgpClient};
use seraphine_platform::{
    atomic_replace_file, current_process_memory, list_league_client_pids, list_processes,
    read_process_command_line,
};
use seraphine_protocol::{
    decode, decode_length_prefix, encode, Message, MessageKind, Metrics, RpcError, MAX_EVENT_QUEUE,
    PROTOCOL_VERSION,
};
use seraphine_resource_scheduler::{GameState, Priority, ResourceScheduler};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};
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

fn active_capabilities() -> Vec<&'static str> {
    let mut capabilities = capabilities::ALL.to_vec();
    if env::var_os("SERAPHINE_ENABLE_LCU_WRITES").as_deref() == Some(std::ffi::OsStr::new("1")) {
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

struct State {
    epoch: u64,
    started_at: Instant,
    metrics: Metrics,
    scheduler: ResourceScheduler,
    connection: Option<ConnectionInfo>,
    lcu: Option<LcuClient>,
    sgp: Option<SgpClient>,
    external: Option<ExternalClient>,
    opgg_cache: JsonCache,
    asset: Option<AssetClient>,
    ws_task: Option<tokio::task::JoinHandle<()>>,
    ws_shutdown: Option<mpsc::Sender<()>>,
    ws_events: Option<mpsc::Receiver<LcuEvent>>,
    ws_stats: Arc<WsStats>,
    sgp_token: Option<Zeroizing<String>>,
    cache_root: PathBuf,
    data_root: PathBuf,
    champion_aliases: Option<ChampionAliasCache>,
    external_proxy: Option<String>,
    automation: Option<DryRunAutomation>,
    write_ledger: HashMap<(u64, String), AutomationState>,
    catalog_champs: HashMap<String, String>,
    catalog_augments: HashMap<String, String>,
    catalog_paths: HashMap<String, String>,
}

impl State {
    fn new() -> Self {
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
            opgg_cache: JsonCache::default(),
            asset: None,
            ws_task: None,
            ws_shutdown: None,
            ws_events: None,
            ws_stats: Arc::new(WsStats::default()),
            sgp_token: None,
            cache_root: env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(env::temp_dir)
                .join("Seraphine")
                .join("Cache")
                .join("game"),
            data_root: env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(env::temp_dir)
                .join("Seraphine"),
            champion_aliases: None,
            external_proxy: None,
            automation: None,
            write_ledger: HashMap::new(),
            catalog_champs: HashMap::new(),
            catalog_augments: HashMap::new(),
            catalog_paths: HashMap::new(),
        }
    }

    fn cache_champion_aliases(&mut self, data: &Value) {
        self.champion_aliases = ChampionAliasCache::from_value(data);
    }

    fn restore_catalog_indexes(&mut self) {
        let path = self.data_root.join("catalog.json");
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
        self.catalog_champs = snapshot
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
            .unwrap_or_default();
        self.catalog_augments = snapshot
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
            .unwrap_or_default();
        self.catalog_paths.clear();
        for (kind, key, path_key) in [
            ("item", "items", "iconPath"),
            ("spell", "summonerSpells", "iconPath"),
            ("rune", "runes", "icon"),
            ("augment", "cherryAugments", "augmentSmallIconPath"),
        ] {
            if let Some(items) = snapshot.get(key).and_then(Value::as_object) {
                for (id, item) in items {
                    if let Some(path) = item.get(path_key).and_then(Value::as_str) {
                        self.catalog_paths
                            .insert(format!("{kind}:{id}"), path.to_string());
                    }
                }
            }
        }
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn read_message<R>(reader: &mut R) -> io::Result<Option<Message>>
where
    R: AsyncRead + Unpin + ?Sized,
{
    let mut prefix = [0_u8; 4];
    match reader.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_le_bytes(prefix) as usize;
    let expected = decode_length_prefix(&prefix)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    if expected != Some(length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid frame length",
        ));
    }
    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload).await?;
    decode(&payload)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

async fn write_message(
    writer: &Arc<Mutex<tokio::io::Stdout>>,
    message: &Message,
) -> io::Result<()> {
    let frame = encode(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut writer = writer.lock().await;
    writer.write_all(&frame).await?;
    writer.flush().await
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

fn community_filename(kind: &str) -> Option<&'static str> {
    match kind {
        "champion_alias" => Some("ChampionAlias.json"),
        "aram_buff" => Some("AramBuff.json"),
        _ => None,
    }
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
    let parent = path
        .parent()
        .ok_or_else(|| RpcError::new("InvalidParams", "cache parent is missing"))?;
    fs::create_dir_all(parent).map_err(|error| RpcError::new("IoError", error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, payload).map_err(|error| RpcError::new("IoError", error.to_string()))?;
    if let Err(error) = atomic_replace_file(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(RpcError::new("IoError", error.to_string()));
    }
    Ok(())
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
                .and_then(|data| {
                    data.get("accessToken")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .map(Zeroizing::new);
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
            state.ws_events = Some(event_rx);
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
        "lcu.write" => {
            if env::var_os("SERAPHINE_ENABLE_LCU_WRITES").as_deref()
                != Some(std::ffi::OsStr::new("1"))
            {
                Err(RpcError::new("WriteDisabled", "LCU writes are disabled"))
            } else if message.epoch != Some(state.epoch) {
                Err(RpcError::new("InvalidParams", "stale connection epoch"))
            } else {
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
                if !matches!(method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE")
                    || !path.starts_with('/')
                    || path.contains("://")
                    || path.contains('\r')
                    || path.contains('\n')
                    || path.len() > 2048
                    || operation_id.is_empty()
                    || operation_id.len() > 128
                {
                    Err(RpcError::new(
                        "InvalidParams",
                        "invalid LCU write parameters",
                    ))
                } else if let Some(body) = data.as_ref() {
                    if serde_json::to_vec(body)
                        .map(|bytes| bytes.len() > 256 * 1024)
                        .unwrap_or(true)
                    {
                        Err(RpcError::new(
                            "InvalidParams",
                            "LCU write body is too large",
                        ))
                    } else {
                        write_lcu(state, epoch, &method, path, operation_id, data).await
                    }
                } else {
                    write_lcu(state, epoch, &method, path, operation_id, None).await
                }
            }
        }
        "lcu.catalog" => {
            let Some(client) = state.lcu.clone() else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("NotConnected", "LCU is not connected"),
                );
            };
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
            let priority = if message
                .args
                .as_ref()
                .and_then(|args| args.get("force"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                Priority::P1
            } else {
                Priority::P4
            };
            for (index, endpoint) in endpoints.into_iter().enumerate() {
                let permit = match state.scheduler.try_acquire(priority, 512 * 1024) {
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
                match client.get_json(endpoint).await {
                    Ok(value) => {
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
                    Err(error) => {
                        drop(permit);
                        state.metrics.errors += 1;
                        return Message::response_error(
                            id,
                            epoch,
                            RpcError::new("LcuError", format!("{endpoint}: {error}"))
                                .retryable(true),
                        );
                    }
                }
            }
            let snapshot = builder.build();
            state.index_catalog(&snapshot);
            let payload = match serde_json::to_vec(&snapshot) {
                Ok(payload) => payload,
                Err(error) => {
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("SerializeError", error.to_string()),
                    );
                }
            };
            let base = env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(env::temp_dir)
                .join("Seraphine");
            if let Err(error) = fs::create_dir_all(&base) {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("IoError", error.to_string()),
                );
            }
            let path = base.join("catalog.json");
            let temporary = base.join("catalog.json.tmp");
            if let Err(error) = fs::write(&temporary, &payload).and_then(|_| {
                if path.exists() {
                    fs::remove_file(&path)?;
                }
                fs::rename(&temporary, &path)
            }) {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("IoError", error.to_string()),
                );
            }
            Ok(json!({
                "path": path,
                "bytes": payload.len(),
                "epoch": state.epoch,
            }))
        }
        "sgp.get" => {
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
                    RpcError::new("InvalidParams", "only SGP paths are allowed"),
                );
            }
            let Some(connection) = state.connection.clone() else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("NotConnected", "LCU is not connected"),
                );
            };
            let Some(token) = state.sgp_token.clone() else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("NotConnected", "SGP token is unavailable"),
                );
            };
            if state.sgp.is_none() {
                match SgpClient::new(&connection.server) {
                    Ok(client) => state.sgp = Some(client),
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
            let permit = match state.scheduler.try_acquire(Priority::P2, 128 * 1024) {
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
            let params = message.args.as_ref().and_then(|args| args.get("params"));
            match state
                .sgp
                .as_ref()
                .unwrap()
                .get_json(path, token.as_str(), params)
                .await
            {
                Ok(data) => {
                    drop(permit);
                    Ok(data)
                }
                Err(error) => {
                    drop(permit);
                    Err(RpcError::new("SgpError", error.to_string()).retryable(true))
                }
            }
        }
        "asset.get" => {
            let args = message.args.clone().unwrap_or(Value::Null);
            let folder = args
                .get("folder")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
            let destination = match safe_cache_path(&state.cache_root, folder, name) {
                Ok(path) => path,
                Err(error) => {
                    state.metrics.errors += 1;
                    return Message::response_error(id, epoch, error);
                }
            };
            if destination.is_file() {
                return Message::response(id, epoch, json!({ "path": destination }));
            }
            let max_bytes = args
                .get("maxBytes")
                .and_then(Value::as_u64)
                .unwrap_or(2 * 1024 * 1024) as usize;
            let priority = if args.get("force").and_then(Value::as_bool).unwrap_or(false) {
                Priority::P2
            } else {
                Priority::P3
            };
            let permit = match state.scheduler.try_acquire(priority, max_bytes) {
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
            let lcu_client = state.lcu.clone();
            let bytes = if let Some(path) = args.get("path").and_then(Value::as_str) {
                if !path.starts_with('/') || path.contains("://") {
                    drop(permit);
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("InvalidParams", "only LCU asset paths are allowed"),
                    );
                }
                let Some(client) = lcu_client else {
                    drop(permit);
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("NotConnected", "LCU is not connected"),
                    );
                };
                client.get_bytes(path, max_bytes).await
            } else if let Some(url) = args.get("url").and_then(Value::as_str) {
                if state.asset.is_none() {
                    match AssetClient::new(
                        None,
                        vec![
                            "raw.communitydragon.org".to_string(),
                            "lol-api-champion.op.gg".to_string(),
                            "game.gtimg.cn".to_string(),
                        ],
                    ) {
                        Ok(client) => state.asset = Some(client),
                        Err(error) => {
                            drop(permit);
                            state.metrics.errors += 1;
                            return Message::response_error(
                                id,
                                epoch,
                                RpcError::new("AssetError", error.to_string()),
                            );
                        }
                    }
                }
                state.asset.as_ref().unwrap().download(url, max_bytes).await
            } else {
                drop(permit);
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "path or url is required"),
                );
            };
            drop(permit);
            let bytes = match bytes {
                Ok(bytes) => bytes,
                Err(error) => {
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("AssetError", error.to_string()).retryable(true),
                    );
                }
            };
            if let Some(parent) = destination.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    state.metrics.errors += 1;
                    return Message::response_error(
                        id,
                        epoch,
                        RpcError::new("IoError", error.to_string()),
                    );
                }
            }
            let temporary = destination.with_file_name(format!("{name}.tmp"));
            if let Err(error) = fs::write(&temporary, &bytes).and_then(|_| {
                if destination.exists() {
                    fs::remove_file(&destination)?;
                }
                fs::rename(&temporary, &destination)
            }) {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("IoError", error.to_string()),
                );
            }
            Ok(json!({ "path": destination }))
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
            let args = message.args.clone().unwrap_or(Value::Null);
            let Some(session_id) = args.get("sessionId").and_then(Value::as_str) else {
                state.metrics.errors += 1;
                return Message::response_error(
                    id,
                    epoch,
                    RpcError::new("InvalidParams", "sessionId is required"),
                );
            };
            let config = args.get("config").cloned().unwrap_or(Value::Null);
            match &mut state.automation {
                None => {
                    state.automation = Some(DryRunAutomation::new(state.epoch, session_id, config));
                }
                Some(automation) => {
                    automation.reset_session(session_id);
                    automation.update_config(config);
                }
            }
            if let Some(queue_id) = args.get("queueId").and_then(Value::as_u64) {
                if let Some(automation) = state.automation.as_mut() {
                    automation.set_queue_id(Some(queue_id));
                }
            }
            Ok(json!({ "configured": true }))
        }
        "automation.dry_run" => {
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
        "opgg.positions" => {
            let (region, mode, tier, _) = match opgg_params(message) {
                Ok(params) => params,
                Err(error) => return Message::response_error(id, epoch, error),
            };
            let champion_id = message
                .args
                .as_ref()
                .and_then(|args| args.get("championId"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let url =
                format!("https://lol-api-champion.op.gg/api/{region}/champions/{mode}?tier={tier}");
            let raw = match state.fetch_opgg(url).await {
                Ok(raw) => raw,
                Err(error) => return Message::response_error(id, epoch, error),
            };
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
            let (region, mode, tier, _) = match opgg_params(message) {
                Ok(params) => params,
                Err(error) => return Message::response_error(id, epoch, error),
            };
            let url =
                format!("https://lol-api-champion.op.gg/api/{region}/champions/{mode}?tier={tier}");
            let raw = match state.fetch_opgg(url).await {
                Ok(raw) => raw,
                Err(error) => return Message::response_error(id, epoch, error),
            };
            let mut data = if mode == "ranked" {
                parse_ranked_tier_list(&raw)
            } else {
                parse_other_tier_list(&raw)
            };
            state.resolve_opgg_icons(&mut data).await;
            Ok(json!({
                "data": data,
                "version": raw.get("meta").and_then(|meta| meta.get("version")).cloned().unwrap_or(Value::Null),
            }))
        }
        "opgg.champion_build" => {
            let (region, mode, tier, mut position) = match opgg_params(message) {
                Ok(params) => params,
                Err(error) => return Message::response_error(id, epoch, error),
            };
            let champion_id = message
                .args
                .as_ref()
                .and_then(|args| args.get("championId"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let tier_url =
                format!("https://lol-api-champion.op.gg/api/{region}/champions/{mode}?tier={tier}");
            let tier_raw = match state.fetch_opgg(tier_url).await {
                Ok(raw) => raw,
                Err(error) => return Message::response_error(id, epoch, error),
            };
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
            let raw = match state.fetch_opgg(url).await {
                Ok(raw) => raw,
                Err(error) => return Message::response_error(id, epoch, error),
            };
            let mut data = if mode == "arena" {
                parse_arena_champion_build(&raw)
            } else {
                parse_other_champion_build(&raw, &position)
            };
            state.resolve_opgg_icons(&mut data).await;
            Ok(json!({
                "data": data,
                "version": raw.get("meta").and_then(|meta| meta.get("version")).cloned().unwrap_or(Value::Null),
                "mode": mode,
            }))
        }
        "champion_alias.refresh" | "aram_buff.refresh" => {
            let kind = if op == "champion_alias.refresh" {
                "champion_alias"
            } else {
                "aram_buff"
            };
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
            state.refresh_community(kind, force, game_version).await
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
            Ok(json!({
                "metrics": state.metrics,
                "opggCache": {
                    "hits": opgg_cache.hits,
                    "coalesced": opgg_cache.coalesced,
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
                }),
                "clients": json!({
                    "lcu": state.lcu.is_some(),
                    "sgp": state.sgp.is_some(),
                    "external": state.external.is_some(),
                    "asset": state.asset.is_some(),
                }),
                "websocket": {
                    "received": state.ws_stats.received.load(Ordering::Relaxed),
                    "dropped": state.ws_stats.dropped.load(Ordering::Relaxed),
                    "reconnects": state.ws_stats.reconnects.load(Ordering::Relaxed),
                    "snapshots": state.ws_stats.snapshots.load(Ordering::Relaxed),
                    "queued": state.ws_events.as_ref().map(|events| events.len()).unwrap_or(0),
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

fn requires_champ_select_guard(path: &str) -> bool {
    path.starts_with("/lol-champ-select/v1/session/")
        && (path.contains("/actions/")
            || path.ends_with("/my-selection")
            || ((path.contains("/trades/") || path.contains("/swaps/"))
                && path.ends_with("/accept")))
}

async fn write_lcu(
    state: &mut State,
    epoch: u64,
    method: &str,
    path: &str,
    operation_id: &str,
    data: Option<Value>,
) -> Result<Value, RpcError> {
    let key = (epoch, operation_id.to_string());
    if let Some(outcome) = state.write_ledger.get(&key) {
        return match outcome {
            AutomationState::Completed => Ok(json!({
                "replayed": true,
                "state": "completed",
                "data": Value::Null
            })),
            AutomationState::Failed => Err(RpcError::new(
                "OutcomeUnknown",
                "previous write failed; inspect LCU state before retrying",
            )),
            _ => Err(RpcError::new(
                "OutcomeUnknown",
                "previous write outcome is unknown",
            )),
        };
    }
    if state.write_ledger.len() >= 256 {
        return Err(RpcError::new("ResourceDeferred", "write ledger is full").retryable(true));
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
    state
        .write_ledger
        .insert(key.clone(), AutomationState::Writing);
    let write_data = data.clone();
    let result = match method {
        "POST" => client.post_json(path, write_data).await,
        "PUT" => client.put_json(path, write_data).await,
        "PATCH" => client.patch_json(path, write_data).await,
        "DELETE" => client.delete(path).await,
        _ => unreachable!(),
    };
    match result {
        Ok(response_data) => {
            if guarded {
                let read_permit = match state.scheduler.try_acquire(Priority::P1, 64 * 1024) {
                    Ok(permit) => permit,
                    Err(error) => {
                        state
                            .write_ledger
                            .insert(key, AutomationState::OutcomeUnknown);
                        return Err(RpcError::new(
                            "OutcomeUnknown",
                            format!("write confirmation was deferred: {error:?}"),
                        ));
                    }
                };
                let confirmation = client
                    .get_json_fresh("/lol-champ-select/v1/session")
                    .await
                    .map_err(|error| RpcError::new("LcuError", error.to_string()))
                    .and_then(|session| {
                        confirm_write(&session, path, data.as_ref())
                            .map_err(|error| RpcError::new("InvalidState", error.as_str()))
                    });
                drop(read_permit);
                if let Err(error) = confirmation {
                    state
                        .write_ledger
                        .insert(key, AutomationState::OutcomeUnknown);
                    return Err(RpcError::new(
                        "OutcomeUnknown",
                        format!("write needs inspection: {}", error.message),
                    ));
                }
            }
            state.write_ledger.insert(key, AutomationState::Completed);
            Ok(json!({ "replayed": false, "state": "completed", "data": response_data }))
        }
        Err(error) => {
            state.write_ledger.insert(key, AutomationState::Failed);
            Err(RpcError::new("LcuError", error.to_string()))
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

impl State {
    async fn cache_icon(
        &mut self,
        folder: &str,
        name: &str,
        path: Option<&str>,
        url: Option<&str>,
        max_bytes: usize,
    ) -> Result<String, RpcError> {
        let destination = safe_cache_path(&self.cache_root, folder, name)?;
        if destination.is_file() {
            return Ok(destination.to_string_lossy().to_string());
        }
        let _permit = self
            .scheduler
            .try_acquire(Priority::P2, max_bytes)
            .map_err(|error| RpcError::new("ResourceDeferred", format!("{error:?}")))?;
        let bytes = if let Some(path) = path {
            let client = self
                .lcu
                .clone()
                .ok_or_else(|| RpcError::new("NotConnected", "LCU is not connected"))?;
            client
                .get_bytes(path, max_bytes)
                .await
                .map_err(|error| RpcError::new("AssetError", error.to_string()))?
        } else if let Some(url) = url {
            if self.asset.is_none() {
                self.asset = Some(
                    AssetClient::new(
                        self.external_proxy.as_deref(),
                        vec![
                            "raw.communitydragon.org".to_string(),
                            "lol-api-champion.op.gg".to_string(),
                            "game.gtimg.cn".to_string(),
                        ],
                    )
                    .map_err(|error| RpcError::new("AssetError", error.to_string()))?,
                );
            }
            self.asset
                .as_ref()
                .unwrap()
                .download(url, max_bytes)
                .await
                .map_err(|error| RpcError::new("AssetError", error.to_string()))?
        } else {
            return Err(RpcError::new("InvalidParams", "icon source is missing"));
        };
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| RpcError::new("IoError", error.to_string()))?;
        }
        let temporary = destination.with_file_name(format!("{name}.tmp"));
        fs::write(&temporary, bytes)
            .and_then(|_| {
                if destination.exists() {
                    fs::remove_file(&destination)?;
                }
                fs::rename(&temporary, &destination)
            })
            .map_err(|error| RpcError::new("IoError", error.to_string()))?;
        Ok(destination.to_string_lossy().to_string())
    }

    async fn resolve_icon(&mut self, spec: String) -> String {
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

    async fn refresh_community(
        &mut self,
        kind: &str,
        force: bool,
        game_version: Option<&str>,
    ) -> Result<Value, RpcError> {
        let filename = community_filename(kind)
            .ok_or_else(|| RpcError::new("InvalidParams", "unknown community data kind"))?;
        let path = self.data_root.join(filename);
        let mut cached = load_community_cache(&path, "champions");
        if kind == "champion_alias" && cached.is_none() {
            cached = self
                .champion_aliases
                .as_ref()
                .map(ChampionAliasCache::to_value);
        }
        if !force {
            if let (Some(game_version), Some(data)) = (game_version, cached.as_ref()) {
                if data.get("version").and_then(Value::as_str) == Some(game_version) {
                    if kind == "champion_alias" {
                        self.cache_champion_aliases(data);
                    }
                    return Ok(json!({
                        "available": true, "updated": false, "data": data, "warning": Value::Null
                    }));
                }
            }
        }

        if kind == "aram_buff"
            && env::var("SERAPHINE_ARAM_APPSECRET")
                .map(|value| value.is_empty())
                .unwrap_or(true)
        {
            if let Some(data) = cached {
                return Ok(json!({
                    "available": true,
                    "updated": false,
                    "data": data,
                    "warning": "ARAM appsecret is not configured; cached data retained"
                }));
            }
            return Ok(json!({
                "available": false,
                "updated": false,
                "data": Value::Null,
                "warning": "ARAM appsecret is not configured"
            }));
        }

        if self.external.is_none() {
            self.external = Some(
                ExternalClient::new(self.external_proxy.as_deref())
                    .map_err(|error| RpcError::new("ConnectionError", error.to_string()))?,
            );
        }
        let max_bytes = if kind == "champion_alias" {
            1024 * 1024
        } else {
            2 * 1024 * 1024
        };
        let permit = self
            .scheduler
            .try_acquire(Priority::P3, max_bytes)
            .map_err(|error| RpcError::new("ResourceDeferred", format!("{error:?}")))?;
        let fetched = if kind == "champion_alias" {
            self.external
                .as_ref()
                .unwrap()
                .get_legacy_json(CHAMPION_ALIAS_URL, "game.gtimg.cn", &json!({}), max_bytes)
                .await
                .map_err(|_| "upstream request failed")
                .and_then(|raw| parse_champion_aliases(&raw).map_err(|_| "invalid alias data"))
        } else {
            let params = json!({
                "appid": 1,
                "appsecret": env::var("SERAPHINE_ARAM_APPSECRET").unwrap_or_default(),
                "s": "news",
                "c": "search",
                "api_call_function": "module_list",
                "pagesize": "200"
            });
            self.external
                .as_ref()
                .unwrap()
                .get_legacy_json(ARAM_URL, "www.jddld.com", &params, max_bytes)
                .await
                .map_err(|_| "upstream request failed")
                .and_then(|raw| parse_aram_buffs(&raw).map_err(|_| "invalid ARAM data"))
        };
        drop(permit);

        let data = match fetched {
            Ok(data) => data,
            Err(error) => {
                if let Some(data) = cached {
                    if kind == "champion_alias" {
                        self.cache_champion_aliases(&data);
                    }
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
        if kind == "champion_alias" {
            self.cache_champion_aliases(&data);
        }
        Ok(json!({
            "available": true, "updated": true, "data": data, "warning": Value::Null
        }))
    }

    async fn fetch_opgg(&mut self, url: String) -> Result<Value, RpcError> {
        loop {
            match self.opgg_cache.start(&url) {
                Start::Cached(value) => return Ok(value),
                Start::Wait(notify) => {
                    notify.notified().await;
                }
                Start::Leader => break,
            }
        }
        if self.external.is_none() {
            match ExternalClient::new(self.external_proxy.as_deref()) {
                Ok(client) => self.external = Some(client),
                Err(error) => {
                    self.opgg_cache.fail(&url);
                    return Err(RpcError::new("ConnectionError", error.to_string()));
                }
            }
        }
        let max_bytes = 2 * 1024 * 1024;
        let _permit = match self.scheduler.try_acquire(Priority::P2, max_bytes) {
            Ok(permit) => permit,
            Err(error) => {
                self.opgg_cache.fail(&url);
                return Err(RpcError::new("ResourceDeferred", format!("{error:?}")));
            }
        };
        let result = self
            .external
            .as_ref()
            .unwrap()
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
                Ok(value)
            }
            Err(error) => {
                self.opgg_cache.fail(&url);
                Err(error)
            }
        }
    }
}

#[async_trait::async_trait]
impl seraphine_domain::opgg::IconResolver for State {
    async fn resolve(&mut self, spec: String) -> String {
        self.resolve_icon(spec).await
    }
}

async fn receive_ws_event(state: &mut State) -> Option<LcuEvent> {
    state.ws_events.as_mut()?.recv().await
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
    let mut reader = stdin;
    let writer = Arc::new(Mutex::new(stdout));
    let mut state = State::new();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;

    write_message(&writer, &hello_message(0)).await?;

    loop {
        tokio::select! {
            message = read_message(&mut reader) => {
                let Some(message) = message? else { break };
                if let Err(error) = message.validate() {
                    let response = Message::response_error(
                        message.id.unwrap_or(0),
                        message.epoch.unwrap_or(state.epoch),
                        RpcError::new("ProtocolError", error.to_string()),
                    );
                    write_message(&writer, &response).await?;
                    continue;
                }

                match message.kind {
                    MessageKind::Hello => {
                        state.epoch = state.epoch.max(message.epoch.unwrap_or(0));
                        write_message(&writer, &hello_message(state.epoch)).await?;
                    }
                    MessageKind::Request => {
                        let started = Instant::now();
                        let response = handle_request(&message, &mut state).await;
                        write_message(&writer, &response).await?;
                        state.metrics.record_latency(started.elapsed().as_micros() as u64);
                    }
                    MessageKind::Ping => {
                        write_message(&writer, &Message::pong(message.nonce.unwrap_or(0))).await?;
                    }
                    MessageKind::Cancel => {}
                    MessageKind::Shutdown => break,
                    _ => {}
                }
            }
            event = receive_ws_event(&mut state), if state.ws_events.is_some() => {
                if let Some(event) = event {
                    state.metrics.events += 1;
                    let topic = event_topic(&event);
                    if topic == "champselect.changed" {
                        if let Some(automation) = state.automation.as_mut() {
                            let plans = automation.handle_event(&event.data);
                            if !plans.is_empty() {
                                let message = Message::event(
                                    state.epoch,
                                    event.seq,
                                    "automation.planned",
                                    json!({ "plans": plans }),
                                );
                                write_message(&writer, &message).await?;
                            }
                        }
                    }
                    if topic == "entitlements.changed" {
                        if let Some(token) = event.data.get("accessToken").and_then(Value::as_str) {
                            state.sgp_token = Some(Zeroizing::new(token.to_string()));
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
                    let message = Message::event(
                        state.epoch,
                        event.seq,
                        topic,
                        data,
                    );
                    write_message(&writer, &message).await?;
                }
            }
            _ = heartbeat.tick() => {}
        }
    }

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
