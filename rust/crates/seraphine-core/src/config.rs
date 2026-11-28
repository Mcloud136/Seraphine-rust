use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use crate::error::CoreError;
use crate::paths::AppPaths;

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_WORKSPACE: &str = "app";
pub const DEFAULT_ARAM_FEED_URL: &str =
    "https://gitee.com/wxbns/seraphine-rust/raw/main/aram/aram-manifest.json";
const LEGACY_ARAM_FEED_URLS: [&str; 3] = [
    "https://github.com/Mcloud136/Seraphine-rust/releases/download/aram-data-latest/aram-manifest.json",
    "https://gitee.com/wxbns/seraphine-rust/releases/download/aram-data-latest/aram-manifest.json",
    "https://raw.githubusercontent.com/Mcloud136/Seraphine-rust/main/aram/aram-manifest.json",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub migrated_from: Option<String>,
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub functions: FunctionConfig,
    #[serde(default)]
    pub personalization: PersonalizationConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub other: OtherConfig,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            migrated_from: None,
            general: GeneralConfig::default(),
            functions: FunctionConfig::default(),
            personalization: PersonalizationConfig::default(),
            theme: ThemeConfig::default(),
            other: OtherConfig::default(),
            extra: Map::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralConfig {
    #[serde(default = "default_backend_mode")]
    pub backend_mode: String,
    #[serde(default = "default_true")]
    pub enable_check_update: bool,
    #[serde(default = "default_true")]
    pub enable_close_to_tray: bool,
    #[serde(default)]
    pub enable_game_start_minimize: bool,
    #[serde(default)]
    pub enable_github_proxy: bool,
    #[serde(default)]
    pub enable_opgg_proxy: bool,
    #[serde(default)]
    pub enable_silent: bool,
    #[serde(default)]
    pub enable_start_lol_with_app: bool,
    #[serde(default)]
    pub github_proxy_addr: String,
    #[serde(default = "default_log_level")]
    pub log_level: i64,
    #[serde(default)]
    pub lol_path: Vec<String>,
    #[serde(default)]
    pub opgg_proxy_addr: String,
    #[serde(default = "default_poll_interval")]
    pub process_poll_ms: u64,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            backend_mode: default_backend_mode(),
            enable_check_update: true,
            enable_close_to_tray: true,
            enable_game_start_minimize: false,
            enable_github_proxy: false,
            enable_opgg_proxy: false,
            enable_silent: false,
            enable_start_lol_with_app: false,
            github_proxy_addr: String::new(),
            log_level: default_log_level(),
            lol_path: Vec::new(),
            opgg_proxy_addr: String::new(),
            process_poll_ms: default_poll_interval(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionConfig {
    #[serde(default = "default_api_concurrency")]
    pub api_concurrency_number: u64,
    #[serde(default = "default_true")]
    pub aram_data_auto_update: bool,
    #[serde(default)]
    pub aram_data_feed_url: String,
    #[serde(default)]
    pub auto_accept_ceil_swap: bool,
    #[serde(default)]
    pub auto_accept_champ_trade: bool,
    #[serde(default)]
    pub auto_accept_matching_delay: u64,
    #[serde(default)]
    pub auto_ban_champion: Vec<u64>,
    #[serde(default)]
    pub auto_ban_champion_bot: Vec<u64>,
    #[serde(default)]
    pub auto_ban_champion_jug: Vec<u64>,
    #[serde(default)]
    pub auto_ban_champion_mid: Vec<u64>,
    #[serde(default)]
    pub auto_ban_champion_sup: Vec<u64>,
    #[serde(default)]
    pub auto_ban_champion_top: Vec<u64>,
    #[serde(default)]
    pub auto_ban_delay: u64,
    #[serde(default)]
    pub auto_select_champion: Vec<u64>,
    #[serde(default)]
    pub auto_select_champion_bot: Vec<u64>,
    #[serde(default)]
    pub auto_select_champion_jug: Vec<u64>,
    #[serde(default)]
    pub auto_select_champion_mid: Vec<u64>,
    #[serde(default)]
    pub auto_select_champion_sup: Vec<u64>,
    #[serde(default)]
    pub auto_select_champion_top: Vec<u64>,
    #[serde(default = "default_spells")]
    pub auto_set_summoner_spell: Vec<u64>,
    #[serde(default = "default_spells")]
    pub auto_set_summoner_spell_bot: Vec<u64>,
    #[serde(default = "default_spells")]
    pub auto_set_summoner_spell_jug: Vec<u64>,
    #[serde(default = "default_spells")]
    pub auto_set_summoner_spell_mid: Vec<u64>,
    #[serde(default = "default_spells")]
    pub auto_set_summoner_spell_sup: Vec<u64>,
    #[serde(default = "default_spells")]
    pub auto_set_summoner_spell_top: Vec<u64>,
    #[serde(default = "default_true")]
    pub auto_show_opgg: bool,
    #[serde(default = "default_career_games")]
    pub career_games_number: u64,
    #[serde(default = "default_true")]
    pub enable_auto_accept_matching: bool,
    #[serde(default)]
    pub enable_auto_ban_champion: bool,
    #[serde(default)]
    pub enable_auto_reconnect: bool,
    #[serde(default)]
    pub enable_auto_select_champion: bool,
    #[serde(default)]
    pub enable_auto_select_timeout_completed: bool,
    #[serde(default)]
    pub enable_auto_set_spells: bool,
    #[serde(default)]
    pub enable_opgg_on_top: bool,
    #[serde(default)]
    pub enable_random_skin: bool,
    #[serde(default)]
    pub enable_reserve_gameinfo: bool,
    #[serde(default)]
    pub game_info_filter: bool,
    #[serde(default = "default_opgg_position")]
    pub opgg_position: String,
    #[serde(default = "default_opgg_region")]
    pub opgg_region: String,
    #[serde(default = "default_opgg_tier")]
    pub opgg_tier: String,
    #[serde(default)]
    pub prentent_ban: bool,
    #[serde(default)]
    pub queue_filter: Map<String, Value>,
    #[serde(default)]
    pub show_tier_in_game_info: bool,
}

impl Default for FunctionConfig {
    fn default() -> Self {
        Self {
            api_concurrency_number: default_api_concurrency(),
            aram_data_auto_update: true,
            aram_data_feed_url: DEFAULT_ARAM_FEED_URL.to_string(),
            auto_accept_ceil_swap: false,
            auto_accept_champ_trade: false,
            auto_accept_matching_delay: 0,
            auto_ban_champion: Vec::new(),
            auto_ban_champion_bot: Vec::new(),
            auto_ban_champion_jug: Vec::new(),
            auto_ban_champion_mid: Vec::new(),
            auto_ban_champion_sup: Vec::new(),
            auto_ban_champion_top: Vec::new(),
            auto_ban_delay: 0,
            auto_select_champion: Vec::new(),
            auto_select_champion_bot: Vec::new(),
            auto_select_champion_jug: Vec::new(),
            auto_select_champion_mid: Vec::new(),
            auto_select_champion_sup: Vec::new(),
            auto_select_champion_top: Vec::new(),
            auto_set_summoner_spell: default_spells(),
            auto_set_summoner_spell_bot: default_spells(),
            auto_set_summoner_spell_jug: default_spells(),
            auto_set_summoner_spell_mid: default_spells(),
            auto_set_summoner_spell_sup: default_spells(),
            auto_set_summoner_spell_top: default_spells(),
            auto_show_opgg: true,
            career_games_number: default_career_games(),
            enable_auto_accept_matching: true,
            enable_auto_ban_champion: false,
            enable_auto_reconnect: false,
            enable_auto_select_champion: false,
            enable_auto_select_timeout_completed: false,
            enable_auto_set_spells: false,
            enable_opgg_on_top: false,
            enable_random_skin: false,
            enable_reserve_gameinfo: false,
            game_info_filter: false,
            opgg_position: default_opgg_position(),
            opgg_region: default_opgg_region(),
            opgg_tier: default_opgg_tier(),
            prentent_ban: false,
            queue_filter: empty_queue_filter(),
            show_tier_in_game_info: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalizationConfig {
    #[serde(default = "default_dark_deaths")]
    pub dark_deaths_number_color: String,
    #[serde(default = "default_dpi_scale")]
    pub dpi_scale: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_light_deaths")]
    pub light_deaths_number_color: String,
    #[serde(default = "default_lose_card")]
    pub lose_card_color: String,
    #[serde(default = "default_true")]
    pub mica_enabled: bool,
    #[serde(default = "default_remake_card")]
    pub remake_card_color: String,
    #[serde(default = "default_win_card")]
    pub win_card_color: String,
    #[serde(default = "default_window_size")]
    pub window_size: String,
}

impl Default for PersonalizationConfig {
    fn default() -> Self {
        Self {
            dark_deaths_number_color: default_dark_deaths(),
            dpi_scale: default_dpi_scale(),
            language: default_language(),
            light_deaths_number_color: default_light_deaths(),
            lose_card_color: default_lose_card(),
            mica_enabled: crate::session::is_windows_11(),
            remake_card_color: default_remake_card(),
            win_card_color: default_win_card(),
            window_size: default_window_size(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeConfig {
    #[serde(default = "default_fonts")]
    pub font_families: Vec<String>,
    #[serde(default = "default_theme_color")]
    pub theme_color: String,
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            font_families: default_fonts(),
            theme_color: default_theme_color(),
            theme_mode: default_theme_mode(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherConfig {
    #[serde(default)]
    pub last_notice_sha: String,
    #[serde(default)]
    pub search_history: String,
}

fn default_backend_mode() -> String {
    "rust".into()
}
fn default_true() -> bool {
    true
}
fn default_log_level() -> i64 {
    40
}
fn default_poll_interval() -> u64 {
    1500
}
fn default_api_concurrency() -> u64 {
    1
}
fn default_spells() -> Vec<u64> {
    vec![54, 54]
}
fn default_career_games() -> u64 {
    20
}
fn default_opgg_position() -> String {
    "ADC".into()
}
fn default_opgg_region() -> String {
    "kr".into()
}
fn default_opgg_tier() -> String {
    "emerald_plus".into()
}
fn default_dark_deaths() -> String {
    "#ffff8d8d".into()
}
fn default_light_deaths() -> String {
    "#ffb60000".into()
}
fn default_dpi_scale() -> String {
    "Auto".into()
}
fn default_language() -> String {
    "Auto".into()
}
fn default_lose_card() -> String {
    "#28d3190c".into()
}
fn default_remake_card() -> String {
    "#28a2a2a2".into()
}
fn default_win_card() -> String {
    "#2839b01b".into()
}
fn default_window_size() -> String {
    "1134, 826".into()
}
fn default_fonts() -> Vec<String> {
    vec![
        "Segoe UI".into(),
        "Microsoft YaHei".into(),
        "PingFang SC".into(),
    ]
}
fn default_theme_color() -> String {
    "#ff009faa".into()
}
fn default_theme_mode() -> String {
    "Light".into()
}
fn empty_queue_filter() -> Map<String, Value> {
    let mut map = Map::new();
    for key in ["420", "430", "440", "450", "480", "2400", "4310", "2450"] {
        map.insert(key.to_string(), Value::Array(Vec::new()));
    }
    map
}

pub struct ConfigStore {
    path: PathBuf,
    inner: RwLock<AppConfig>,
}

impl ConfigStore {
    /// Loads the Rust config, migrating the legacy Python config on first run.
    pub fn load(paths: &AppPaths) -> Result<Self, CoreError> {
        paths.ensure().map_err(CoreError::io)?;
        let path = paths.config_file();
        let mut corrupt = false;
        if let Ok(bytes) = std::fs::read(&path) {
            match serde_json::from_slice::<AppConfig>(&bytes) {
                Ok(config) => {
                    return Ok(Self {
                        path,
                        inner: RwLock::new(normalize(config)),
                    });
                }
                Err(error) => {
                    log::warn!("failed to parse config {}: {error}", path.display());
                    match backup_corrupt(&path) {
                        Ok(backup) => {
                            log::warn!("backed up corrupt config to {}", backup.display());
                            corrupt = true;
                        }
                        Err(backup_error) => {
                            log::error!(
                                "failed to back up corrupt config {}: {backup_error}",
                                path.display()
                            );
                        }
                    }
                }
            }
        }

        let legacy_path = paths.legacy_config_file();
        let (config, migrated) = match std::fs::read(&legacy_path) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) => (
                    migrate_legacy(&value, &legacy_path),
                    Some(legacy_path.display().to_string()),
                ),
                Err(_) => (AppConfig::default(), None),
            },
            Err(_) => (AppConfig::default(), None),
        };
        let store = Self {
            path,
            inner: RwLock::new(normalize(config)),
        };
        // Only write defaults when the corrupt file was safely moved aside.
        if migrated.is_some() || corrupt {
            store.save_current()?;
        }
        Ok(store)
    }

    pub async fn snapshot(&self) -> AppConfig {
        self.inner.read().await.clone()
    }

    /// Non-blocking snapshot for synchronous startup code.
    pub fn peek(&self) -> Option<AppConfig> {
        self.inner.try_read().ok().map(|guard| guard.clone())
    }

    pub async fn replace(&self, config: AppConfig) -> Result<AppConfig, CoreError> {
        let mut guard = self.inner.write().await;
        *guard = config;
        let snapshot = guard.clone();
        self.write(&snapshot)?;
        Ok(snapshot)
    }

    pub async fn patch(&self, patch: Value) -> Result<AppConfig, CoreError> {
        let mut guard = self.inner.write().await;
        let mut current = serde_json::to_value(&*guard).map_err(CoreError::json)?;
        merge(&mut current, patch);
        let updated: AppConfig = serde_json::from_value(current)
            .map_err(|error| CoreError::Config(error.to_string()))?;
        *guard = updated.clone();
        self.write(&updated)?;
        Ok(updated)
    }

    /// Synchronous patch for non-async callbacks (tray menu, window events).
    pub fn patch_blocking(&self, patch: Value) -> Result<AppConfig, CoreError> {
        let mut guard = self
            .inner
            .try_write()
            .map_err(|error| CoreError::Config(error.to_string()))?;
        let mut current = serde_json::to_value(&*guard).map_err(CoreError::json)?;
        merge(&mut current, patch);
        let updated: AppConfig = serde_json::from_value(current)
            .map_err(|error| CoreError::Config(error.to_string()))?;
        *guard = updated.clone();
        self.write(&updated)?;
        Ok(updated)
    }

    fn save_current(&self) -> Result<(), CoreError> {
        let snapshot = self
            .inner
            .try_read()
            .map(|guard| guard.clone())
            .map_err(|error| CoreError::Config(error.to_string()))?;
        self.write(&snapshot)
    }

    fn write(&self, config: &AppConfig) -> Result<(), CoreError> {
        let bytes = serde_json::to_vec_pretty(config).map_err(CoreError::json)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(CoreError::io)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, &bytes).map_err(CoreError::io)?;
        seraphine_platform::atomic_replace_file(&temporary, &self.path).map_err(CoreError::io)?;
        Ok(())
    }
}

fn backup_corrupt(path: &Path) -> Result<PathBuf, std::io::Error> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let backup = path.with_file_name(format!("config.json.corrupt-{stamp}"));
    std::fs::rename(path, &backup)?;
    Ok(backup)
}

fn normalize(mut config: AppConfig) -> AppConfig {
    config.schema_version = SCHEMA_VERSION;
    config.general.backend_mode = "rust".into();
    if LEGACY_ARAM_FEED_URLS.contains(&config.functions.aram_data_feed_url.as_str()) {
        config.functions.aram_data_feed_url = DEFAULT_ARAM_FEED_URL.into();
    }
    config
}

fn merge(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                match target.get_mut(&key) {
                    Some(existing) => merge(existing, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (target, patch) => *target = patch,
    }
}

#[derive(Debug, Default, Deserialize)]
struct LegacyRoot {
    #[serde(default, rename = "General")]
    general: LegacyGeneral,
    #[serde(default, rename = "Functions")]
    functions: LegacyFunctions,
    #[serde(default, rename = "Personalization")]
    personalization: LegacyPersonalization,
    #[serde(default, rename = "QFluentWidgets")]
    theme: LegacyTheme,
    #[serde(default, rename = "Other")]
    other: LegacyOther,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyGeneral {
    #[serde(default, rename = "EnableCheckUpdate")]
    enable_check_update: Option<bool>,
    #[serde(default, rename = "EnableCloseToTray", deserialize_with = "opt_bool")]
    enable_close_to_tray: Option<bool>,
    #[serde(default, rename = "EnableGameStartMinimize")]
    enable_game_start_minimize: Option<bool>,
    #[serde(default, rename = "EnableGithubProxy")]
    enable_github_proxy: Option<bool>,
    #[serde(default, rename = "EnableOpggProxy")]
    enable_opgg_proxy: Option<bool>,
    #[serde(default, rename = "EnableSilent")]
    enable_silent: Option<bool>,
    #[serde(default, rename = "EnableStartLolWithApp")]
    enable_start_lol_with_app: Option<bool>,
    #[serde(default, rename = "GithubProxyAddr")]
    github_proxy_addr: Option<String>,
    #[serde(default, rename = "LogLevel")]
    log_level: Option<i64>,
    #[serde(default, rename = "LolPath")]
    lol_path: Option<Vec<String>>,
    #[serde(default, rename = "OpggProxyAddr")]
    opgg_proxy_addr: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyFunctions {
    api_concurrency_number: Option<u64>,
    aram_data_auto_update: Option<bool>,
    aram_data_feed_url: Option<String>,
    auto_accept_ceil_swap: Option<bool>,
    auto_accept_champ_trade: Option<bool>,
    auto_accept_matching_delay: Option<u64>,
    auto_ban_champion: Option<Vec<u64>>,
    auto_ban_champion_bot: Option<Vec<u64>>,
    auto_ban_champion_jug: Option<Vec<u64>>,
    auto_ban_champion_mid: Option<Vec<u64>>,
    auto_ban_champion_sup: Option<Vec<u64>>,
    auto_ban_champion_top: Option<Vec<u64>>,
    auto_ban_delay: Option<u64>,
    auto_select_champion: Option<Vec<u64>>,
    auto_select_champion_bot: Option<Vec<u64>>,
    auto_select_champion_jug: Option<Vec<u64>>,
    auto_select_champion_mid: Option<Vec<u64>>,
    auto_select_champion_sup: Option<Vec<u64>>,
    auto_select_champion_top: Option<Vec<u64>>,
    auto_set_summoner_spell: Option<Vec<u64>>,
    auto_set_summoner_spell_bot: Option<Vec<u64>>,
    auto_set_summoner_spell_jug: Option<Vec<u64>>,
    auto_set_summoner_spell_mid: Option<Vec<u64>>,
    auto_set_summoner_spell_sup: Option<Vec<u64>>,
    auto_set_summoner_spell_top: Option<Vec<u64>>,
    auto_show_opgg: Option<bool>,
    career_games_number: Option<u64>,
    enable_auto_accept_matching: Option<bool>,
    enable_auto_ban_champion: Option<bool>,
    enable_auto_reconnect: Option<bool>,
    enable_auto_select_champion: Option<bool>,
    enable_auto_select_timeout_completed: Option<bool>,
    enable_auto_set_spells: Option<bool>,
    enable_opgg_on_top: Option<bool>,
    #[serde(rename = "enableRandomSkin")]
    enable_random_skin: Option<bool>,
    enable_reserve_gameinfo: Option<bool>,
    game_info_filter: Option<bool>,
    opgg_position: Option<String>,
    opgg_region: Option<String>,
    opgg_tier: Option<String>,
    #[serde(rename = "PrententBan")]
    prentent_ban: Option<bool>,
    queue_filter: Option<Map<String, Value>>,
    show_tier_in_game_info: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyPersonalization {
    dark_deaths_number_color: Option<String>,
    dpi_scale: Option<String>,
    language: Option<String>,
    light_deaths_number_color: Option<String>,
    lose_card_color: Option<String>,
    mica_enabled: Option<bool>,
    remake_card_color: Option<String>,
    win_card_color: Option<String>,
    window_size: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyTheme {
    font_families: Option<Vec<String>>,
    theme_color: Option<String>,
    theme_mode: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyOther {
    last_notice_sha: Option<String>,
    search_history: Option<String>,
}

fn opt_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match value {
        Value::Bool(flag) => Some(flag),
        Value::Number(number) => number.as_i64().map(|number| number != 0),
        _ => None,
    }))
}

fn migrate_legacy(value: &Value, source: &std::path::Path) -> AppConfig {
    let legacy: LegacyRoot = serde_json::from_value(value.clone()).unwrap_or_default();
    let mut config = AppConfig {
        migrated_from: Some(source.display().to_string()),
        ..AppConfig::default()
    };

    let general = &mut config.general;
    assign(
        &mut general.enable_check_update,
        legacy.general.enable_check_update,
    );
    assign(
        &mut general.enable_close_to_tray,
        legacy.general.enable_close_to_tray,
    );
    assign(
        &mut general.enable_game_start_minimize,
        legacy.general.enable_game_start_minimize,
    );
    assign(
        &mut general.enable_github_proxy,
        legacy.general.enable_github_proxy,
    );
    assign(
        &mut general.enable_opgg_proxy,
        legacy.general.enable_opgg_proxy,
    );
    assign(&mut general.enable_silent, legacy.general.enable_silent);
    assign(
        &mut general.enable_start_lol_with_app,
        legacy.general.enable_start_lol_with_app,
    );
    assign(
        &mut general.github_proxy_addr,
        legacy.general.github_proxy_addr,
    );
    assign(&mut general.log_level, legacy.general.log_level);
    assign(&mut general.lol_path, legacy.general.lol_path);
    assign(&mut general.opgg_proxy_addr, legacy.general.opgg_proxy_addr);

    let functions = &mut config.functions;
    assign(
        &mut functions.api_concurrency_number,
        legacy.functions.api_concurrency_number,
    );
    assign(
        &mut functions.aram_data_auto_update,
        legacy.functions.aram_data_auto_update,
    );
    assign(
        &mut functions.aram_data_feed_url,
        legacy.functions.aram_data_feed_url,
    );
    assign(
        &mut functions.auto_accept_ceil_swap,
        legacy.functions.auto_accept_ceil_swap,
    );
    assign(
        &mut functions.auto_accept_champ_trade,
        legacy.functions.auto_accept_champ_trade,
    );
    assign(
        &mut functions.auto_accept_matching_delay,
        legacy.functions.auto_accept_matching_delay,
    );
    assign(
        &mut functions.auto_ban_champion,
        legacy.functions.auto_ban_champion,
    );
    assign(
        &mut functions.auto_ban_champion_bot,
        legacy.functions.auto_ban_champion_bot,
    );
    assign(
        &mut functions.auto_ban_champion_jug,
        legacy.functions.auto_ban_champion_jug,
    );
    assign(
        &mut functions.auto_ban_champion_mid,
        legacy.functions.auto_ban_champion_mid,
    );
    assign(
        &mut functions.auto_ban_champion_sup,
        legacy.functions.auto_ban_champion_sup,
    );
    assign(
        &mut functions.auto_ban_champion_top,
        legacy.functions.auto_ban_champion_top,
    );
    assign(
        &mut functions.auto_ban_delay,
        legacy.functions.auto_ban_delay,
    );
    assign(
        &mut functions.auto_select_champion,
        legacy.functions.auto_select_champion,
    );
    assign(
        &mut functions.auto_select_champion_bot,
        legacy.functions.auto_select_champion_bot,
    );
    assign(
        &mut functions.auto_select_champion_jug,
        legacy.functions.auto_select_champion_jug,
    );
    assign(
        &mut functions.auto_select_champion_mid,
        legacy.functions.auto_select_champion_mid,
    );
    assign(
        &mut functions.auto_select_champion_sup,
        legacy.functions.auto_select_champion_sup,
    );
    assign(
        &mut functions.auto_select_champion_top,
        legacy.functions.auto_select_champion_top,
    );
    assign(
        &mut functions.auto_set_summoner_spell,
        legacy.functions.auto_set_summoner_spell,
    );
    assign(
        &mut functions.auto_set_summoner_spell_bot,
        legacy.functions.auto_set_summoner_spell_bot,
    );
    assign(
        &mut functions.auto_set_summoner_spell_jug,
        legacy.functions.auto_set_summoner_spell_jug,
    );
    assign(
        &mut functions.auto_set_summoner_spell_mid,
        legacy.functions.auto_set_summoner_spell_mid,
    );
    assign(
        &mut functions.auto_set_summoner_spell_sup,
        legacy.functions.auto_set_summoner_spell_sup,
    );
    assign(
        &mut functions.auto_set_summoner_spell_top,
        legacy.functions.auto_set_summoner_spell_top,
    );
    assign(
        &mut functions.auto_show_opgg,
        legacy.functions.auto_show_opgg,
    );
    assign(
        &mut functions.career_games_number,
        legacy.functions.career_games_number,
    );
    assign(
        &mut functions.enable_auto_accept_matching,
        legacy.functions.enable_auto_accept_matching,
    );
    assign(
        &mut functions.enable_auto_ban_champion,
        legacy.functions.enable_auto_ban_champion,
    );
    assign(
        &mut functions.enable_auto_reconnect,
        legacy.functions.enable_auto_reconnect,
    );
    assign(
        &mut functions.enable_auto_select_champion,
        legacy.functions.enable_auto_select_champion,
    );
    assign(
        &mut functions.enable_auto_select_timeout_completed,
        legacy.functions.enable_auto_select_timeout_completed,
    );
    assign(
        &mut functions.enable_auto_set_spells,
        legacy.functions.enable_auto_set_spells,
    );
    assign(
        &mut functions.enable_opgg_on_top,
        legacy.functions.enable_opgg_on_top,
    );
    assign(
        &mut functions.enable_random_skin,
        legacy.functions.enable_random_skin,
    );
    assign(
        &mut functions.enable_reserve_gameinfo,
        legacy.functions.enable_reserve_gameinfo,
    );
    assign(
        &mut functions.game_info_filter,
        legacy.functions.game_info_filter,
    );
    assign(&mut functions.opgg_position, legacy.functions.opgg_position);
    assign(&mut functions.opgg_region, legacy.functions.opgg_region);
    assign(&mut functions.opgg_tier, legacy.functions.opgg_tier);
    assign(&mut functions.prentent_ban, legacy.functions.prentent_ban);
    assign(&mut functions.queue_filter, legacy.functions.queue_filter);
    assign(
        &mut functions.show_tier_in_game_info,
        legacy.functions.show_tier_in_game_info,
    );

    let personalization = &mut config.personalization;
    assign(
        &mut personalization.dark_deaths_number_color,
        legacy.personalization.dark_deaths_number_color,
    );
    assign(
        &mut personalization.dpi_scale,
        legacy.personalization.dpi_scale,
    );
    assign(
        &mut personalization.language,
        legacy.personalization.language,
    );
    assign(
        &mut personalization.light_deaths_number_color,
        legacy.personalization.light_deaths_number_color,
    );
    assign(
        &mut personalization.lose_card_color,
        legacy.personalization.lose_card_color,
    );
    assign(
        &mut personalization.mica_enabled,
        legacy.personalization.mica_enabled,
    );
    assign(
        &mut personalization.remake_card_color,
        legacy.personalization.remake_card_color,
    );
    assign(
        &mut personalization.win_card_color,
        legacy.personalization.win_card_color,
    );
    assign(
        &mut personalization.window_size,
        legacy.personalization.window_size,
    );

    assign(&mut config.theme.font_families, legacy.theme.font_families);
    assign(&mut config.theme.theme_color, legacy.theme.theme_color);
    assign(&mut config.theme.theme_mode, legacy.theme.theme_mode);
    assign(
        &mut config.other.last_notice_sha,
        legacy.other.last_notice_sha,
    );
    assign(
        &mut config.other.search_history,
        legacy.other.search_history,
    );

    config
}

fn assign<T>(target: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *target = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_groups_and_odd_keys() {
        let legacy = serde_json::json!({
            "General": {
                "EnableCloseToTray": 1,
                "LolPath": ["C:/League"],
                "LogLevel": 30
            },
            "Functions": {
                "PrententBan": true,
                "enableRandomSkin": true,
                "AutoBanChampion": [1, 2],
                "QueueFilter": { "420": [] }
            },
            "Personalization": {
                "Language": "zh_CN",
                "MicaEnabled": false
            },
            "QFluentWidgets": {
                "ThemeMode": "Dark",
                "FontFamilies": ["Microsoft YaHei"]
            },
            "Other": {
                "SearchHistory": "player#1234"
            }
        });
        let config = migrate_legacy(&legacy, std::path::Path::new("legacy.json"));

        assert!(config.general.enable_close_to_tray);
        assert_eq!(config.general.lol_path, vec!["C:/League"]);
        assert_eq!(config.general.log_level, 30);
        assert!(config.functions.prentent_ban);
        assert!(config.functions.enable_random_skin);
        assert_eq!(config.functions.auto_ban_champion, vec![1, 2]);
        assert_eq!(config.personalization.language, "zh_CN");
        assert!(!config.personalization.mica_enabled);
        assert_eq!(config.theme.theme_mode, "Dark");
        assert_eq!(config.theme.font_families, vec!["Microsoft YaHei"]);
        assert_eq!(config.other.search_history, "player#1234");
        assert!(config.migrated_from.is_some());
    }

    #[test]
    fn patch_merges_nested_groups() {
        let mut current = serde_json::to_value(AppConfig::default()).unwrap();
        merge(
            &mut current,
            serde_json::json!({ "personalization": { "language": "en_US" } }),
        );
        let updated: AppConfig = serde_json::from_value(current).unwrap();
        assert_eq!(updated.personalization.language, "en_US");
        assert_eq!(
            updated.personalization.dpi_scale,
            AppConfig::default().personalization.dpi_scale
        );
    }

    #[test]
    fn backs_up_corrupt_config_instead_of_overwriting_it() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("seraphine-config-test-{stamp}"));
        let paths = AppPaths {
            legacy_data: root.join("legacy"),
            local: root.join("local"),
            state: root.join("state"),
            cache: root.join("cache"),
            catalog: root.join("catalog"),
        };
        std::fs::create_dir_all(&paths.state).unwrap();
        let config_path = paths.config_file();
        std::fs::write(&config_path, b"{ this is not json").unwrap();
        let store = ConfigStore::load(&paths).unwrap();
        assert_eq!(store.peek().unwrap().schema_version, SCHEMA_VERSION);
        let backups = std::fs::read_dir(&paths.state)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("config.json.corrupt-"))
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migrates_legacy_github_feed_to_gitee() {
        for legacy in LEGACY_ARAM_FEED_URLS {
            let mut config = AppConfig::default();
            config.functions.aram_data_feed_url = legacy.into();
            let normalized = normalize(config);
            assert_eq!(
                normalized.functions.aram_data_feed_url,
                DEFAULT_ARAM_FEED_URL
            );
        }

        let mut custom = AppConfig::default();
        custom.functions.aram_data_feed_url = "https://example.com/manifest.json".into();
        assert_eq!(
            normalize(custom).functions.aram_data_feed_url,
            "https://example.com/manifest.json"
        );
    }
}
