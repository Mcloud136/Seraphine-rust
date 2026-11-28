use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use seraphine_domain::opgg::{
    parse_arena_champion_build, parse_other_champion_build, parse_other_tier_list,
    parse_ranked_tier_list, resolve_icons, IconResolver,
};
use seraphine_lcu::cache::{CacheFlight, JsonCache, Start};
use seraphine_lcu::ExternalClient;
use seraphine_resource_scheduler::ResourceScheduler;

use crate::catalog::Catalog;
use crate::error::CoreError;

const OPGG_HOST: &str = "lol-api-champion.op.gg";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(60);

pub const REGIONS: [&str; 2] = ["kr", "global"];
pub const MODES: [&str; 6] = ["ranked", "aram", "arena", "urf", "nexus_blitz", "classic"];
pub const TIERS: [&str; 10] = [
    "all",
    "ibsg",
    "gold_plus",
    "platinum_plus",
    "emerald_plus",
    "diamond_plus",
    "master",
    "master_plus",
    "grandmaster",
    "challenger",
];
pub const POSITIONS: [&str; 6] = ["none", "TOP", "JUNGLE", "MID", "ADC", "SUPPORT"];

#[derive(Clone)]
pub struct OpggService {
    external: ExternalClient,
    cache: Arc<JsonCache>,
    scheduler: ResourceScheduler,
    catalog: Catalog,
}

impl OpggService {
    pub fn new(proxy: Option<&str>, catalog: Catalog) -> Result<Self, CoreError> {
        let external =
            ExternalClient::new(proxy).map_err(|error| CoreError::Lcu(error.to_string()))?;
        Ok(Self {
            external,
            cache: Arc::new(JsonCache::default()),
            scheduler: ResourceScheduler::new(),
            catalog,
        })
    }

    pub async fn tier_list(
        &self,
        region: &str,
        mode: &str,
        tier: &str,
    ) -> Result<Value, CoreError> {
        validate(region, mode, tier, "none")?;
        let url = format!("https://{OPGG_HOST}/api/{region}/champions/{mode}?tier={tier}");
        let raw = self.fetch(&url).await?;
        let mut data = if mode == "ranked" {
            parse_ranked_tier_list(&raw)
        } else {
            parse_other_tier_list(&raw)
        };
        resolve_names(&mut data, &self.catalog);
        Ok(json!({
            "data": data,
            "version": raw.get("meta").and_then(|meta| meta.get("version")).cloned().unwrap_or(Value::Null),
            "catalogVersion": self.catalog_version(),
        }))
    }

    pub async fn positions(
        &self,
        region: &str,
        mode: &str,
        tier: &str,
        champion_id: u64,
    ) -> Result<Value, CoreError> {
        validate(region, mode, tier, "none")?;
        let url = format!("https://{OPGG_HOST}/api/{region}/champions/{mode}?tier={tier}");
        let raw = self.fetch(&url).await?;
        Ok(json!(extract_positions(&raw, champion_id)))
    }

    pub async fn champion_build(
        &self,
        region: &str,
        mode: &str,
        tier: &str,
        champion_id: u64,
        position: &str,
    ) -> Result<Value, CoreError> {
        validate(region, mode, tier, position)?;
        let tier_url = format!("https://{OPGG_HOST}/api/{region}/champions/{mode}?tier={tier}");
        let tier_raw = self.fetch(&tier_url).await?;
        let aliases = extract_positions(&tier_raw, champion_id);
        let mut position = position.to_string();
        if !aliases.contains(&position)
            && !aliases.is_empty()
            && matches!(mode, "ranked" | "classic")
        {
            position = aliases[0].clone();
        }
        let url = if mode == "arena" {
            format!("https://{OPGG_HOST}/api/{region}/champions/{mode}/{champion_id}?tier={tier}")
        } else {
            format!(
                "https://{OPGG_HOST}/api/{region}/champions/{mode}/{champion_id}/{position}?tier={tier}"
            )
        };
        let raw = self.fetch(&url).await?;
        let mut data = if mode == "arena" {
            parse_arena_champion_build(&raw)
        } else {
            parse_other_champion_build(&raw, &position)
        };
        let mut resolver = SpecResolver {
            catalog: &self.catalog,
        };
        resolve_icons(&mut data, &mut resolver).await;
        Ok(json!({
            "data": data,
            "version": raw.get("meta").and_then(|meta| meta.get("version")).cloned().unwrap_or(Value::Null),
            "mode": mode,
            "position": position,
        }))
    }

    async fn fetch(&self, url: &str) -> Result<Value, CoreError> {
        loop {
            match self.cache.start(url) {
                Start::Cached(value) => return Ok(value),
                Start::Wait(mut receiver) => {
                    let _ = receiver.changed().await;
                }
                Start::Leader => break,
            }
        }
        let flight = CacheFlight::new(Arc::clone(&self.cache), url.to_string());
        let _permit = self
            .scheduler
            .acquire(seraphine_resource_scheduler::Priority::P2, MAX_BYTES)
            .await
            .map_err(|error| CoreError::Deferred(format!("{error:?}")))?;
        let value = self
            .external
            .get_legacy_json(url, OPGG_HOST, &json!({}), MAX_BYTES)
            .await
            .map_err(|error| CoreError::Lcu(error.to_string()))?;
        let size = serde_json::to_vec(&value)
            .map(|bytes| bytes.len())
            .unwrap_or(MAX_BYTES);
        self.cache.complete(url, value.clone(), size, CACHE_TTL);
        flight.disarm();
        Ok(value)
    }

    fn catalog_version(&self) -> Value {
        self.catalog
            .snapshot()
            .get("gameVersion")
            .cloned()
            .unwrap_or(Value::Null)
    }
}

fn resolve_names(value: &mut Value, catalog: &Catalog) {
    match value {
        Value::String(text) => {
            if let Some(id) = text
                .strip_prefix("@champion_name:")
                .and_then(|id| id.parse::<u64>().ok())
            {
                if let Some(name) = catalog.champion_name(id) {
                    *text = name.to_string();
                }
            } else if let Some(id) = text
                .strip_prefix("@augment_name:")
                .and_then(|id| id.parse::<u64>().ok())
            {
                if let Some(name) = catalog.augment_name(id) {
                    *text = name.to_string();
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_names(item, catalog);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                resolve_names(item, catalog);
            }
        }
        _ => {}
    }
}

fn extract_positions(raw: &Value, champion_id: u64) -> Vec<String> {
    raw.get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|item| item.get("id").and_then(Value::as_u64) == Some(champion_id))
        .map(|item| {
            item.get("positions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|position| position.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

struct SpecResolver<'a> {
    catalog: &'a Catalog,
}

#[async_trait]
impl IconResolver for SpecResolver<'_> {
    async fn resolve(&mut self, spec: String) -> String {
        resolve_spec(&spec, self.catalog)
    }
}

fn resolve_spec(spec: &str, catalog: &Catalog) -> String {
    if let Some(id) = spec
        .strip_prefix("@champion:")
        .and_then(|id| id.parse::<u64>().ok())
    {
        return format!("lcu:/lol-game-data/assets/v1/champion-icons/{id}.png");
    }
    for kind in ["item", "spell", "rune", "augment"] {
        if let Some(id) = spec
            .strip_prefix(&format!("@{kind}:"))
            .and_then(|id| id.parse::<u64>().ok())
        {
            if let Some(path) = catalog.asset_path(kind, id) {
                return format!("lcu:{path}");
            }
            return spec.to_string();
        }
    }
    if let Some(id) = spec
        .strip_prefix("@champion_name:")
        .and_then(|id| id.parse::<u64>().ok())
    {
        if let Some(name) = catalog.champion_name(id) {
            return name.to_string();
        }
    }
    if let Some(id) = spec
        .strip_prefix("@augment_name:")
        .and_then(|id| id.parse::<u64>().ok())
    {
        if let Some(name) = catalog.augment_name(id) {
            return name.to_string();
        }
    }
    if spec.starts_with("@classic_rune:") || spec.starts_with("@legacy_mastery:") {
        if let Some((url, _)) = spec.rsplit_once('|') {
            return url
                .trim_start_matches("@classic_rune:")
                .trim_start_matches("@legacy_mastery:")
                .to_string();
        }
    }
    spec.to_string()
}

fn validate(region: &str, mode: &str, tier: &str, position: &str) -> Result<(), CoreError> {
    if !REGIONS.contains(&region)
        || !MODES.contains(&mode)
        || !TIERS.contains(&tier)
        || !POSITIONS.contains(&position)
    {
        return Err(CoreError::InvalidParams("invalid OP.GG parameters".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parameters_outside_the_allowlist() {
        assert!(validate("kr", "ranked", "emerald_plus", "ADC").is_ok());
        assert!(validate("na", "ranked", "all", "none").is_err());
        assert!(validate("kr", "ranked", "all", "../../etc").is_err());
        assert!(validate("kr", "unknown", "all", "none").is_err());
        assert!(validate("kr", "ranked", "all", "TOP;rm").is_err());
    }

    #[test]
    fn resolves_icon_specs_without_network() {
        let catalog = Catalog::empty();
        assert_eq!(
            resolve_spec("@champion:202", &catalog),
            "lcu:/lol-game-data/assets/v1/champion-icons/202.png"
        );
        assert_eq!(resolve_spec("@item:1001", &catalog), "@item:1001");
        assert_eq!(
            resolve_spec("@classic_rune:https://example.com/x.png|8000", &catalog),
            "https://example.com/x.png"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_champion_build_returns_sections() {
        let catalog = Catalog::load(&crate::paths::AppPaths::discover());
        let service = OpggService::new(None, catalog).unwrap();
        let value = service
            .champion_build("kr", "ranked", "emerald_plus", 202, "ADC")
            .await
            .unwrap();
        let data = &value["data"];
        assert!(data["summary"]["name"].as_str().is_some());
        assert!(data["items"]["coreItems"].as_array().is_some());
        assert!(data["perks"].as_array().is_some());
        println!("summary={}", data["summary"]);
        println!("items={}", data["items"]);
    }
}
