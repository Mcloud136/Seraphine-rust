use serde_json::{json, Value};

use crate::career;
use crate::catalog::Catalog;
use crate::error::CoreError;
use crate::session::SessionManager;

/// Loads the recent Teamfight Tactics games for a player.
pub async fn recent(
    session: &SessionManager,
    catalog: &Catalog,
    puuid: Option<&str>,
    count: u32,
) -> Result<Value, CoreError> {
    let (puuid, _) = match puuid {
        Some(puuid) => (puuid.to_string(), None),
        None => {
            let summoner = career::current_summoner(session).await?;
            let puuid = summoner
                .get("puuid")
                .and_then(Value::as_str)
                .ok_or_else(|| CoreError::InvalidParams("summoner has no puuid".into()))?
                .to_string();
            (puuid, Some(summoner))
        }
    };
    let count = count.clamp(1, 40);
    let path = format!("/lol-match-history/v1/products/tft/{puuid}/matches?begin=0&count={count}");
    let data = session.lcu_get(&path).await?;
    let games = data
        .get("games")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut parsed = Vec::new();
    let mut wins = 0_u64;
    let mut losses = 0_u64;
    let mut placement_sum = 0_u64;
    for entry in &games {
        if let Some(game) = summarize_game(entry, &puuid, catalog) {
            let placement = game["placement"].as_u64().unwrap_or(0);
            if placement > 0 {
                placement_sum += placement;
                if placement <= 4 {
                    wins += 1;
                } else {
                    losses += 1;
                }
            }
            parsed.push(game);
        }
    }
    let total = wins + losses;
    let average = if total > 0 {
        (placement_sum as f64 / total as f64 * 100.0).round() / 100.0
    } else {
        0.0
    };

    Ok(json!({
        "puuid": puuid,
        "wins": wins,
        "losses": losses,
        "averagePlacement": average,
        "games": parsed,
    }))
}

fn summarize_game(entry: &Value, puuid: &str, catalog: &Catalog) -> Option<Value> {
    let game = entry.get("json").unwrap_or(entry);
    let participants = game.get("participants").and_then(Value::as_array)?;
    let me = participants
        .iter()
        .find(|participant| participant.get("puuid").and_then(Value::as_str) == Some(puuid))?;
    let placement = me.get("placement").and_then(Value::as_u64).unwrap_or(0);
    let mut units = Vec::new();
    for unit in me
        .get("units")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let character = unit
            .get("character_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let alias = character.split('_').next_back().unwrap_or_default();
        let champion_id = catalog.champion_id_by_alias(alias).unwrap_or(0);
        units.push(json!({
            "characterId": character,
            "alias": alias,
            "championId": champion_id,
            "championName": catalog.champion_name(champion_id).unwrap_or(alias),
            "tier": unit.get("tier").and_then(Value::as_u64).unwrap_or(0),
            "rarity": unit.get("rarity").and_then(Value::as_u64).unwrap_or(0),
        }));
    }
    units.sort_by_key(|unit| {
        (
            std::cmp::Reverse(unit.get("rarity").and_then(Value::as_u64).unwrap_or(0)),
            std::cmp::Reverse(unit.get("tier").and_then(Value::as_u64).unwrap_or(0)),
        )
    });

    let mut augments = Vec::new();
    for augment in me
        .get("augments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = augment.as_u64().unwrap_or(0);
        augments.push(json!({
            "id": id,
            "name": catalog.augment_name(id).unwrap_or(""),
        }));
    }

    Some(json!({
        "gameId": game.get("gameId").and_then(Value::as_u64)
            .or_else(|| game.get("game_id").and_then(Value::as_u64))
            .unwrap_or(0),
        "queueId": game.get("queueId").and_then(Value::as_u64)
            .or_else(|| game.get("queue_id").and_then(Value::as_u64))
            .unwrap_or(0),
        "placement": placement,
        "win": placement > 0 && placement <= 4,
        "level": me.get("level").and_then(Value::as_u64).unwrap_or(0),
        "duration": game.get("game_length").and_then(Value::as_f64).unwrap_or(0.0)
            .round() as u64,
        "gameCreation": game.get("game_datetime").and_then(Value::as_u64)
            .or_else(|| game.get("gameCreation").and_then(Value::as_u64))
            .unwrap_or(0),
        "setNumber": game.get("tft_set_number").and_then(Value::as_u64).unwrap_or(0),
        "units": units,
        "augments": augments,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_tft_units_and_augments() {
        let entry = json!({
            "json": {
                "gameId": 9,
                "queueId": 1100,
                "game_length": 1850.4,
                "game_datetime": 1_700_000_000_000u64,
                "tft_set_number": 17,
                "participants": [{
                    "puuid": "p1",
                    "placement": 2,
                    "level": 9,
                    "units": [
                        { "character_id": "TFT17_Talon", "tier": 2, "rarity": 1 },
                        { "character_id": "TFT17_Ahri", "tier": 1, "rarity": 2 }
                    ],
                    "augments": [1, 2]
                }]
            }
        });
        let game = summarize_game(&entry, "p1", &Catalog::empty()).unwrap();
        assert_eq!(game["placement"], 2);
        assert_eq!(game["win"], true);
        assert_eq!(game["duration"], 1850);
        assert_eq!(game["setNumber"], 17);
        assert_eq!(game["units"].as_array().unwrap().len(), 2);
        assert_eq!(game["units"][0]["alias"], "Ahri");
        assert_eq!(game["augments"].as_array().unwrap().len(), 2);
    }
}
