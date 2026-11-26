use futures_util::future::join_all;
use serde_json::{json, Value};

use crate::career;
use crate::catalog::Catalog;
use crate::error::CoreError;
use crate::session::SessionManager;

const RECENT_GAMES: usize = 11;
const SGP_FETCH_COUNT: u32 = 15;

/// Loads one side of the current champion select or game session.
pub async fn fetch(
    session: &SessionManager,
    catalog: &Catalog,
    side: &str,
    queue_filter: Option<&serde_json::Map<String, Value>>,
) -> Result<Value, CoreError> {
    let champ_select = session.lcu_get("/lol-champ-select/v1/session").await.ok();
    let gameflow = session.lcu_get("/lol-gameflow/v1/session").await.ok();
    let current = career::current_summoner(session).await.ok();
    let current_id = current
        .as_ref()
        .and_then(|summoner| summoner.get("summonerId"))
        .and_then(Value::as_u64);

    let queue_id = gameflow
        .as_ref()
        .and_then(|session| session.pointer("/gameData/queue/id"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let is_aram = champ_select
        .as_ref()
        .and_then(|session| session.get("benchEnabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let allowed_queues = queue_filter
        .and_then(|filter| filter.get(queue_id.to_string().as_str()))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_u64).collect::<Vec<_>>())
        .unwrap_or_default();

    let members = match side {
        "ally" => ally_members(champ_select.as_ref(), gameflow.as_ref(), current_id),
        "enemy" => enemy_members(gameflow.as_ref(), current_id),
        other => {
            return Err(CoreError::InvalidParams(format!(
                "unknown game info side: {other}"
            )))
        }
    };

    let futures = members.iter().map(|member| {
        member_info(
            session,
            catalog,
            member,
            queue_id,
            current_id,
            &allowed_queues,
        )
    });
    let mut summoners = join_all(futures)
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    if !summoners.is_empty() {
        summoners.sort_by_key(|summoner| {
            (
                summoner
                    .get("cellId")
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::MAX),
                summoner
                    .get("summonerId")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            )
        });
    }

    if matches!(queue_id, 420 | 440) {
        summoners = sort_by_ranked_position(summoners);
    }

    Ok(json!({
        "side": side,
        "queueId": queue_id,
        "isAram": is_aram,
        "summoners": summoners,
        "currentSummonerId": current_id,
    }))
}

fn ally_members(
    champ_select: Option<&Value>,
    gameflow: Option<&Value>,
    current_id: Option<u64>,
) -> Vec<Value> {
    if let Some(team) = champ_select
        .and_then(|session| session.get("myTeam"))
        .and_then(Value::as_array)
    {
        if !team.is_empty() {
            return team.clone();
        }
    }
    let (ally, _) = separate_gameflow(gameflow, current_id);
    ally
}

fn enemy_members(gameflow: Option<&Value>, current_id: Option<u64>) -> Vec<Value> {
    let (_, enemy) = separate_gameflow(gameflow, current_id);
    enemy
}

fn separate_gameflow(
    gameflow: Option<&Value>,
    current_id: Option<u64>,
) -> (Vec<Value>, Vec<Value>) {
    let Some(data) = gameflow.and_then(|session| session.get("gameData")) else {
        return (Vec::new(), Vec::new());
    };
    let team_one = data
        .get("teamOne")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let team_two = data
        .get("teamTwo")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let in_team_one = current_id.is_some_and(|id| {
        team_one
            .iter()
            .any(|member| member.get("summonerId").and_then(Value::as_u64) == Some(id))
    });
    if in_team_one {
        (team_one, team_two)
    } else {
        (team_two, team_one)
    }
}

fn sort_by_ranked_position(mut summoners: Vec<Value>) -> Vec<Value> {
    const POSITIONS: [&str; 5] = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"];
    let positions = summoners
        .iter()
        .map(|summoner| {
            summoner
                .get("position")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();
    if positions
        .iter()
        .all(|position| POSITIONS.contains(&position.as_str()))
    {
        summoners.sort_by_key(|summoner| {
            let position = summoner
                .get("position")
                .and_then(Value::as_str)
                .unwrap_or_default();
            POSITIONS
                .iter()
                .position(|candidate| candidate == &position)
                .unwrap_or(POSITIONS.len())
        });
    }
    summoners
}

async fn member_info(
    session: &SessionManager,
    catalog: &Catalog,
    member: &Value,
    queue_id: u64,
    current_id: Option<u64>,
    allowed_queues: &[u64],
) -> Option<Value> {
    if member.get("nameVisibilityType").and_then(Value::as_str) == Some("HIDDEN") {
        return None;
    }
    let summoner_id = member
        .get("summonerId")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)?;
    let summoner = session
        .lcu_get(&format!("/lol-summoner/v1/summoners/{summoner_id}"))
        .await
        .ok()?;
    let puuid = summoner.get("puuid").and_then(Value::as_str)?;
    if puuid.is_empty() || puuid == "00000000-0000-0000-0000-000000000000" {
        return None;
    }

    let ranked = career::ranked_stats(session, puuid)
        .await
        .unwrap_or(Value::Null);
    let recent = recent_games(
        session,
        catalog,
        puuid,
        queue_id,
        current_id,
        allowed_queues,
    )
    .await;
    let champion_id = member
        .get("championId")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let position = member
        .get("selectedPosition")
        .or_else(|| member.get("assignedPosition"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    Some(json!({
        "summonerId": summoner_id,
        "puuid": puuid,
        "name": summoner
            .get("gameName")
            .or_else(|| summoner.get("displayName"))
            .or_else(|| summoner.get("internalName"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        "tagLine": summoner.get("tagLine").and_then(Value::as_str).unwrap_or(""),
        "level": summoner.get("summonerLevel").and_then(Value::as_u64).unwrap_or(0),
        "privacy": summoner.get("privacy").and_then(Value::as_str).unwrap_or(""),
        "championId": champion_id,
        "championName": catalog.champion_name(champion_id).unwrap_or(""),
        "cellId": member.get("cellId").and_then(Value::as_u64),
        "position": position,
        "rank": rank_summary(&ranked),
        "recent": recent,
    }))
}

fn rank_summary(ranked: &Value) -> Value {
    let pick = |key: &str| {
        let queue = &ranked["queueMap"][key];
        if queue.is_null() {
            Value::Null
        } else {
            json!({
                "tier": queue.get("tier").and_then(Value::as_str).unwrap_or(""),
                "division": queue.get("division").and_then(Value::as_str).unwrap_or(""),
                "lp": queue.get("leaguePoints").and_then(Value::as_u64).unwrap_or(0),
                "wins": queue.get("wins").and_then(Value::as_u64).unwrap_or(0),
                "losses": queue.get("losses").and_then(Value::as_u64).unwrap_or(0),
            })
        }
    };
    json!({
        "solo": pick("RANKED_SOLO_5x5"),
        "flex": pick("RANKED_FLEX_SR"),
    })
}

async fn recent_games(
    session: &SessionManager,
    catalog: &Catalog,
    puuid: &str,
    _queue_id: u64,
    current_id: Option<u64>,
    allowed_queues: &[u64],
) -> Value {
    let path = format!("/match-history-query/v1/products/lol/player/{puuid}/SUMMARY");
    let params = json!({ "startIndex": 0, "count": SGP_FETCH_COUNT });
    if let Ok(data) = session.sgp_get(&path, Some(params)).await {
        if let Some(summary) = parse_sgp_summary(&data, puuid, current_id, catalog, allowed_queues)
        {
            return summary;
        }
    }
    lcu_recent_games(session, catalog, puuid, current_id, allowed_queues)
        .await
        .unwrap_or_else(empty_recent)
}

fn empty_recent() -> Value {
    json!({
        "games": [],
        "kills": 0,
        "deaths": 0,
        "assists": 0,
        "wins": 0,
        "losses": 0,
        "recentChampionId": 0,
        "recentChampionName": "",
        "fateFlag": Value::Null,
    })
}

fn parse_sgp_summary(
    data: &Value,
    puuid: &str,
    current_id: Option<u64>,
    catalog: &Catalog,
    allowed_queues: &[u64],
) -> Option<Value> {
    let games = data.get("games").and_then(Value::as_array)?;
    let mut parsed = Vec::new();
    let mut kills = 0_u64;
    let mut deaths = 0_u64;
    let mut assists = 0_u64;
    let mut wins = 0_u64;
    let mut losses = 0_u64;
    let mut recent_champion_id = 0_u64;
    let mut fate_flag = Value::Null;

    for (index, entry) in games.iter().take(RECENT_GAMES).enumerate() {
        let game = entry.get("json").unwrap_or(entry);
        if !allowed_queues.is_empty() {
            let queue = game.get("queueId").and_then(Value::as_u64).unwrap_or(0);
            if !allowed_queues.contains(&queue) {
                continue;
            }
        }
        let Some(participants) = game.get("participants").and_then(Value::as_array) else {
            continue;
        };
        let Some(me) = participants
            .iter()
            .find(|participant| participant.get("puuid").and_then(Value::as_str) == Some(puuid))
        else {
            continue;
        };
        let remake = me
            .get("gameEndedInEarlySurrender")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let win = me.get("win").and_then(Value::as_bool).unwrap_or(false);
        let champion_id = me.get("championId").and_then(Value::as_u64).unwrap_or(0);
        let game_kills = me.get("kills").and_then(Value::as_u64).unwrap_or(0);
        let game_deaths = me.get("deaths").and_then(Value::as_u64).unwrap_or(0);
        let game_assists = me.get("assists").and_then(Value::as_u64).unwrap_or(0);

        if !remake {
            kills += game_kills;
            deaths += game_deaths;
            assists += game_assists;
            if win {
                wins += 1;
            } else {
                losses += 1;
            }
        }

        if index == 0 {
            recent_champion_id = champion_id;
            if let (Some(current), Some(team)) =
                (current_id, me.get("teamId").and_then(Value::as_u64))
            {
                for participant in participants {
                    if participant.get("summonerId").and_then(Value::as_u64) != Some(current) {
                        continue;
                    }
                    let same_team = participant.get("teamId").and_then(Value::as_u64) == Some(team);
                    fate_flag = Value::String(if same_team { "ally" } else { "enemy" }.into());
                    break;
                }
            }
        }

        parsed.push(json!({
            "gameId": game.get("gameId").and_then(Value::as_u64).unwrap_or(0),
            "queueId": game.get("queueId").and_then(Value::as_u64).unwrap_or(0),
            "gameCreation": game.get("gameCreation").and_then(Value::as_u64).unwrap_or(0),
            "gameDuration": game.get("gameDuration").and_then(Value::as_u64).unwrap_or(0),
            "championId": champion_id,
            "championName": catalog.champion_name(champion_id).unwrap_or(""),
            "kills": game_kills,
            "deaths": game_deaths,
            "assists": game_assists,
            "win": win,
            "remake": remake,
            "cs": me.get("totalMinionsKilled").and_then(Value::as_u64).unwrap_or(0)
                + me.get("neutralMinionsKilled").and_then(Value::as_u64).unwrap_or(0),
        }));
    }

    Some(json!({
        "games": parsed,
        "kills": kills,
        "deaths": deaths,
        "assists": assists,
        "wins": wins,
        "losses": losses,
        "recentChampionId": recent_champion_id,
        "recentChampionName": catalog.champion_name(recent_champion_id).unwrap_or(""),
        "fateFlag": fate_flag,
    }))
}

async fn lcu_recent_games(
    session: &SessionManager,
    catalog: &Catalog,
    puuid: &str,
    current_id: Option<u64>,
    allowed_queues: &[u64],
) -> Option<Value> {
    let history = career::match_history(session, puuid, 0, 4).await.ok()?;
    let games = history
        .get("games")
        .and_then(|games| games.get("games"))
        .and_then(Value::as_array)?
        .iter()
        .filter(|game| {
            allowed_queues.is_empty()
                || game
                    .get("queueId")
                    .and_then(Value::as_u64)
                    .is_some_and(|queue| allowed_queues.contains(&queue))
        })
        .cloned()
        .collect::<Vec<_>>();
    let futures = games.iter().take(5).map(|game| {
        let game_id = game.get("gameId").and_then(Value::as_u64).unwrap_or(0);
        career::match_detail(session, game_id)
    });
    let details = join_all(futures).await;
    let mut parsed = Vec::new();
    let mut kills = 0_u64;
    let mut deaths = 0_u64;
    let mut assists = 0_u64;
    let mut wins = 0_u64;
    let mut losses = 0_u64;
    let mut recent_champion_id = 0_u64;
    let mut fate_flag = Value::Null;

    for (index, detail) in details.into_iter().enumerate() {
        let Ok(game) = detail else { continue };
        let Some(participants) = game.get("participants").and_then(Value::as_array) else {
            continue;
        };
        let Some(participant_id) = game
            .get("participantIdentities")
            .and_then(Value::as_array)
            .and_then(|identities| {
                identities.iter().find(|identity| {
                    identity.pointer("/player/puuid").and_then(Value::as_str) == Some(puuid)
                })
            })
            .and_then(|identity| identity.get("participantId"))
            .and_then(Value::as_u64)
        else {
            continue;
        };
        let Some(me) = participants.iter().find(|participant| {
            participant.get("participantId").and_then(Value::as_u64) == Some(participant_id)
        }) else {
            continue;
        };
        let null_stats = Value::Null;
        let stats = me.get("stats").unwrap_or(&null_stats);
        let win = stats.get("win").and_then(Value::as_bool).unwrap_or(false);
        let game_kills = stats.get("kills").and_then(Value::as_u64).unwrap_or(0);
        let game_deaths = stats.get("deaths").and_then(Value::as_u64).unwrap_or(0);
        let game_assists = stats.get("assists").and_then(Value::as_u64).unwrap_or(0);
        let champion_id = me.get("championId").and_then(Value::as_u64).unwrap_or(0);
        kills += game_kills;
        deaths += game_deaths;
        assists += game_assists;
        if win {
            wins += 1;
        } else {
            losses += 1;
        }
        if index == 0 {
            recent_champion_id = champion_id;
            if let (Some(current), Some(team)) =
                (current_id, me.get("teamId").and_then(Value::as_u64))
            {
                for other in participants {
                    if other.get("summonerId").and_then(Value::as_u64) != Some(current) {
                        continue;
                    }
                    let same_team = other.get("teamId").and_then(Value::as_u64) == Some(team);
                    fate_flag = Value::String(if same_team { "ally" } else { "enemy" }.into());
                    break;
                }
            }
        }
        parsed.push(json!({
            "gameId": game.get("gameId").and_then(Value::as_u64).unwrap_or(0),
            "queueId": game.get("queueId").and_then(Value::as_u64).unwrap_or(0),
            "gameCreation": game.get("gameCreation").and_then(Value::as_u64).unwrap_or(0),
            "gameDuration": game.get("gameDuration").and_then(Value::as_u64).unwrap_or(0),
            "championId": champion_id,
            "championName": catalog.champion_name(champion_id).unwrap_or(""),
            "kills": game_kills,
            "deaths": game_deaths,
            "assists": game_assists,
            "win": win,
            "remake": stats
                .get("gameEndedInEarlySurrender")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "cs": stats.get("totalMinionsKilled").and_then(Value::as_u64).unwrap_or(0)
                + stats.get("neutralMinionsKilled").and_then(Value::as_u64).unwrap_or(0),
        }));
    }

    Some(json!({
        "games": parsed,
        "kills": kills,
        "deaths": deaths,
        "assists": assists,
        "wins": wins,
        "losses": losses,
        "recentChampionId": recent_champion_id,
        "recentChampionName": catalog.champion_name(recent_champion_id).unwrap_or(""),
        "fateFlag": fate_flag,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::empty()
    }

    #[test]
    fn parses_sgp_summary_and_fate() {
        let data = json!({
            "games": [
                { "json": {
                    "gameId": 1, "queueId": 420, "gameCreation": 1000, "gameDuration": 1800,
                    "participants": [
                        { "puuid": "p1", "summonerId": 11, "teamId": 100, "championId": 7,
                          "win": true, "kills": 5, "deaths": 2, "assists": 8,
                          "totalMinionsKilled": 100, "neutralMinionsKilled": 10,
                          "gameEndedInEarlySurrender": false },
                        { "puuid": "p2", "summonerId": 22, "teamId": 100, "championId": 8,
                          "win": true, "kills": 0, "deaths": 0, "assists": 0 },
                        { "puuid": "p3", "summonerId": 33, "teamId": 200, "championId": 9,
                          "win": false, "kills": 0, "deaths": 0, "assists": 0 }
                    ]
                }},
                { "json": {
                    "gameId": 2, "queueId": 420, "gameCreation": 2000, "gameDuration": 1800,
                    "participants": [
                        { "puuid": "p1", "summonerId": 11, "teamId": 100, "championId": 7,
                          "win": false, "kills": 1, "deaths": 4, "assists": 2,
                          "totalMinionsKilled": 50, "neutralMinionsKilled": 0,
                          "gameEndedInEarlySurrender": false }
                    ]
                }}
            ]
        });
        let summary = parse_sgp_summary(&data, "p1", Some(22), &catalog(), &[]).unwrap();
        assert_eq!(summary["kills"], 6);
        assert_eq!(summary["deaths"], 6);
        assert_eq!(summary["assists"], 10);
        assert_eq!(summary["wins"], 1);
        assert_eq!(summary["losses"], 1);
        assert_eq!(summary["recentChampionId"], 7);
        assert_eq!(summary["fateFlag"], "ally");
        assert_eq!(summary["games"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn filters_games_by_allowed_queues() {
        let data = json!({
            "games": [
                { "json": {
                    "gameId": 1, "queueId": 420, "gameCreation": 1000, "gameDuration": 1800,
                    "participants": [{ "puuid": "p1", "summonerId": 11, "teamId": 100,
                        "championId": 7, "win": true, "kills": 5, "deaths": 2, "assists": 8 }]
                }},
                { "json": {
                    "gameId": 2, "queueId": 450, "gameCreation": 2000, "gameDuration": 900,
                    "participants": [{ "puuid": "p1", "summonerId": 11, "teamId": 100,
                        "championId": 9, "win": false, "kills": 1, "deaths": 4, "assists": 2 }]
                }}
            ]
        });
        let summary = parse_sgp_summary(&data, "p1", None, &catalog(), &[450]).unwrap();
        assert_eq!(summary["games"].as_array().unwrap().len(), 1);
        assert_eq!(summary["games"][0]["gameId"], 2);
        assert_eq!(summary["wins"], 0);
        assert_eq!(summary["losses"], 1);
        assert_eq!(summary["kills"], 1);

        let all = parse_sgp_summary(&data, "p1", None, &catalog(), &[]).unwrap();
        assert_eq!(all["games"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn skips_remake_aggregates_but_keeps_the_game() {
        let data = json!({
            "games": [{ "json": {
                "gameId": 3, "queueId": 450, "gameCreation": 3000, "gameDuration": 300,
                "participants": [
                    { "puuid": "p1", "summonerId": 11, "teamId": 100, "championId": 1,
                      "win": true, "kills": 9, "deaths": 9, "assists": 9,
                      "gameEndedInEarlySurrender": true }
                ]
            }}]
        });
        let summary = parse_sgp_summary(&data, "p1", None, &catalog(), &[]).unwrap();
        assert_eq!(summary["kills"], 0);
        assert_eq!(summary["wins"], 0);
        assert_eq!(summary["games"].as_array().unwrap().len(), 1);
        assert_eq!(summary["fateFlag"], Value::Null);
    }
}
