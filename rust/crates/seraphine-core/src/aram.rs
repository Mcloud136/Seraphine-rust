use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::CoreError;
use crate::paths::AppPaths;

const BUNDLED_DATA: &str = include_str!("../../../../app/resource/data/AramBuff.json");
const MIN_CHAMPIONS: usize = 100;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_DATA_BYTES: usize = 2 * 1024 * 1024;
const MIN_ATTEMPT_INTERVAL: u64 = 600;
const ALLOWED_HOSTS: [&str; 5] = [
    "gitee.com",
    "github.com",
    "raw.githubusercontent.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// ARAM balance data, preferred from the app cache and backed by the bundle.
#[derive(Clone)]
pub struct AramData {
    data: Value,
    source: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AramUpdate {
    pub updated: bool,
    pub version: String,
    pub warning: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateState {
    last_attempt: u64,
    version: String,
}

impl AramData {
    pub fn load(paths: &AppPaths) -> Self {
        for (path, source) in [
            (paths.state.join("AramBuff.json"), "state"),
            (paths.legacy_data.join("AramBuff.json"), "cache"),
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(data) = serde_json::from_slice::<Value>(&bytes) {
                    if validate(&data).is_ok() {
                        return Self { data, source };
                    }
                }
            }
        }
        let data = serde_json::from_str::<Value>(BUNDLED_DATA).unwrap_or(Value::Null);
        Self {
            data,
            source: "bundled",
        }
    }

    pub fn source(&self) -> &'static str {
        self.source
    }

    pub fn version(&self) -> &str {
        self.data
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub fn champion(&self, champion_id: u64) -> Option<Value> {
        let entry = self.data.get("champions")?.get(champion_id.to_string())?;
        Some(serde_json::json!({
            "championId": champion_id,
            "name": entry.get("catname").and_then(Value::as_str).unwrap_or(""),
            "description": entry.get("description").and_then(Value::as_str).unwrap_or(""),
            "damageDealt": percent(entry.get("zcsh")),
            "damageTaken": percent(entry.get("sdsh")),
            "healing": percent(entry.get("zlxl")),
            "shielding": percent(entry.get("hdxn")),
            "abilityHaste": number(entry.get("jnjs")),
            "tenacity": number(entry.get("renxing")),
        }))
    }
}

fn percent(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<i64>().ok())
}

fn number(value: Option<&Value>) -> Option<i64> {
    percent(value)
}

/// Downloads and verifies a feed update, then replaces the state cache.
pub async fn update(
    paths: &AppPaths,
    feed_url: &str,
    current_version: &str,
    force: bool,
    proxy: Option<&str>,
) -> Result<AramUpdate, CoreError> {
    let feed_url = feed_url.trim();
    if feed_url.is_empty() {
        return Err(CoreError::InvalidParams(
            "ARAM feed URL is not configured".into(),
        ));
    }
    validate_url(feed_url)?;

    let state_path = paths.state.join("aram-update-state.json");
    let mut state = read_state(&state_path);
    let now = unix_seconds();
    if !force
        && state.last_attempt > 0
        && now.saturating_sub(state.last_attempt) < MIN_ATTEMPT_INTERVAL
    {
        return Ok(AramUpdate {
            updated: false,
            version: current_version.to_string(),
            warning: Some("刚刚检查过，请稍后再试".into()),
        });
    }
    state.last_attempt = now;
    let _ = write_state(&state_path, &state);

    let client = build_client(proxy)?;
    let manifest_bytes = fetch(&client, feed_url, MAX_MANIFEST_BYTES).await?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| CoreError::InvalidParams(format!("invalid manifest: {error}")))?;
    validate_manifest(&manifest)?;
    let version = manifest["version"]
        .as_str()
        .ok_or_else(|| CoreError::InvalidParams("manifest has no version".into()))?;

    if !force && version == current_version {
        state.version = version.to_string();
        let _ = write_state(&state_path, &state);
        return Ok(AramUpdate {
            updated: false,
            version: version.to_string(),
            warning: None,
        });
    }

    let file = manifest["file"]
        .as_str()
        .ok_or_else(|| CoreError::InvalidParams("manifest has no file".into()))?;
    let base = reqwest::Url::parse(feed_url)
        .map_err(|error| CoreError::InvalidParams(error.to_string()))?;
    let file_url = base
        .join(file)
        .map_err(|error| CoreError::InvalidParams(error.to_string()))?;
    validate_url(file_url.as_str())?;

    let data_bytes = fetch(&client, file_url.as_str(), MAX_DATA_BYTES).await?;
    let expected_size = manifest["size"].as_u64().unwrap_or(0);
    if data_bytes.len() as u64 != expected_size {
        return Err(CoreError::InvalidParams(format!(
            "size mismatch: expected {expected_size}, got {}",
            data_bytes.len()
        )));
    }
    let expected_hash = manifest["sha256"]
        .as_str()
        .ok_or_else(|| CoreError::InvalidParams("manifest has no sha256".into()))?;
    let actual_hash = format!("{:x}", Sha256::digest(&data_bytes));
    if !actual_hash.eq_ignore_ascii_case(expected_hash) {
        return Err(CoreError::InvalidParams("sha256 mismatch".into()));
    }
    let data: Value = serde_json::from_slice(&data_bytes)
        .map_err(|error| CoreError::InvalidParams(format!("invalid ARAM data: {error}")))?;
    validate(&data).map_err(|error| CoreError::InvalidParams(error.into()))?;
    if data.get("version").and_then(Value::as_str) != Some(version) {
        return Err(CoreError::InvalidParams(
            "manifest and data versions differ".into(),
        ));
    }

    std::fs::create_dir_all(&paths.state).map_err(CoreError::io)?;
    write_atomic(&paths.state.join("AramBuff.json"), &data_bytes)?;
    write_atomic(&paths.state.join("aram-manifest.json"), &manifest_bytes)?;
    state.version = version.to_string();
    let _ = write_state(&state_path, &state);
    Ok(AramUpdate {
        updated: true,
        version: version.to_string(),
        warning: None,
    })
}

fn validate_url(url: &str) -> Result<(), CoreError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| CoreError::InvalidParams(format!("invalid URL: {error}")))?;
    if parsed.scheme() != "https" {
        return Err(CoreError::InvalidParams("only HTTPS is allowed".into()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| CoreError::InvalidParams("URL has no host".into()))?;
    if !ALLOWED_HOSTS
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
    {
        return Err(CoreError::InvalidParams(format!(
            "host is not allowed: {host}"
        )));
    }
    Ok(())
}

fn build_client(proxy: Option<&str>) -> Result<reqwest::Client, CoreError> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5));
    if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
        let proxy = if proxy.contains("://") {
            proxy.to_string()
        } else {
            format!("http://{proxy}")
        };
        builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(CoreError::io)?);
    }
    builder.build().map_err(CoreError::io)
}

async fn fetch(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, CoreError> {
    use futures_util::StreamExt;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| CoreError::Io(error.to_string()))?;
    if !response.status().is_success() {
        return Err(CoreError::Io(format!(
            "feed returned HTTP {}",
            response.status()
        )));
    }
    validate_url(response.url().as_str())?;
    if let Some(length) = response.content_length() {
        if length > max_bytes as u64 {
            return Err(CoreError::InvalidParams("feed exceeds byte budget".into()));
        }
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| CoreError::Io(error.to_string()))?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(CoreError::InvalidParams("feed exceeds byte budget".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn validate_manifest(manifest: &Value) -> Result<(), CoreError> {
    if manifest.get("schema").and_then(Value::as_u64) != Some(1) {
        return Err(CoreError::InvalidParams(
            "unsupported manifest schema".into(),
        ));
    }
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !valid_version(version) {
        return Err(CoreError::InvalidParams("invalid manifest version".into()));
    }
    let file = manifest.get("file").and_then(Value::as_str).unwrap_or("");
    if file.is_empty()
        || file.contains('/')
        || file.contains('\\')
        || !file.to_ascii_lowercase().ends_with(".json")
    {
        return Err(CoreError::InvalidParams(
            "invalid manifest file name".into(),
        ));
    }
    let size = manifest.get("size").and_then(Value::as_u64).unwrap_or(0);
    if size == 0 || size > MAX_DATA_BYTES as u64 {
        return Err(CoreError::InvalidParams("invalid manifest size".into()));
    }
    let digest = manifest.get("sha256").and_then(Value::as_str).unwrap_or("");
    if digest.len() != 64
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(CoreError::InvalidParams("invalid manifest sha256".into()));
    }
    let champions = manifest
        .get("champions")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if champions < MIN_CHAMPIONS as u64 {
        return Err(CoreError::InvalidParams(
            "invalid manifest champion count".into(),
        ));
    }
    Ok(())
}

fn valid_version(version: &str) -> bool {
    let parts = version.split('.').collect::<Vec<_>>();
    (parts.len() == 2 || parts.len() == 3)
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

fn read_state(path: &std::path::Path) -> UpdateState {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_state(path: &std::path::Path, state: &UpdateState) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(CoreError::io)?;
    }
    let payload = serde_json::to_vec_pretty(state).map_err(CoreError::json)?;
    write_atomic(path, &payload)
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(CoreError::io)?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes).map_err(CoreError::io)?;
    seraphine_platform::atomic_replace_file(&temporary, path).map_err(CoreError::io)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn validate(data: &Value) -> Result<(), &'static str> {
    if !data.is_object() {
        return Err("ARAM data is not an object");
    }
    let version = data.get("version").and_then(Value::as_str).unwrap_or("");
    if !valid_version(version) {
        return Err("ARAM data has an invalid version");
    }
    let champions = data
        .get("champions")
        .and_then(Value::as_object)
        .ok_or("ARAM data has no champions")?;
    if champions.len() < MIN_CHAMPIONS {
        return Err("ARAM data has too few champions");
    }
    for (id, entry) in champions {
        if id.parse::<u64>().is_err() || !entry.is_object() {
            return Err("ARAM data has an invalid champion entry");
        }
        let name = entry.get("catname").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() {
            return Err("ARAM champion has no name");
        }
        for field in ["zcsh", "sdsh", "zlxl", "hdxn", "jnjs", "renxing"] {
            if entry
                .get(field)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<i64>().ok())
                .is_none()
            {
                return Err("ARAM champion has an invalid numeric field");
            }
        }
        if entry.get("description").and_then(Value::as_str).is_none() {
            return Err("ARAM champion has no description");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_data_parses_and_exposes_buffs() {
        let data = AramData::load(&AppPaths::discover());
        assert!(data.version().starts_with("16."));
        let champion = data.champion(1).expect("champion 1");
        assert!(champion["description"]
            .as_str()
            .is_some_and(|text| !text.is_empty()));
        assert!(champion["damageDealt"].as_i64().is_some());
    }

    #[test]
    fn unknown_champion_returns_none() {
        let data = AramData::load(&AppPaths::discover());
        assert!(data.champion(999_999).is_none());
    }

    #[test]
    fn manifest_validation_rejects_unsafe_fields() {
        let digest = "a".repeat(64);
        let valid = serde_json::json!({
            "schema": 1,
            "version": "16.23.1",
            "file": "AramBuff.json",
            "size": 1024,
            "sha256": digest,
            "champions": 132
        });
        assert!(validate_manifest(&valid).is_ok());

        let mut bad = valid.clone();
        bad["file"] = serde_json::json!("../evil.json");
        assert!(validate_manifest(&bad).is_err());

        let mut bad = valid.clone();
        bad["sha256"] = serde_json::json!("not-a-hash");
        assert!(validate_manifest(&bad).is_err());

        let mut bad = valid.clone();
        bad["champions"] = serde_json::json!(3);
        assert!(validate_manifest(&bad).is_err());

        assert!(valid_version("16.23"));
        assert!(valid_version("16.23.1"));
        assert!(!valid_version("16"));
        assert!(!valid_version("v16.23"));
    }

    #[test]
    fn rejects_urls_outside_the_allowlist() {
        assert!(validate_url("https://gitee.com/owner/repo/raw/master/aram-manifest.json").is_ok());
        assert!(validate_url(
            "https://raw.githubusercontent.com/owner/repo/main/aram-manifest.json"
        )
        .is_ok());
        assert!(validate_url("http://gitee.com/x").is_err());
        assert!(validate_url("https://example.com/x").is_err());
    }
}
