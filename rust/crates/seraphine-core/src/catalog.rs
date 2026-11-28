use base64::Engine;
use serde_json::{json, Value};

use crate::error::CoreError;
use crate::paths::AppPaths;
use crate::session::SessionManager;

const MAX_ASSET_BYTES: usize = 4 * 1024 * 1024;
const ASSET_PREFIX: &str = "/lol-game-data/assets/";

/// Read-only view of the catalog snapshot produced by the sidecar.
#[derive(Clone, Debug)]
pub struct Catalog {
    snapshot: Value,
}

impl Catalog {
    pub fn empty() -> Self {
        Self {
            snapshot: Value::Object(Default::default()),
        }
    }

    pub fn load(paths: &AppPaths) -> Self {
        for path in [
            paths.state.join("catalog.json"),
            paths.catalog.join("catalog.json"),
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                if bytes.len() <= 64 * 1024 * 1024 {
                    if let Ok(snapshot) = serde_json::from_slice::<Value>(&bytes) {
                        return Self { snapshot };
                    }
                }
            }
        }
        Self::empty()
    }

    /// Rebuilds the catalog from the connected client and stores it in the
    /// app state directory so item/rune icons follow the current game version.
    pub async fn refresh(session: &SessionManager, paths: &AppPaths) -> Result<Self, CoreError> {
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
        let mut builder = seraphine_domain::catalog::CatalogBuilder::default();
        for (index, endpoint) in endpoints.into_iter().enumerate() {
            let value = session.lcu_get(endpoint).await?;
            match index {
                0 => builder.add_items(value),
                1 => builder.add_spells(value),
                2 => builder.add_runes(value),
                3 => builder.add_queues(value),
                4 => builder.add_champions(value),
                5 => builder.add_skins(value),
                6 => builder.add_perks(value),
                7 => builder.add_augments(value),
                _ => {}
            }
        }
        let snapshot = builder.build();
        let payload = serde_json::to_vec(&snapshot).map_err(CoreError::json)?;
        std::fs::create_dir_all(&paths.state).map_err(CoreError::io)?;
        let destination = paths.state.join("catalog.json");
        let temporary = destination.with_extension("json.tmp");
        std::fs::write(&temporary, &payload).map_err(CoreError::io)?;
        seraphine_platform::atomic_replace_file(&temporary, &destination).map_err(CoreError::io)?;
        Ok(Self { snapshot })
    }

    pub fn champion_name(&self, id: u64) -> Option<&str> {
        self.snapshot
            .get("champs")
            .and_then(|champs| champs.get(id.to_string()))
            .and_then(Value::as_str)
    }

    pub fn champion_id_by_alias(&self, alias: &str) -> Option<u64> {
        let key = alias.to_ascii_lowercase();
        self.snapshot
            .get("champAliases")
            .and_then(|aliases| aliases.get(key))
            .and_then(Value::as_u64)
    }

    pub fn augment_name(&self, id: u64) -> Option<&str> {
        self.snapshot
            .get("cherryAugments")
            .and_then(|augments| augments.get(id.to_string()))
            .and_then(|augment| augment.get("nameTRA"))
            .and_then(Value::as_str)
    }

    pub fn asset_path(&self, kind: &str, id: u64) -> Option<&str> {
        match kind {
            "item" => self.entry_path("items", id, "iconPath"),
            "spell" => self.entry_path("summonerSpells", id, "iconPath"),
            "augment" => self.entry_path("cherryAugments", id, "augmentSmallIconPath"),
            "rune" => self.entry_path("runes", id, "icon").or_else(|| {
                self.snapshot
                    .get("perks")
                    .and_then(|perks| perks.get("styles"))
                    .and_then(Value::as_array)
                    .and_then(|styles| {
                        styles
                            .iter()
                            .find(|style| style.get("id").and_then(Value::as_u64) == Some(id))
                    })
                    .and_then(|style| style.get("iconPath"))
                    .and_then(Value::as_str)
            }),
            _ => None,
        }
    }

    fn entry_path(&self, section: &str, id: u64, key: &str) -> Option<&str> {
        self.snapshot
            .get(section)
            .and_then(|entries| entries.get(id.to_string()))
            .and_then(|entry| entry.get(key))
            .and_then(Value::as_str)
    }

    pub fn queue_name(&self, id: u64) -> Option<&str> {
        self.snapshot
            .get("queues")
            .and_then(|queues| queues.get(id.to_string()))
            .and_then(|queue| queue.get("name"))
            .and_then(Value::as_str)
    }

    pub fn snapshot(&self) -> &Value {
        &self.snapshot
    }

    /// Subset the web view needs for names and icons.
    pub fn champion_skins(&self, champion: &str) -> Value {
        self.snapshot
            .get("championSkins")
            .and_then(|skins| skins.get(champion))
            .cloned()
            .unwrap_or_else(|| json!({}))
    }

    pub fn web_snapshot(&self) -> Value {
        let section = |key: &str| self.snapshot.get(key).cloned().unwrap_or(json!({}));
        json!({
            "champions": section("champs"),
            "items": section("items"),
            "spells": web_spells(&self.snapshot),
            "runes": section("runes"),
            "queues": section("queues"),
            "augments": section("cherryAugments"),
            "champAliases": section("champAliases"),
        })
    }
}

/// Summoner spells offered by the picker, matching the legacy whitelist.
/// The client also returns Classic (Jade) and mode-specific variants
/// (Poro King, Arena) that must not appear.
const WEB_SUMMONER_SPELLS: [&str; 12] = [
    "1", "3", "4", "6", "7", "11", "12", "13", "14", "21", "32", "54",
];

fn web_spells(snapshot: &Value) -> Value {
    let Some(spells) = snapshot.get("summonerSpells").and_then(Value::as_object) else {
        return json!({});
    };
    Value::Object(
        spells
            .iter()
            .filter(|(id, _)| WEB_SUMMONER_SPELLS.contains(&id.as_str()))
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect(),
    )
}

/// Fetches an LCU asset and returns it as a data URL for the web view.
pub async fn asset_data_url(session: &SessionManager, path: &str) -> Result<String, CoreError> {
    if !path.starts_with(ASSET_PREFIX) {
        return Err(CoreError::InvalidParams(
            "asset path is outside the game data root".into(),
        ));
    }
    let lcu = session.lcu().await.ok_or(CoreError::NotConnected)?;
    let bytes = lcu
        .get_bytes(path, MAX_ASSET_BYTES)
        .await
        .map_err(|error| CoreError::Lcu(error.to_string()))?;
    let mime = if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".webp") {
        "image/webp"
    } else {
        "image/png"
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_snapshot_excludes_classic_and_mode_spells() {
        let catalog = Catalog {
            snapshot: json!({
                "summonerSpells": {
                    "4": { "name": "闪现" },
                    "32": { "name": "标记" },
                    "54": { "name": "占位" },
                    "74": { "name": "闪现" },
                    "777": { "name": "重生" },
                    "2202": { "name": "闪现" }
                }
            }),
        };
        let spells = catalog.web_snapshot()["spells"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        assert!(spells.contains_key("4"));
        assert!(spells.contains_key("32"));
        assert!(spells.contains_key("54"));
        assert!(!spells.contains_key("74"));
        assert!(!spells.contains_key("777"));
        assert!(!spells.contains_key("2202"));
    }
}
