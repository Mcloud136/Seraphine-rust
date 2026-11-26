use serde_json::{json, Value};

use crate::catalog::Catalog;
use crate::error::CoreError;
use crate::session::SessionManager;

pub async fn current_summoner(session: &SessionManager) -> Result<Value, CoreError> {
    session.lcu_get("/lol-summoner/v1/current-summoner").await
}

pub async fn summoner_by_puuid(session: &SessionManager, puuid: &str) -> Result<Value, CoreError> {
    let path = format!(
        "/lol-summoner/v2/summoners/puuid/{}",
        urlencoding::encode(puuid)
    );
    session.lcu_get(&path).await
}

/// Resolves a summoner by display name or by PUUID.
pub async fn resolve_summoner(session: &SessionManager, query: &str) -> Result<Value, CoreError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(CoreError::InvalidParams("summoner name is empty".into()));
    }
    let path = if query.len() > 30 && query.contains('-') {
        format!(
            "/lol-summoner/v2/summoners/puuid/{}",
            urlencoding::encode(query)
        )
    } else {
        format!(
            "/lol-summoner/v1/summoners?name={}",
            urlencoding::encode(query)
        )
    };
    session.lcu_get(&path).await
}

pub async fn ranked_stats(session: &SessionManager, puuid: &str) -> Result<Value, CoreError> {
    let path = format!("/lol-ranked/v1/ranked-stats/{puuid}");
    session.lcu_get(&path).await
}

pub async fn match_history(
    session: &SessionManager,
    puuid: &str,
    begin: u32,
    end: u32,
) -> Result<Value, CoreError> {
    let path = format!(
        "/lol-match-history/v1/products/lol/{puuid}/matches?begIndex={begin}&endIndex={end}"
    );
    session.lcu_get(&path).await
}

pub async fn match_detail(session: &SessionManager, game_id: u64) -> Result<Value, CoreError> {
    let path = format!("/lol-match-history/v1/games/{game_id}");
    session.lcu_get(&path).await
}

pub async fn platform_config(session: &SessionManager) -> Result<Value, CoreError> {
    session.lcu_get("/lol-platform-config/v1/namespaces").await
}

/// Builds the career page payload: summoner, ranked queues and recent games.
pub async fn overview(
    session: &SessionManager,
    catalog: &Catalog,
    limit: u32,
) -> Result<Value, CoreError> {
    let summoner = current_summoner(session).await?;
    let puuid = summoner
        .get("puuid")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| CoreError::InvalidParams("summoner has no puuid".into()))?;
    overview_for_puuid(session, catalog, &puuid, limit, Some(summoner)).await
}

pub async fn overview_for_puuid(
    session: &SessionManager,
    catalog: &Catalog,
    puuid: &str,
    limit: u32,
    summoner: Option<Value>,
) -> Result<Value, CoreError> {
    let summoner = match summoner {
        Some(summoner) => summoner,
        None => summoner_by_puuid(session, puuid).await?,
    };
    let ranked = ranked_stats(session, puuid).await.unwrap_or(Value::Null);
    let history = match_history(session, puuid, 0, limit).await?;
    let matches = summarize_history(session, catalog, &history, puuid, limit).await;

    Ok(json!({
        "summoner": summoner,
        "ranked": ranked,
        "matches": matches,
    }))
}

/// Loads one page of games for the search view.
pub async fn matches_page(
    session: &SessionManager,
    catalog: &Catalog,
    puuid: &str,
    begin: u32,
    end: u32,
) -> Result<Value, CoreError> {
    let history = match_history(session, puuid, begin, end).await?;
    let limit = end.saturating_sub(begin).saturating_add(1);
    let matches = summarize_history(session, catalog, &history, puuid, limit).await;
    Ok(json!({ "matches": matches, "hasMore": matches.len() as u32 >= limit }))
}

async fn summarize_history(
    session: &SessionManager,
    catalog: &Catalog,
    history: &Value,
    puuid: &str,
    limit: u32,
) -> Vec<Value> {
    let games = history
        .get("games")
        .and_then(|games| games.get("games"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut matches = Vec::with_capacity(games.len().min(limit as usize));
    for game in games.iter().take(limit as usize) {
        let Some(game_id) = game.get("gameId").and_then(Value::as_u64) else {
            continue;
        };
        let detail = match_detail(session, game_id).await.ok();
        matches.push(summarize_game(game, detail.as_ref(), puuid, catalog));
    }
    matches
}

fn summarize_game(game: &Value, detail: Option<&Value>, puuid: &str, catalog: &Catalog) -> Value {
    let source = detail.unwrap_or(game);
    let participant = source
        .get("participantIdentities")
        .and_then(Value::as_array)
        .and_then(|identities| {
            identities.iter().find(|identity| {
                identity
                    .get("player")
                    .and_then(|player| player.get("puuid"))
                    .and_then(Value::as_str)
                    == Some(puuid)
            })
        })
        .and_then(|identity| identity.get("participantId"))
        .and_then(Value::as_u64);

    let participants = source.get("participants").and_then(Value::as_array);
    let stats = participants
        .and_then(|participants| {
            participants.iter().find(|candidate| {
                participant.is_some_and(|id| {
                    candidate.get("participantId").and_then(Value::as_u64) == Some(id)
                })
            })
        })
        .or_else(|| participants.and_then(|participants| participants.first()));

    let stats = stats.and_then(|entry| entry.get("stats"));
    let champion_id = stats
        .and_then(|stats| stats.get("championId"))
        .and_then(Value::as_u64)
        .or_else(|| {
            source
                .get("participants")
                .and_then(Value::as_array)
                .and_then(|participants| {
                    participants
                        .iter()
                        .find(|entry| {
                            entry.get("participantId").and_then(Value::as_u64) == participant
                        })
                        .and_then(|entry| entry.get("championId"))
                        .and_then(Value::as_u64)
                })
        })
        .unwrap_or(0);
    let queue_id = source
        .get("queueId")
        .and_then(Value::as_u64)
        .or_else(|| game.get("queueId").and_then(Value::as_u64))
        .unwrap_or(0);
    let queue_name = catalog
        .queue_name(queue_id)
        .map(str::to_string)
        .or_else(|| {
            source
                .get("gameMode")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("Queue {queue_id}"));
    let champion_name = catalog
        .champion_name(champion_id)
        .map(str::to_string)
        .unwrap_or_else(|| format!("#{champion_id}"));

    json!({
        "gameId": source.get("gameId").and_then(Value::as_u64).unwrap_or(0),
        "queueId": queue_id,
        "queueName": queue_name,
        "gameMode": source.get("gameMode").and_then(Value::as_str).unwrap_or(""),
        "gameCreation": source.get("gameCreation").and_then(Value::as_u64).unwrap_or(0),
        "gameDuration": source.get("gameDuration").and_then(Value::as_u64).unwrap_or(0),
        "championId": champion_id,
        "championName": champion_name,
        "win": stats.and_then(|stats| stats.get("win")).and_then(Value::as_bool).unwrap_or(false),
        "kills": stats.and_then(|stats| stats.get("kills")).and_then(Value::as_u64).unwrap_or(0),
        "deaths": stats.and_then(|stats| stats.get("deaths")).and_then(Value::as_u64).unwrap_or(0),
        "assists": stats.and_then(|stats| stats.get("assists")).and_then(Value::as_u64).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarizes_the_searched_participant() {
        let game = json!({ "gameId": 42, "queueId": 450 });
        let detail = json!({
            "gameId": 42,
            "queueId": 450,
            "gameMode": "ARAM",
            "gameCreation": 1_700_000_000_000u64,
            "gameDuration": 900,
            "participantIdentities": [
                { "participantId": 3, "player": { "puuid": "p1" } }
            ],
            "participants": [
                {
                    "participantId": 3,
                    "teamId": 100,
                    "championId": 1,
                    "stats": { "win": true, "kills": 5, "deaths": 2, "assists": 10 }
                },
                {
                    "participantId": 4,
                    "teamId": 200,
                    "championId": 2,
                    "stats": { "win": false, "kills": 1, "deaths": 8, "assists": 3 }
                }
            ]
        });

        let summary = summarize_game(&game, Some(&detail), "p1", &Catalog::empty());
        assert_eq!(summary["gameId"], 42);
        assert_eq!(summary["queueId"], 450);
        assert_eq!(summary["queueName"], "ARAM");
        assert_eq!(summary["championId"], 1);
        assert_eq!(summary["championName"], "#1");
        assert_eq!(summary["win"], true);
        assert_eq!(summary["kills"], 5);
        assert_eq!(summary["deaths"], 2);
        assert_eq!(summary["assists"], 10);
    }

    #[test]
    fn falls_back_to_the_list_entry_without_details() {
        let game = json!({
            "gameId": 7,
            "queueId": 420,
            "gameMode": "CLASSIC",
            "gameCreation": 1_700_000_000_000u64,
            "gameDuration": 1800
        });
        let summary = summarize_game(&game, None, "p1", &Catalog::empty());
        assert_eq!(summary["gameId"], 7);
        assert_eq!(summary["championId"], 0);
        assert_eq!(summary["win"], false);
        assert_eq!(summary["kills"], 0);
    }
}
