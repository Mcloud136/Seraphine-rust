#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod automation;
mod window_fix;

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, LogicalSize, Manager, State};

use seraphine_core::aram::AramData;
use seraphine_core::catalog::{asset_data_url, Catalog};
use seraphine_core::live::LiveClient;
use seraphine_core::opgg::OpggService;
use seraphine_core::writes::WriteService;
use seraphine_core::{career, AppConfig, AppPaths, ConfigStore, SessionManager, SessionStatus};
use seraphine_platform::{list_league_client_pids, read_process_command_line};

pub(crate) struct AppState {
    paths: AppPaths,
    pub(crate) config: ConfigStore,
    pub(crate) session: Arc<SessionManager>,
    catalog: tokio::sync::RwLock<Catalog>,
    aram: tokio::sync::RwLock<AramData>,
    opgg: tokio::sync::RwLock<OpggService>,
    live: LiveClient,
    pub(crate) writes: WriteService,
    pub(crate) automation: Mutex<automation::AutomationState>,
    pub(crate) automation_lock: tokio::sync::Mutex<()>,
    assets: tokio::sync::Mutex<HashMap<String, String>>,
    close_to_tray: AtomicBool,
    reconnect_running: AtomicBool,
}

fn detail(error: impl std::fmt::Display) -> String {
    error.to_string()
}

struct FileLogger {
    file: Mutex<std::fs::File>,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let Ok(mut file) = self.file.lock() else {
            return;
        };
        let _ = writeln!(
            file,
            "{} [{}] {}: {}",
            humantime::format_rfc3339(std::time::SystemTime::now()),
            record.level(),
            record.target(),
            record.args()
        );
        let _ = file.flush();
    }

    fn flush(&self) {}
}

fn init_logger(paths: &AppPaths, level: i64) {
    let _ = paths.ensure();
    let path = paths.log_dir().join("seraphine.log");
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 5 * 1024 * 1024) {
        let _ = std::fs::rename(&path, paths.log_dir().join("seraphine.log.1"));
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let logger = FileLogger {
        file: Mutex::new(file),
    };
    let logger: &'static FileLogger = Box::leak(Box::new(logger));
    if log::set_logger(logger).is_ok() {
        let level = if level <= 10 {
            log::LevelFilter::Debug
        } else if level <= 20 {
            log::LevelFilter::Info
        } else {
            log::LevelFilter::Warn
        };
        log::set_max_level(level);
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default_hook(info);
    }));
}

fn parse_window_size(value: &str) -> Option<(f64, f64)> {
    let (width, height) = value.split_once(',')?;
    let width = width.trim().parse::<f64>().ok()?;
    let height = height.trim().parse::<f64>().ok()?;
    (width >= 640.0 && height >= 480.0).then_some((width, height))
}

fn window_size_value(scale: f64, width: u32, height: u32) -> Value {
    let width = (width as f64 / scale).round() as u32;
    let height = (height as f64 / scale).round() as u32;
    json!({ "personalization": { "windowSize": format!("{width}, {height}") } })
}

fn persist_window_size(handle: tauri::AppHandle, scale: f64, width: u32, height: u32) {
    let patch = window_size_value(scale, width, height);
    tauri::async_runtime::spawn(async move {
        let state = handle.state::<AppState>();
        let _ = state.config.patch(patch).await;
    });
}

fn show_opgg_window(app: &tauri::AppHandle, always_on_top: bool) {
    if let Some(window) = app.get_webview_window("opgg") {
        let _ = window.set_always_on_top(always_on_top);
        let _ = window.show();
        return;
    }
    let builder =
        tauri::WebviewWindowBuilder::new(app, "opgg", tauri::WebviewUrl::App("index.html".into()))
            .title("Seraphine · OP.GG")
            .inner_size(520.0, 760.0)
            .min_inner_size(360.0, 480.0)
            .always_on_top(always_on_top);
    match builder.build() {
        Ok(window) => {
            let _ = window.show();
            let _ = window.set_focus();
        }
        Err(error) => log::warn!("failed to open OP.GG window: {error}"),
    }
}

fn hide_opgg_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("opgg") {
        let _ = window.hide();
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn spawn_auto_reconnect(app: tauri::AppHandle) {
    if app
        .state::<AppState>()
        .reconnect_running
        .swap(true, Ordering::SeqCst)
    {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            {
                let state = app.state::<AppState>();
                let config = state.config.snapshot().await;
                if !config.functions.enable_auto_reconnect
                    || !state.session.status().await.connected
                {
                    break;
                }
                let phase = state
                    .session
                    .lcu_get_fresh("/lol-gameflow/v1/gameflow-phase")
                    .await
                    .unwrap_or(Value::Null);
                if phase.as_str() != Some("Reconnect") {
                    break;
                }
                let operation_id = format!("auto-reconnect:{}", unix_millis());
                if let Err(error) = state
                    .writes
                    .execute(&operation_id, "POST", "/lol-gameflow/v1/reconnect", None)
                    .await
                {
                    log::warn!("auto reconnect failed: {error}");
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        app.state::<AppState>()
            .reconnect_running
            .store(false, Ordering::SeqCst);
    });
}

#[tauri::command]
fn set_window_size(window: tauri::WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    if width < 640.0 || height < 480.0 {
        return Err("window size must be at least 640x480".into());
    }
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(detail)
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn config_get(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.config.snapshot().await)
}

#[tauri::command]
async fn config_patch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    patch: Value,
) -> Result<AppConfig, String> {
    let config = state.config.patch(patch).await.map_err(detail)?;
    state
        .close_to_tray
        .store(config.general.enable_close_to_tray, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("opgg") {
        let _ = window.set_always_on_top(config.functions.enable_opgg_on_top);
        if !config.functions.auto_show_opgg {
            let _ = window.hide();
        }
    }
    Ok(config)
}

#[tauri::command]
fn set_mica(window: tauri::WebviewWindow, enabled: bool, dark: bool) -> Result<(), String> {
    if enabled {
        window_vibrancy::apply_mica(&window, Some(dark)).map_err(detail)
    } else {
        window_vibrancy::clear_mica(&window).map_err(detail)
    }
}

#[tauri::command]
async fn session_status(state: State<'_, AppState>) -> Result<SessionStatus, String> {
    Ok(state.session.status().await)
}

#[tauri::command]
async fn lcu_clients() -> Result<Value, String> {
    let pids = list_league_client_pids().map_err(detail)?;
    let mut clients = Vec::new();
    for pid in pids {
        if let Ok(command_line) = read_process_command_line(pid) {
            clients.push(json!({
                "pid": pid,
                "port": command_line.port,
                "server": command_line.server.unwrap_or_default(),
            }));
        }
    }
    Ok(json!(clients))
}

#[tauri::command]
async fn session_connect(
    state: State<'_, AppState>,
    pid: Option<u32>,
) -> Result<seraphine_core::session::SessionInfo, String> {
    state.session.set_auto_connect(true);
    state.session.connect(pid).await.map_err(detail)
}

#[tauri::command]
async fn session_disconnect(state: State<'_, AppState>) -> Result<(), String> {
    state.session.set_auto_connect(false);
    state.session.disconnect("user").await;
    Ok(())
}

#[tauri::command]
async fn career_overview(state: State<'_, AppState>) -> Result<Value, String> {
    let config = state.config.snapshot().await;
    let limit = config.functions.career_games_number.clamp(1, 20) as u32;
    let catalog = state.catalog.read().await.clone();
    career::overview(&state.session, &catalog, limit)
        .await
        .map_err(detail)
}

#[tauri::command]
async fn catalog_get(state: State<'_, AppState>) -> Result<Value, String> {
    Ok(state.catalog.read().await.web_snapshot())
}

#[tauri::command]
async fn player_overview(state: State<'_, AppState>, query: String) -> Result<Value, String> {
    let summoner = career::resolve_summoner(&state.session, &query)
        .await
        .map_err(detail)?;
    if summoner.get("errorCode").is_some() {
        return Err("SummonerNotFound".into());
    }
    let puuid = summoner
        .get("puuid")
        .and_then(Value::as_str)
        .ok_or("SummonerNotFound")?
        .to_string();
    let config = state.config.snapshot().await;
    let limit = config.functions.career_games_number.clamp(1, 20) as u32;
    let catalog = state.catalog.read().await.clone();
    career::overview_for_puuid(&state.session, &catalog, &puuid, limit, Some(summoner))
        .await
        .map_err(detail)
}

#[tauri::command]
async fn matches_page(
    state: State<'_, AppState>,
    puuid: String,
    begin: u32,
    end: u32,
) -> Result<Value, String> {
    let catalog = state.catalog.read().await.clone();
    career::matches_page(&state.session, &catalog, &puuid, begin, end)
        .await
        .map_err(detail)
}

#[tauri::command]
async fn game_detail(state: State<'_, AppState>, game_id: u64) -> Result<Value, String> {
    career::match_detail(&state.session, game_id)
        .await
        .map_err(detail)
}

#[tauri::command]
async fn gameinfo_fetch(state: State<'_, AppState>, side: String) -> Result<Value, String> {
    let catalog = state.catalog.read().await.clone();
    let config = state.config.snapshot().await;
    let filter = config.functions.queue_filter.clone();
    let queue_filter = if filter.is_empty() {
        None
    } else {
        Some(&filter)
    };
    seraphine_core::gameinfo::fetch(&state.session, &catalog, &side, queue_filter)
        .await
        .map_err(detail)
}

#[tauri::command]
async fn champselect_state(state: State<'_, AppState>) -> Result<Value, String> {
    state
        .session
        .lcu_get_optional("/lol-champ-select/v1/session")
        .await
        .map(|session| session.unwrap_or(Value::Null))
        .map_err(detail)
}

#[tauri::command]
async fn live_game(state: State<'_, AppState>) -> Result<Value, String> {
    let catalog = state.catalog.read().await.clone();
    state
        .live
        .snapshot(&catalog)
        .await
        .map(|snapshot| snapshot.unwrap_or(Value::Null))
        .map_err(detail)
}

#[tauri::command]
async fn fix_client_window(state: State<'_, AppState>) -> Result<Value, String> {
    let zoom = state
        .session
        .lcu_get_fresh("/riotclient/zoom-scale")
        .await
        .ok()
        .and_then(|value| value.as_f64())
        .unwrap_or(1.0);
    window_fix::launch_elevated(zoom)?;
    Ok(json!({ "launched": true, "zoom": zoom }))
}

#[tauri::command]
fn persisted_settings_path(state: &AppState) -> Option<std::path::PathBuf> {
    let config = state.config.peek()?;
    let root = config.general.lol_path.first()?;
    Some(std::path::Path::new(root).join("../Game/Config/PersistedSettings.json"))
}

#[tauri::command]
fn lock_config_state(state: State<'_, AppState>) -> Value {
    match persisted_settings_path(&state) {
        Some(path) if path.is_file() => json!({
            "available": true,
            "locked": path
                .metadata()
                .map(|metadata| metadata.permissions().readonly())
                .unwrap_or(false),
        }),
        _ => json!({ "available": false, "locked": false }),
    }
}

#[tauri::command]
fn set_lock_config(state: State<'_, AppState>, enabled: bool) -> Result<bool, String> {
    let path = persisted_settings_path(&state)
        .filter(|path| path.is_file())
        .ok_or("PersistedSettings.json not found")?;
    let mut permissions = path.metadata().map_err(detail)?.permissions();
    permissions.set_readonly(enabled);
    std::fs::set_permissions(&path, permissions).map_err(detail)?;
    path.metadata()
        .map(|metadata| metadata.permissions().readonly())
        .map_err(detail)
}

#[tauri::command]
async fn auxiliary_read(state: State<'_, AppState>, path: String) -> Result<Value, String> {
    const ALLOWED: [&str; 3] = [
        "/lol-chat/v1/me",
        "/lol-regalia/v2/current-summoner/regalia",
        "/lol-summoner/v1/current-summoner/summoner-profile",
    ];
    if !ALLOWED.contains(&path.as_str()) {
        return Err("read path is not allowed".into());
    }
    state.session.lcu_get_fresh(&path).await.map_err(detail)
}

#[tauri::command]
async fn aux_spectate(
    state: State<'_, AppState>,
    operation_id: String,
    name: String,
) -> Result<Value, String> {
    let summoner = career::resolve_summoner(&state.session, &name)
        .await
        .map_err(detail)?;
    let puuid = summoner
        .get("puuid")
        .and_then(Value::as_str)
        .ok_or("SummonerNotFound")?
        .to_string();
    state
        .writes
        .execute(
            &operation_id,
            "POST",
            "/lol-spectator/v1/spectate/launch",
            Some(json!({
                "allowObserveMode": "ALL",
                "dropInSpectateGameId": name,
                "gameQueueType": "",
                "puuid": puuid,
            })),
        )
        .await
        .map_err(detail)
}

#[tauri::command]
async fn write_lcu(
    state: State<'_, AppState>,
    operation_id: String,
    method: String,
    path: String,
    data: Option<Value>,
) -> Result<Value, String> {
    state
        .writes
        .execute(&operation_id, &method, &path, data)
        .await
        .map_err(detail)
}

#[tauri::command]
fn ui_log(message: String) {
    log::warn!("ui: {message}");
}

#[tauri::command]
async fn champion_skins(state: State<'_, AppState>, champion: String) -> Result<Value, String> {
    let catalog = state.catalog.read().await.clone();
    Ok(catalog.champion_skins(&champion))
}

#[tauri::command]
async fn aram_champion(
    state: State<'_, AppState>,
    champion_id: u64,
) -> Result<Option<Value>, String> {
    Ok(state.aram.read().await.champion(champion_id))
}

#[tauri::command]
async fn aram_meta(state: State<'_, AppState>) -> Result<Value, String> {
    let aram = state.aram.read().await;
    Ok(serde_json::json!({
        "version": aram.version(),
        "source": aram.source(),
    }))
}

#[tauri::command]
async fn aram_update(
    state: State<'_, AppState>,
    force: Option<bool>,
) -> Result<seraphine_core::aram::AramUpdate, String> {
    let config = state.config.snapshot().await;
    let current = state.aram.read().await.version().to_string();
    let proxy =
        if config.general.enable_github_proxy && !config.general.github_proxy_addr.is_empty() {
            Some(config.general.github_proxy_addr.clone())
        } else {
            None
        };
    let outcome = seraphine_core::aram::update(
        &state.paths,
        &config.functions.aram_data_feed_url,
        &current,
        force.unwrap_or(false),
        proxy.as_deref(),
    )
    .await
    .map_err(detail)?;
    if outcome.updated {
        *state.aram.write().await = AramData::load(&state.paths);
    }
    Ok(outcome)
}

#[tauri::command]
async fn tft_recent(state: State<'_, AppState>, count: Option<u32>) -> Result<Value, String> {
    let catalog = state.catalog.read().await.clone();
    seraphine_core::tft::recent(&state.session, &catalog, None, count.unwrap_or(20))
        .await
        .map_err(detail)
}

#[tauri::command]
async fn opgg_tier_list(
    state: State<'_, AppState>,
    region: String,
    mode: String,
    tier: String,
) -> Result<Value, String> {
    let opgg = state.opgg.read().await.clone();
    opgg.tier_list(&region, &mode, &tier).await.map_err(detail)
}

#[tauri::command]
async fn opgg_positions(
    state: State<'_, AppState>,
    region: String,
    mode: String,
    tier: String,
    champion_id: u64,
) -> Result<Value, String> {
    let opgg = state.opgg.read().await.clone();
    opgg.positions(&region, &mode, &tier, champion_id)
        .await
        .map_err(detail)
}

#[tauri::command]
async fn opgg_champion_build(
    state: State<'_, AppState>,
    region: String,
    mode: String,
    tier: String,
    champion_id: u64,
    position: String,
) -> Result<Value, String> {
    let opgg = state.opgg.read().await.clone();
    opgg.champion_build(&region, &mode, &tier, champion_id, &position)
        .await
        .map_err(detail)
}

#[tauri::command]
async fn asset(path: String, state: State<'_, AppState>) -> Result<String, String> {
    {
        let cache = state.assets.lock().await;
        if let Some(url) = cache.get(&path) {
            return Ok(url.clone());
        }
    }
    let url = asset_data_url(&state.session, &path)
        .await
        .map_err(detail)?;
    let mut cache = state.assets.lock().await;
    if cache.len() >= 512 {
        cache.clear();
    }
    cache.insert(path, url.clone());
    Ok(url)
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("--fix-client-window") {
        let zoom = args
            .get(2)
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(1.0);
        window_fix::apply(zoom);
        return;
    }
    let paths = AppPaths::discover();
    let config = ConfigStore::load(&paths).expect("failed to load configuration");
    let startup_config = config.peek().unwrap_or_default();
    init_logger(&paths, startup_config.general.log_level);
    log::info!(
        "starting Seraphine Rust {} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );
    let session = SessionManager::new(paths.clone());
    let writes = WriteService::new(Arc::clone(&session), &paths);
    let catalog = Catalog::load(&paths);
    let aram = AramData::load(&paths);
    let live = LiveClient::new().expect("failed to initialise the live client");
    let proxy = if startup_config.general.enable_opgg_proxy
        && !startup_config.general.opgg_proxy_addr.is_empty()
    {
        Some(startup_config.general.opgg_proxy_addr.clone())
    } else {
        None
    };
    let opgg = OpggService::new(proxy.as_deref(), catalog.clone())
        .or_else(|_| OpggService::new(None, catalog.clone()))
        .expect("failed to initialise the OP.GG client");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            paths: paths.clone(),
            config,
            session: Arc::clone(&session),
            catalog: tokio::sync::RwLock::new(catalog),
            aram: tokio::sync::RwLock::new(aram),
            opgg: tokio::sync::RwLock::new(opgg),
            live,
            writes,
            automation: Mutex::new(automation::AutomationState::default()),
            automation_lock: tokio::sync::Mutex::new(()),
            assets: tokio::sync::Mutex::new(HashMap::new()),
            close_to_tray: AtomicBool::new(startup_config.general.enable_close_to_tray),
            reconnect_running: AtomicBool::new(false),
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let scale = window.scale_factor().unwrap_or(1.0);
                let size = window.inner_size().unwrap_or_default();
                if state.close_to_tray.load(Ordering::SeqCst) {
                    persist_window_size(
                        window.app_handle().clone(),
                        scale,
                        size.width,
                        size.height,
                    );
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    let patch = window_size_value(scale, size.width, size.height);
                    let _ = state.config.patch_blocking(patch);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            config_get,
            config_patch,
            set_window_size,
            app_version,
            set_mica,
            session_status,
            lcu_clients,
            session_connect,
            session_disconnect,
            career_overview,
            catalog_get,
            champion_skins,
            ui_log,
            player_overview,
            matches_page,
            game_detail,
            gameinfo_fetch,
            champselect_state,
            live_game,
            fix_client_window,
            lock_config_state,
            set_lock_config,
            auxiliary_read,
            aux_spectate,
            write_lcu,
            aram_champion,
            aram_meta,
            aram_update,
            tft_recent,
            opgg_tier_list,
            opgg_positions,
            opgg_champion_build,
            asset
        ])
        .setup(move |app| {
            let state = app.state::<AppState>();
            let handle = app.handle().clone();
            let mut events = state.session.subscribe();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = events.recv().await {
                    match &event {
                        seraphine_core::SessionEvent::Connected { info } => {
                            log::info!(
                                "lcu connected pid={} port={} server={}",
                                info.pid,
                                info.port,
                                info.server
                            );
                        }
                        seraphine_core::SessionEvent::Disconnected { reason } => {
                            log::warn!("lcu disconnected: {reason}");
                            hide_opgg_window(&handle);
                        }
                        seraphine_core::SessionEvent::Error { message } => {
                            log::error!("session error: {message}");
                        }
                        seraphine_core::SessionEvent::Lcu {
                            epoch,
                            seq,
                            uri,
                            data,
                            ..
                        } if uri.ends_with("gameflow-phase")
                            && data.as_str() == Some("ReadyCheck") =>
                        {
                            automation::reset(&handle);
                            let accept_handle = handle.clone();
                            let epoch = *epoch;
                            let seq = *seq;
                            tauri::async_runtime::spawn(async move {
                                let state = accept_handle.state::<AppState>();
                                let config = state.config.snapshot().await;
                                if !config.functions.enable_auto_accept_matching {
                                    return;
                                }
                                if config.functions.auto_accept_matching_delay > 0 {
                                    tokio::time::sleep(std::time::Duration::from_secs(
                                        config.functions.auto_accept_matching_delay,
                                    ))
                                    .await;
                                }
                                let phase = state
                                    .session
                                    .lcu_get_fresh("/lol-gameflow/v1/gameflow-phase")
                                    .await
                                    .unwrap_or(Value::Null);
                                if phase.as_str() != Some("ReadyCheck") {
                                    return;
                                }
                                let mut last_error = None;
                                for attempt in 0..3 {
                                    let operation_id =
                                        format!("ready-check:{epoch}:{seq}:{attempt}");
                                    match state
                                        .writes
                                        .execute(
                                            &operation_id,
                                            "POST",
                                            "/lol-matchmaking/v1/ready-check/accept",
                                            None,
                                        )
                                        .await
                                    {
                                        Ok(_) => {
                                            log::info!("auto accepted match");
                                            return;
                                        }
                                        Err(error) => {
                                            last_error = Some(error);
                                            let phase = state
                                                .session
                                                .lcu_get_fresh(
                                                    "/lol-gameflow/v1/gameflow-phase",
                                                )
                                                .await
                                                .unwrap_or(Value::Null);
                                            if phase.as_str() != Some("ReadyCheck") {
                                                log::info!(
                                                    "auto accept skipped: ready check already resolved"
                                                );
                                                return;
                                            }
                                            tokio::time::sleep(
                                                std::time::Duration::from_millis(400),
                                            )
                                            .await;
                                        }
                                    }
                                }
                                if let Some(error) = last_error {
                                    log::warn!("auto accept failed: {error}");
                                }
                            });
                        }
                        seraphine_core::SessionEvent::Lcu {
                            epoch,
                            seq,
                            uri,
                            data,
                            ..
                        } if uri.ends_with("gameflow-phase") => {
                            match data.as_str() {
                                Some("ChampSelect") => {
                                    automation::begin(&handle, *epoch, *seq);
                                    let config =
                                        handle.state::<AppState>().config.snapshot().await;
                                    if config.functions.auto_show_opgg {
                                        show_opgg_window(
                                            &handle,
                                            config.functions.enable_opgg_on_top,
                                        );
                                    }
                                }
                                Some("Reconnect") => {
                                    automation::reset(&handle);
                                    let config =
                                        handle.state::<AppState>().config.snapshot().await;
                                    if config.functions.enable_auto_reconnect {
                                        spawn_auto_reconnect(handle.clone());
                                    }
                                }
                                Some(_) => {
                                    automation::reset(&handle);
                                    hide_opgg_window(&handle);
                                }
                                None => {}
                            }
                        }
                        seraphine_core::SessionEvent::Lcu {
                            epoch,
                            seq,
                            uri,
                            data,
                            ..
                        } if uri.contains("/lol-champ-select/v1/session") => {
                            let app = handle.clone();
                            let epoch = *epoch;
                            let seq = *seq;
                            let data = data.clone();
                            tauri::async_runtime::spawn(async move {
                                automation::on_champ_select(app, epoch, seq, data).await;
                            });
                        }
                        _ => {}
                    }
                    let _ = handle.emit("session://event", &event);
                }
            });
            let monitor_session = Arc::clone(&state.session);
            tauri::async_runtime::spawn(async move {
                monitor_session.monitor().await;
            });

            let automation_poll = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let state = automation_poll.state::<AppState>();
                    let status = state.session.status().await;
                    if !status.connected {
                        continue;
                    }
                    let phase = state
                        .session
                        .lcu_get_fresh("/lol-gameflow/v1/gameflow-phase")
                        .await
                        .unwrap_or(Value::Null);
                    if phase.as_str() != Some("ChampSelect") {
                        continue;
                    }
                    let config = state.config.snapshot().await;
                    if config.functions.auto_show_opgg {
                        show_opgg_window(&automation_poll, config.functions.enable_opgg_on_top);
                    }
                    let session = match state
                        .session
                        .lcu_get_fresh("/lol-champ-select/v1/session")
                        .await
                    {
                        Ok(session) if session.is_object() => session,
                        _ => continue,
                    };
                    automation::on_champ_select(automation_poll.clone(), status.epoch, 0, session)
                        .await;
                }
            });

            let refresh_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let state = refresh_handle.state::<AppState>();
                    if !state.session.status().await.connected {
                        continue;
                    }
                    let catalog = match Catalog::refresh(&state.session, &state.paths).await {
                        Ok(catalog) => catalog,
                        Err(error) => {
                            log::warn!("catalog refresh failed: {error}");
                            continue;
                        }
                    };
                    let config = state.config.snapshot().await;
                    let proxy = if config.general.enable_opgg_proxy
                        && !config.general.opgg_proxy_addr.is_empty()
                    {
                        Some(config.general.opgg_proxy_addr.clone())
                    } else {
                        None
                    };
                    let service = OpggService::new(proxy.as_deref(), catalog.clone())
                        .or_else(|_| OpggService::new(None, catalog.clone()));
                    if let Ok(service) = service {
                        *state.catalog.write().await = catalog;
                        *state.opgg.write().await = service;
                        let _ = refresh_handle.emit("catalog://refreshed", ());
                        break;
                    }
                }
            });

            let aram_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let state = aram_handle.state::<AppState>();
                let config = state.config.snapshot().await;
                if !config.functions.aram_data_auto_update
                    || config.functions.aram_data_feed_url.is_empty()
                {
                    return;
                }
                let current = state.aram.read().await.version().to_string();
                let proxy = if config.general.enable_github_proxy
                    && !config.general.github_proxy_addr.is_empty()
                {
                    Some(config.general.github_proxy_addr.clone())
                } else {
                    None
                };
                match seraphine_core::aram::update(
                    &state.paths,
                    &config.functions.aram_data_feed_url,
                    &current,
                    false,
                    proxy.as_deref(),
                )
                .await
                {
                    Ok(outcome) => {
                        if outcome.updated {
                            *state.aram.write().await = AramData::load(&state.paths);
                            let _ = aram_handle.emit(
                                "aram://updated",
                                serde_json::json!({ "version": outcome.version }),
                            );
                        }
                    }
                    Err(error) => log::warn!("aram update failed: {error}"),
                }
            });

            let config = tauri::async_runtime::block_on(state.config.snapshot());
            if let Some(window) = app.get_webview_window("main") {
                if let Some((width, height)) =
                    parse_window_size(&config.personalization.window_size)
                {
                    let _ = window.set_size(LogicalSize::new(width, height));
                }
            }
            if config.personalization.mica_enabled && seraphine_core::session::is_windows_11() {
                if let Some(window) = app.get_webview_window("main") {
                    let dark = config.theme.theme_mode.eq_ignore_ascii_case("dark");
                    let _ = window_vibrancy::apply_mica(&window, Some(dark));
                }
            }

            let show = MenuItemBuilder::with_id("show", "显示主界面").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;
            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or("missing application icon")?;
            TrayIconBuilder::new()
                .icon(icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let state = app.state::<AppState>();
                            let scale = window.scale_factor().unwrap_or(1.0);
                            let size = window.inner_size().unwrap_or_default();
                            let patch = window_size_value(scale, size.width, size.height);
                            let _ = state.config.patch_blocking(patch);
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Seraphine");
}
