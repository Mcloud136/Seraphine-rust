use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tauri::Manager;

use seraphine_core::config::FunctionConfig;
use seraphine_core::CoreError;

use crate::AppState;

const LOCK_LEAD_MS: i64 = 4_000;

#[derive(Default)]
pub struct AutomationState {
    active: bool,
    token: String,
    draft_id: Option<u64>,
    spells_done: bool,
    ban_done: bool,
    pick_done: bool,
    complete_done: bool,
    spells_attempt: u32,
    ban_attempt: u32,
    pick_attempt: u32,
    complete_attempt: u32,
    ban_ready_at: Option<Instant>,
}

pub fn begin(app: &tauri::AppHandle, _epoch: u64, _seq: u64) {
    let state = app.state::<AppState>();
    let Ok(mut automation) = state.automation.lock() else {
        return;
    };
    *automation = AutomationState {
        active: true,
        ..AutomationState::default()
    };
}

pub fn reset(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let Ok(mut automation) = state.automation.lock() else {
        return;
    };
    *automation = AutomationState::default();
}

pub async fn on_champ_select(app: tauri::AppHandle, epoch: u64, _seq: u64, data: Value) {
    let state = app.state::<AppState>();
    let _serial = state.automation_lock.lock().await;
    let config = state.config.snapshot().await;
    let functions = &config.functions;
    let session = data;
    let draft_id = session.get("gameId").and_then(Value::as_u64);
    let token = {
        let Ok(mut automation) = state.automation.lock() else {
            return;
        };
        let new_draft = !automation.active
            || draft_id.is_some_and(|draft_id| automation.draft_id != Some(draft_id));
        if new_draft {
            *automation = AutomationState {
                active: true,
                draft_id,
                ..AutomationState::default()
            };
        }
        if automation.token.is_empty() {
            // Guard keys are persisted per client pid; epoch:seq repeats across
            // drafts, so a per-draft nonce keeps operation ids unique.
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0);
            automation.token = format!("{epoch}-{nanos}");
        }
        automation.token.clone()
    };
    let Some(local_cell) = session.get("localPlayerCellId").and_then(Value::as_u64) else {
        return;
    };
    let my_team = session
        .get("myTeam")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(me) = my_team
        .iter()
        .find(|player| player.get("cellId").and_then(Value::as_u64) == Some(local_cell))
    else {
        return;
    };
    let position = me
        .get("assignedPosition")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let champion_id = me.get("championId").and_then(Value::as_u64).unwrap_or(0);
    let pick_intent = me
        .get("championPickIntent")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let bans = session
        .get("bans")
        .map(|bans| {
            ["myTeamBans", "theirTeamBans"]
                .iter()
                .flat_map(|key| {
                    bans.get(key)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_u64)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let phase = session
        .get("timer")
        .and_then(|timer| timer.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if matches!(phase, "BAN_PICK" | "FINALIZATION") {
        auto_spells(&state, functions, &token, &position).await;
    }
    auto_ban(
        &state, functions, &token, &session, local_cell, &position, &my_team, &bans,
    )
    .await;
    auto_pick(
        &state,
        functions,
        &token,
        &session,
        local_cell,
        &position,
        champion_id,
        pick_intent,
        &bans,
    )
    .await;
    auto_complete(&state, functions, &token, &session, local_cell, &position).await;
    auto_trades(&state, functions, &session, &token).await;
}

fn terminal(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::OperationConflict(_) | CoreError::OutcomeUnknown(_)
    )
}

const MAX_ATTEMPTS: u32 = 5;

fn position_values(values: &[u64], general: &[u64]) -> Vec<u64> {
    let mut result = values.to_vec();
    result.extend_from_slice(general);
    result
}

fn candidates(config: &FunctionConfig, kind: &str, position: &str) -> Vec<u64> {
    let (specific, general): (&[u64], &[u64]) = match (kind, position) {
        ("ban", "top") => (&config.auto_ban_champion_top, &config.auto_ban_champion),
        ("ban", "jungle") => (&config.auto_ban_champion_jug, &config.auto_ban_champion),
        ("ban", "middle") => (&config.auto_ban_champion_mid, &config.auto_ban_champion),
        ("ban", "bottom") => (&config.auto_ban_champion_bot, &config.auto_ban_champion),
        ("ban", "utility") => (&config.auto_ban_champion_sup, &config.auto_ban_champion),
        ("pick", "top") => (
            &config.auto_select_champion_top,
            &config.auto_select_champion,
        ),
        ("pick", "jungle") => (
            &config.auto_select_champion_jug,
            &config.auto_select_champion,
        ),
        ("pick", "middle") => (
            &config.auto_select_champion_mid,
            &config.auto_select_champion,
        ),
        ("pick", "bottom") => (
            &config.auto_select_champion_bot,
            &config.auto_select_champion,
        ),
        ("pick", "utility") => (
            &config.auto_select_champion_sup,
            &config.auto_select_champion,
        ),
        _ => (&[], &[]),
    };
    position_values(specific, general)
}

async fn auto_spells(state: &AppState, config: &FunctionConfig, token: &str, position: &str) {
    if !config.enable_auto_set_spells {
        return;
    }
    {
        let Ok(automation) = state.automation.lock() else {
            return;
        };
        if automation.spells_done || automation.spells_attempt >= MAX_ATTEMPTS {
            return;
        }
    }
    let attempt = {
        let Ok(automation) = state.automation.lock() else {
            return;
        };
        automation.spells_attempt
    };
    let specific = match position {
        "top" => &config.auto_set_summoner_spell_top,
        "jungle" => &config.auto_set_summoner_spell_jug,
        "middle" => &config.auto_set_summoner_spell_mid,
        "bottom" => &config.auto_set_summoner_spell_bot,
        "utility" => &config.auto_set_summoner_spell_sup,
        _ => &config.auto_set_summoner_spell,
    };
    let mut spells = specific.clone();
    if spells.contains(&54) {
        spells = config.auto_set_summoner_spell.clone();
    }
    if spells.len() < 2 || spells.contains(&54) {
        if let Ok(mut automation) = state.automation.lock() {
            automation.spells_done = true;
        }
        return;
    }
    let operation_id = format!("spells:{token}:{attempt}");
    let result = state
        .writes
        .execute(
            &operation_id,
            "PATCH",
            "/lol-champ-select/v1/session/my-selection",
            Some(json!({ "spell1Id": spells[0], "spell2Id": spells[1] })),
        )
        .await;
    match result {
        Ok(_) => {
            if let Ok(mut automation) = state.automation.lock() {
                automation.spells_done = true;
            }
            log::info!("auto set summoner spells");
        }
        Err(error) => {
            if let Ok(mut automation) = state.automation.lock() {
                if terminal(&error) || attempt + 1 >= MAX_ATTEMPTS {
                    automation.spells_done = true;
                } else {
                    automation.spells_attempt = attempt + 1;
                }
            }
            log::warn!("auto set spells failed (attempt {attempt}): {error}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn auto_ban(
    state: &AppState,
    config: &FunctionConfig,
    token: &str,
    session: &Value,
    local_cell: u64,
    position: &str,
    my_team: &[Value],
    bans: &[u64],
) {
    if !config.enable_auto_ban_champion {
        return;
    }
    let attempt = {
        let Ok(automation) = state.automation.lock() else {
            return;
        };
        if automation.ban_done || automation.ban_attempt >= MAX_ATTEMPTS {
            return;
        }
        automation.ban_attempt
    };
    // The client accepts ban PATCHes during PLANNING but silently ignores them;
    // bans only take effect once the BAN_PICK phase starts.
    if session
        .get("timer")
        .and_then(|timer| timer.get("phase"))
        .and_then(Value::as_str)
        != Some("BAN_PICK")
    {
        return;
    }
    let Some(action_id) = find_action(session, local_cell, "ban", true) else {
        return;
    };
    let mut pool = candidates(config, "ban", position)
        .into_iter()
        .filter(|candidate| !bans.contains(candidate))
        .collect::<Vec<_>>();
    if config.prentent_ban {
        let intents = my_team
            .iter()
            .filter_map(|player| player.get("championPickIntent").and_then(Value::as_u64))
            .filter(|intent| *intent > 0)
            .collect::<Vec<_>>();
        pool.retain(|candidate| !intents.contains(candidate));
    }
    let Some(champion_id) = pool.first().copied() else {
        if let Ok(mut automation) = state.automation.lock() {
            automation.ban_done = true;
        }
        return;
    };
    if config.auto_ban_delay > 0 {
        let ready_at = {
            let Ok(mut automation) = state.automation.lock() else {
                return;
            };
            *automation
                .ban_ready_at
                .get_or_insert_with(|| Instant::now() + Duration::from_secs(config.auto_ban_delay))
        };
        if Instant::now() < ready_at {
            return;
        }
    }
    let path = format!("/lol-champ-select/v1/session/actions/{action_id}");
    let hover = state
        .writes
        .execute(
            &format!("ban:{token}:{action_id}:{champion_id}:{attempt}:set"),
            "PATCH",
            &path,
            Some(json!({ "championId": champion_id, "type": "ban" })),
        )
        .await;
    let result = match hover {
        Ok(_) => {
            state
                .writes
                .execute(
                    &format!("ban:{token}:{action_id}:{champion_id}:{attempt}:done"),
                    "PATCH",
                    &path,
                    Some(json!({ "championId": champion_id, "type": "ban", "completed": true })),
                )
                .await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(_) => {
            if let Ok(mut automation) = state.automation.lock() {
                automation.ban_done = true;
            }
            log::info!("auto banned champion {champion_id}");
        }
        Err(error) => {
            if let Ok(mut automation) = state.automation.lock() {
                if terminal(&error) || attempt + 1 >= MAX_ATTEMPTS {
                    automation.ban_done = true;
                } else {
                    automation.ban_attempt = attempt + 1;
                }
            }
            log::warn!("auto ban failed (attempt {attempt}): {error}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn auto_pick(
    state: &AppState,
    config: &FunctionConfig,
    token: &str,
    session: &Value,
    local_cell: u64,
    position: &str,
    champion_id: u64,
    pick_intent: u64,
    bans: &[u64],
) {
    if !config.enable_auto_select_champion {
        return;
    }
    {
        let Ok(automation) = state.automation.lock() else {
            return;
        };
        if automation.pick_done || automation.pick_attempt >= MAX_ATTEMPTS {
            return;
        }
    }
    let attempt = {
        let Ok(automation) = state.automation.lock() else {
            return;
        };
        automation.pick_attempt
    };
    if champion_id > 0 || pick_intent > 0 {
        if let Ok(mut automation) = state.automation.lock() {
            automation.pick_done = true;
        }
        return;
    }
    let Some(action_id) = find_action(session, local_cell, "pick", true) else {
        return;
    };
    let pool = candidates(config, "pick", position)
        .into_iter()
        .filter(|candidate| !bans.contains(candidate))
        .collect::<Vec<_>>();
    let Some(champion_id) = pool.first().copied() else {
        if let Ok(mut automation) = state.automation.lock() {
            automation.pick_done = true;
        }
        return;
    };
    let operation_id = format!("pick:{token}:{action_id}:{champion_id}:{attempt}");
    match state
        .writes
        .execute(
            &operation_id,
            "PATCH",
            &format!("/lol-champ-select/v1/session/actions/{action_id}"),
            Some(json!({ "championId": champion_id, "type": "pick" })),
        )
        .await
    {
        Ok(_) => {
            if let Ok(mut automation) = state.automation.lock() {
                automation.pick_done = true;
            }
            log::info!("auto picked champion {champion_id}");
        }
        Err(error) => {
            if let Ok(mut automation) = state.automation.lock() {
                if terminal(&error) || attempt + 1 >= MAX_ATTEMPTS {
                    automation.pick_done = true;
                } else {
                    automation.pick_attempt = attempt + 1;
                }
            }
            log::warn!("auto pick failed (attempt {attempt}): {error}");
        }
    }
}

async fn auto_complete(
    state: &AppState,
    config: &FunctionConfig,
    token: &str,
    session: &Value,
    local_cell: u64,
    position: &str,
) {
    if !config.enable_auto_select_timeout_completed {
        return;
    }
    {
        let Ok(automation) = state.automation.lock() else {
            return;
        };
        if automation.complete_done || automation.complete_attempt >= MAX_ATTEMPTS {
            return;
        }
    }
    let attempt = {
        let Ok(automation) = state.automation.lock() else {
            return;
        };
        automation.complete_attempt
    };
    let now = now_ms();
    if !completion_ready(session, now) {
        return;
    }
    // The timer can be stale/frozen near the end of a phase. Re-read the
    // session before locking and re-check the action and candidate list.
    let Ok(fresh) = state
        .session
        .lcu_get_fresh("/lol-champ-select/v1/session")
        .await
    else {
        return;
    };
    if !completion_ready(&fresh, now) {
        return;
    }
    let Some((action_id, current_champion)) = find_in_progress_pick(&fresh, local_cell) else {
        return;
    };
    let unavailable = cant_select(&fresh, local_cell);
    let champion_id = if !unavailable.contains(&current_champion) {
        current_champion
    } else {
        let Some(candidate) = candidates(config, "pick", position)
            .into_iter()
            .find(|candidate| !unavailable.contains(candidate))
        else {
            return;
        };
        candidate
    };
    let operation_id = format!("complete:{token}:{action_id}:{champion_id}:{attempt}");
    match state
        .writes
        .execute(
            &operation_id,
            "PATCH",
            &format!("/lol-champ-select/v1/session/actions/{action_id}"),
            Some(json!({ "championId": champion_id, "type": "pick", "completed": true })),
        )
        .await
    {
        Ok(_) => {
            if let Ok(mut automation) = state.automation.lock() {
                automation.complete_done = true;
            }
            log::info!("auto completed champion selection");
        }
        Err(error) => {
            if let Ok(mut automation) = state.automation.lock() {
                if terminal(&error) || attempt + 1 >= MAX_ATTEMPTS {
                    automation.complete_done = true;
                } else {
                    automation.complete_attempt = attempt + 1;
                }
            }
            log::warn!("auto complete failed (attempt {attempt}): {error}");
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn completion_ready(session: &Value, now_ms: i64) -> bool {
    let Some(timer) = session.get("timer") else {
        return false;
    };
    if timer.get("isInfinite").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    let Some(left) = timer.get("adjustedTimeLeftInPhase").and_then(Value::as_i64) else {
        return false;
    };
    let remaining = timer
        .get("internalNowInEpochMs")
        .and_then(Value::as_i64)
        .map(|internal| internal.saturating_add(left).saturating_sub(now_ms))
        .unwrap_or(left);
    remaining <= LOCK_LEAD_MS
}

fn cant_select(session: &Value, local_cell: u64) -> Vec<u64> {
    let mut unavailable = session
        .get("bans")
        .map(|bans| {
            ["myTeamBans", "theirTeamBans"]
                .iter()
                .flat_map(|key| {
                    bans.get(key)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_u64)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for action in flattened_actions(session) {
        if action.get("type").and_then(Value::as_str) != Some("pick")
            || action.get("actorCellId").and_then(Value::as_u64) == Some(local_cell)
            || action.get("completed").and_then(Value::as_bool) != Some(true)
        {
            continue;
        }
        if let Some(champion_id) = action.get("championId").and_then(Value::as_u64) {
            if champion_id > 0 {
                unavailable.push(champion_id);
            }
        }
    }
    unavailable
}

fn trade_enabled(config: &FunctionConfig, collection: &str) -> bool {
    match collection {
        "trades" => config.auto_accept_champ_trade,
        "pickOrderSwaps" => config.auto_accept_ceil_swap,
        _ => false,
    }
}

async fn auto_trades(state: &AppState, config: &FunctionConfig, session: &Value, token: &str) {
    for (collection, accept, clear) in [
        ("trades", "trades", "ongoing-trade"),
        ("pickOrderSwaps", "swaps", "ongoing-swap"),
    ] {
        if !trade_enabled(config, collection) {
            continue;
        }
        let Some(items) = session.get(collection).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            if item.get("state").and_then(Value::as_str) != Some("RECEIVED") {
                continue;
            }
            let Some(id) = item.get("id").and_then(Value::as_u64) else {
                continue;
            };
            let operation_id = format!("{accept}:{token}:{id}");
            match state
                .writes
                .execute(
                    &operation_id,
                    "POST",
                    &format!("/lol-champ-select/v1/session/{accept}/{id}/accept"),
                    None,
                )
                .await
            {
                Ok(_) => {
                    log::info!("auto accepted {accept} {id}");
                    let _ = state
                        .writes
                        .execute(
                            &format!("{operation_id}:clear"),
                            "POST",
                            &format!("/lol-champ-select/v1/session/{clear}/{id}"),
                            None,
                        )
                        .await;
                }
                Err(error) => log::warn!("auto accept {accept} {id} failed: {error}"),
            }
        }
    }
}

fn flattened_actions(session: &Value) -> Vec<&Value> {
    session
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| group.as_array().into_iter().flatten())
        .collect()
}

fn find_action(session: &Value, cell: u64, kind: &str, in_progress: bool) -> Option<u64> {
    flattened_actions(session)
        .into_iter()
        .find(|action| {
            action.get("actorCellId").and_then(Value::as_u64) == Some(cell)
                && action.get("type").and_then(Value::as_str) == Some(kind)
                && action.get("completed").and_then(Value::as_bool) != Some(true)
                && (!in_progress
                    || action.get("isInProgress").and_then(Value::as_bool) == Some(true))
        })
        .and_then(|action| action.get("id").and_then(Value::as_u64))
}

fn find_in_progress_pick(session: &Value, cell: u64) -> Option<(u64, u64)> {
    flattened_actions(session)
        .into_iter()
        .find(|action| {
            action.get("actorCellId").and_then(Value::as_u64) == Some(cell)
                && action.get("type").and_then(Value::as_str) == Some("pick")
                && action.get("isInProgress").and_then(Value::as_bool) == Some(true)
                && action.get("completed").and_then(Value::as_bool) != Some(true)
        })
        .and_then(|action| {
            let id = action.get("id").and_then(Value::as_u64)?;
            let champion_id = action.get("championId").and_then(Value::as_u64)?;
            (champion_id > 0).then_some((id, champion_id))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> FunctionConfig {
        serde_json::from_value(json!({
            "autoBanChampion": [10],
            "autoBanChampionTop": [1],
            "autoSelectChampion": [20],
            "autoSelectChampionTop": [2],
            "autoSetSummonerSpell": [4, 14],
            "autoSetSummonerSpellTop": [4, 12]
        }))
        .unwrap()
    }

    fn session() -> Value {
        json!({
            "localPlayerCellId": 0,
            "myTeam": [{"cellId": 0, "assignedPosition": "top"}],
            "actions": [[
                {"id": 7, "actorCellId": 0, "type": "ban", "isInProgress": true, "completed": false},
                {"id": 8, "actorCellId": 0, "type": "pick", "isInProgress": true, "completed": false, "championId": 2}
            ]]
        })
    }

    #[test]
    fn candidates_put_position_first_then_general() {
        let config = config();
        assert_eq!(candidates(&config, "ban", "top"), vec![1, 10]);
        assert_eq!(candidates(&config, "pick", "top"), vec![2, 20]);
        assert_eq!(candidates(&config, "pick", "unknown"), Vec::<u64>::new());
    }

    #[test]
    fn finds_owned_actions() {
        let session = session();
        assert_eq!(find_action(&session, 0, "ban", true), Some(7));
        assert_eq!(find_action(&session, 0, "pick", true), Some(8));
        assert_eq!(find_in_progress_pick(&session, 0), Some((8, 2)));
        assert!(find_action(&session, 5, "ban", true).is_none());
    }

    #[test]
    fn completion_waits_for_the_last_four_seconds() {
        let now = 1_000_000;
        let timer = |left: i64| {
            json!({
                "timer": {
                    "adjustedTimeLeftInPhase": left,
                    "internalNowInEpochMs": now,
                    "isInfinite": false
                }
            })
        };
        assert!(!completion_ready(&timer(10_000), now));
        assert!(completion_ready(&timer(3_999), now));
        assert!(!completion_ready(&json!({}), now));
        assert!(!completion_ready(
            &json!({
                "timer": {
                    "adjustedTimeLeftInPhase": 1_000,
                    "internalNowInEpochMs": now,
                    "isInfinite": true
                }
            }),
            now
        ));
    }

    #[test]
    fn unavailable_champions_include_bans_and_completed_enemy_picks() {
        let session = json!({
            "localPlayerCellId": 0,
            "bans": {"myTeamBans": [10], "theirTeamBans": [11]},
            "actions": [[
                {"actorCellId": 0, "type": "pick", "completed": true, "championId": 12},
                {"actorCellId": 1, "type": "pick", "completed": true, "championId": 13},
                {"actorCellId": 2, "type": "pick", "completed": false, "championId": 14}
            ]]
        });
        let unavailable = cant_select(&session, 0);
        assert_eq!(unavailable, vec![10, 11, 13]);
    }

    #[test]
    fn trade_switches_are_respected() {
        let mut config = FunctionConfig::default();
        assert!(!trade_enabled(&config, "trades"));
        assert!(!trade_enabled(&config, "pickOrderSwaps"));
        config.auto_accept_champ_trade = true;
        config.auto_accept_ceil_swap = true;
        assert!(trade_enabled(&config, "trades"));
        assert!(trade_enabled(&config, "pickOrderSwaps"));
    }

    #[test]
    fn outcome_unknown_is_terminal() {
        assert!(terminal(&CoreError::OutcomeUnknown("unknown".into())));
        assert!(terminal(&CoreError::OperationConflict("conflict".into())));
    }
}
