use serde_json::{json, Value};
use std::time::Duration;

use crate::catalog::Catalog;
use crate::error::CoreError;

const LIVE_URL: &str = "https://127.0.0.1:2999/liveclientdata/allgamedata";

#[derive(Clone)]
pub struct LiveClient {
    client: reqwest::Client,
}

impl LiveClient {
    pub fn new() -> Result<Self, CoreError> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .connect_timeout(Duration::from_millis(800))
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(CoreError::io)?;
        Ok(Self { client })
    }

    /// Returns `None` when no game is running or the live API is unavailable.
    pub async fn snapshot(&self, catalog: &Catalog) -> Result<Option<Value>, CoreError> {
        let response = match self.client.get(LIVE_URL).send().await {
            Ok(response) => response,
            Err(_) => return Ok(None),
        };
        if !response.status().is_success() {
            return Ok(None);
        }
        let data: Value = match response.json().await {
            Ok(data) => data,
            Err(_) => return Ok(None),
        };
        Ok(normalize(&data, catalog))
    }
}

fn normalize(data: &Value, catalog: &Catalog) -> Option<Value> {
    let players = data.get("allPlayers").and_then(Value::as_array)?;
    let active = data
        .get("activePlayer")
        .and_then(|player| player.get("summonerName"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let mut normalized = Vec::with_capacity(players.len());
    for player in players {
        let raw_champion = player
            .get("rawChampionName")
            .and_then(Value::as_str)
            .unwrap_or("");
        let alias = raw_champion.split('_').next_back().unwrap_or("");
        let champion_id = catalog.champion_id_by_alias(alias).unwrap_or(0);
        let scores = player.get("scores").cloned().unwrap_or(Value::Null);
        let items = player
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("itemID").and_then(Value::as_u64))
            .collect::<Vec<_>>();
        normalized.push(json!({
            "summonerName": player.get("summonerName").and_then(Value::as_str).unwrap_or(""),
            "championName": player.get("championName").and_then(Value::as_str).unwrap_or(""),
            "championId": champion_id,
            "level": player.get("level").and_then(Value::as_u64).unwrap_or(0),
            "team": player.get("team").and_then(Value::as_str).unwrap_or(""),
            "isBot": player.get("isBot").and_then(Value::as_bool).unwrap_or(false),
            "kills": scores.get("kills").and_then(Value::as_u64).unwrap_or(0),
            "deaths": scores.get("deaths").and_then(Value::as_u64).unwrap_or(0),
            "assists": scores.get("assists").and_then(Value::as_u64).unwrap_or(0),
            "cs": scores.get("creepScore").and_then(Value::as_u64).unwrap_or(0),
            "items": items,
            "isActive": player.get("summonerName").and_then(Value::as_str) == Some(active),
        }));
    }

    let game_data = data.get("gameData").cloned().unwrap_or(Value::Null);
    Some(json!({
        "gameTime": game_data.get("gameTime").and_then(Value::as_f64).unwrap_or(0.0),
        "gameMode": game_data.get("gameMode").and_then(Value::as_str).unwrap_or(""),
        "mapName": game_data.get("mapName").and_then(Value::as_str).unwrap_or(""),
        "gameVersion": game_data.get("gameVersion").and_then(Value::as_str).unwrap_or(""),
        "activeSummoner": active,
        "players": normalized,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_live_players() {
        let data = json!({
            "activePlayer": { "summonerName": "Me#1234" },
            "gameData": {
                "gameTime": 720.5,
                "gameMode": "CLASSIC",
                "mapName": "召唤师峡谷",
                "gameVersion": "16.23.1"
            },
            "allPlayers": [{
                "summonerName": "Me#1234",
                "championName": "阿狸",
                "rawChampionName": "game_character_displayname_Ahri",
                "level": 11,
                "team": "ORDER",
                "isBot": false,
                "scores": { "kills": 3, "deaths": 1, "assists": 5, "creepScore": 120 },
                "items": [{ "itemID": 1001 }, { "itemID": 3009 }]
            }]
        });
        let snapshot = normalize(&data, &Catalog::empty()).unwrap();
        assert_eq!(snapshot["gameTime"], 720.5);
        assert_eq!(snapshot["activeSummoner"], "Me#1234");
        assert_eq!(snapshot["players"][0]["kills"], 3);
        assert_eq!(snapshot["players"][0]["cs"], 120);
        assert_eq!(snapshot["players"][0]["isActive"], true);
        assert_eq!(snapshot["players"][0]["items"].as_array().unwrap().len(), 2);
    }
}
