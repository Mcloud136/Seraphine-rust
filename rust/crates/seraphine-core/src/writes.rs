use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use seraphine_automation::write_guard::{confirm_write, validate_write};
use seraphine_lcu::LcuClient;

use crate::error::CoreError;
use crate::paths::AppPaths;
use crate::session::SessionManager;

const CHAMP_SELECT_SESSION: &str = "/lol-champ-select/v1/session";
const GUARD_RETENTION_SECONDS: u64 = 30 * 24 * 3600;
const MAX_GUARDS: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct GuardRecord {
    request_hash: String,
    completed: bool,
    updated_at: u64,
}

pub struct WriteService {
    session: Arc<SessionManager>,
    guards: Mutex<HashMap<String, GuardRecord>>,
    path: PathBuf,
}

impl WriteService {
    pub fn new(session: Arc<SessionManager>, paths: &AppPaths) -> Self {
        let path = paths.state.join("write-guards.json");
        let mut guards = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<HashMap<String, GuardRecord>>(&bytes).ok())
            .unwrap_or_default();
        let now = unix_seconds();
        guards.retain(|_, record| {
            !record.completed || now.saturating_sub(record.updated_at) < GUARD_RETENTION_SECONDS
        });
        Self {
            session,
            guards: Mutex::new(guards),
            path,
        }
    }

    pub async fn execute(
        &self,
        operation_id: &str,
        method: &str,
        path: &str,
        data: Option<Value>,
    ) -> Result<Value, CoreError> {
        if operation_id.is_empty()
            || operation_id.len() > 128
            || !operation_id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.:".contains(character))
        {
            return Err(CoreError::InvalidParams("invalid operation id".into()));
        }
        let method = method.to_ascii_uppercase();
        if !allowed(&method, path) {
            return Err(CoreError::InvalidParams(format!(
                "write path is not allowed: {method} {path}"
            )));
        }
        let status = self.session.status().await;
        if !status.connected {
            return Err(CoreError::NotConnected);
        }
        let key = format!("{}:{operation_id}", status.pid);
        let request_hash = request_hash(&method, path, data.as_ref());
        let now = unix_seconds();
        {
            let mut guards = self
                .guards
                .lock()
                .map_err(|_| CoreError::InvalidState("write guard lock poisoned".into()))?;
            if let Some(record) = guards.get(&key) {
                if record.request_hash != request_hash {
                    return Err(CoreError::OperationConflict(
                        "operation id was used for a different request".into(),
                    ));
                }
                if record.completed {
                    return Ok(json!({
                        "replayed": true,
                        "state": "completed",
                        "data": Value::Null,
                    }));
                }
                return Err(CoreError::OutcomeUnknown(
                    "previous write outcome is unknown; inspect the client state".into(),
                ));
            }
            if guards.len() >= MAX_GUARDS {
                if let Some(oldest) = guards
                    .iter()
                    .filter(|(_, record)| record.completed)
                    .min_by_key(|(_, record)| record.updated_at)
                    .map(|(key, _)| key.clone())
                {
                    guards.remove(&oldest);
                } else {
                    return Err(CoreError::Deferred("write guard capacity reached".into()));
                }
            }
        }

        let lcu = self.session.lcu().await.ok_or(CoreError::NotConnected)?;
        let guarded = path.starts_with(CHAMP_SELECT_SESSION);
        if guarded {
            let session = self.session.lcu_get_fresh(CHAMP_SELECT_SESSION).await?;
            validate_write(&session, path, data.as_ref())
                .map_err(|error| CoreError::InvalidState(error.as_str().to_string()))?;
        }

        let result = execute_http(&lcu, &method, path, data.clone()).await;
        if guarded {
            let mut confirmation = confirm_once(&self.session, path, data.as_ref()).await;
            // The client sometimes applies champ-select writes a beat after the
            // HTTP response, so re-read before declaring the outcome unknown.
            for _ in 0..2 {
                if confirmation.is_ok() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                confirmation = confirm_once(&self.session, path, data.as_ref()).await;
            }
            match confirmation {
                Ok(()) => {
                    let confirmed_after_error = result.is_err();
                    self.store(&key, &request_hash, true, now);
                    let mut response = match result {
                        Ok(value) => value,
                        Err(error) => json!({ "writeError": error.to_string() }),
                    };
                    response["replayed"] = Value::Bool(false);
                    response["state"] = Value::String("completed".into());
                    if confirmed_after_error {
                        response["confirmedAfterError"] = Value::Bool(true);
                    }
                    return Ok(response);
                }
                Err(error) => {
                    self.store(&key, &request_hash, false, now);
                    let detail = match &result {
                        Ok(_) => String::new(),
                        Err(error) => format!("; request error: {error}"),
                    };
                    return Err(CoreError::OutcomeUnknown(format!(
                        "write confirmation failed: {error}{detail}"
                    )));
                }
            }
        }

        match result {
            Ok(value) => {
                self.store(&key, &request_hash, true, now);
                Ok(json!({
                    "replayed": false,
                    "state": "completed",
                    "data": value,
                }))
            }
            Err(error) => Err(error),
        }
    }

    fn store(&self, key: &str, hash: &str, completed: bool, now: u64) {
        let Ok(mut guards) = self.guards.lock() else {
            return;
        };
        guards.insert(
            key.to_string(),
            GuardRecord {
                request_hash: hash.to_string(),
                completed,
                updated_at: now,
            },
        );
        let payload = serde_json::to_vec_pretty(&*guards).unwrap_or_default();
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let temporary = self.path.with_extension("json.tmp");
        if std::fs::write(&temporary, payload).is_ok() {
            let _ = seraphine_platform::atomic_replace_file(&temporary, &self.path);
        }
    }
}

fn request_hash(method: &str, path: &str, data: Option<&Value>) -> String {
    let body = data.map(Value::to_string).unwrap_or_default();
    format!(
        "{:x}",
        Sha256::digest(format!("{method}\n{path}\n{body}").as_bytes())
    )
}

async fn confirm_once(
    session: &SessionManager,
    path: &str,
    data: Option<&Value>,
) -> Result<(), String> {
    match session.lcu_get_fresh(CHAMP_SELECT_SESSION).await {
        Ok(session) => {
            confirm_write(&session, path, data).map_err(|error| error.as_str().to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

async fn execute_http(
    lcu: &LcuClient,
    method: &str,
    path: &str,
    data: Option<Value>,
) -> Result<Value, CoreError> {
    let result = match method {
        "POST" => lcu.post_json(path, data).await,
        "PUT" => lcu.put_json(path, data).await,
        "PATCH" => lcu.patch_json(path, data).await,
        "DELETE" => lcu.delete(path).await,
        _ => {
            return Err(CoreError::InvalidParams(format!(
                "unsupported write method: {method}"
            )))
        }
    };
    result.map_err(|error| CoreError::Lcu(error.to_string()))
}

fn allowed(method: &str, path: &str) -> bool {
    if !path.starts_with('/') || path.contains("://") || path.contains("..") {
        return false;
    }
    match (method, path) {
        ("POST", "/riotclient/kill-and-restart-ux")
        | ("POST", "/lol-matchmaking/v1/ready-check/accept")
        | ("POST", "/lol-gameflow/v1/reconnect")
        | ("POST", "/lol-spectator/v1/spectate/launch")
        | ("POST", "/lol-summoner/v1/current-summoner/summoner-profile")
        | ("POST", "/lol-challenges/v1/update-player-preferences/")
        | ("PUT", "/lol-chat/v1/me")
        | ("PUT", "/lol-summoner/v1/current-summoner/icon")
        | ("PUT", "/lol-regalia/v2/current-summoner/regalia")
        | ("PATCH", "/lol-champ-select/v1/session/my-selection") => true,
        ("PATCH", value) => action_id(value).is_some(),
        ("POST", value) => {
            let segments = value.split('/').collect::<Vec<_>>();
            (segments.len() == 7
                && segments[1..4] == ["lol-champ-select", "v1", "session"]
                && matches!(segments[4], "trades" | "swaps")
                && segments[5].parse::<u64>().is_ok()
                && segments[6] == "accept")
                || (segments.len() == 6
                    && segments[1..4] == ["lol-champ-select", "v1", "session"]
                    && matches!(segments[4], "ongoing-trade" | "ongoing-swap")
                    && segments[5].parse::<u64>().is_ok())
        }
        _ => false,
    }
}

fn action_id(path: &str) -> Option<u64> {
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.len() == 6
        && segments[1..4] == ["lol-champ-select", "v1", "session"]
        && segments[4] == "actions"
    {
        segments[5].parse().ok()
    } else {
        None
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_covers_only_expected_writes() {
        assert!(allowed("POST", "/lol-matchmaking/v1/ready-check/accept"));
        assert!(allowed("PATCH", "/lol-champ-select/v1/session/actions/42"));
        assert!(allowed(
            "PATCH",
            "/lol-champ-select/v1/session/my-selection"
        ));
        assert!(allowed(
            "POST",
            "/lol-champ-select/v1/session/trades/3/accept"
        ));
        assert!(allowed(
            "POST",
            "/lol-champ-select/v1/session/ongoing-swap/3"
        ));
        assert!(!allowed(
            "PATCH",
            "/lol-champ-select/v1/session/actions/abc"
        ));
        assert!(!allowed(
            "POST",
            "/lol-summoner/v1/current-summoner/summoner-profile/extra"
        ));
        assert!(!allowed("DELETE", "/lol-champ-select/v1/session"));
        assert!(!allowed("GET", "/lol-chat/v1/me"));
        assert!(!allowed("PUT", "/lol-chat/v1/me/../other"));
    }

    #[test]
    fn request_hash_is_stable_and_body_sensitive() {
        let first = request_hash("PATCH", "/x", Some(&json!({"a": 1})));
        let second = request_hash("PATCH", "/x", Some(&json!({"a": 1})));
        let third = request_hash("PATCH", "/x", Some(&json!({"a": 2})));
        assert_eq!(first, second);
        assert_ne!(first, third);
    }
}
