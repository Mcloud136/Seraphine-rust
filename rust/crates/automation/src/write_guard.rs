use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteGuardError {
    MissingSession,
    InvalidPhase,
    MissingLocalPlayer,
    ActionNotFound,
    WrongActor,
    ActionNotActive,
    ActionCompleted,
    TypeMismatch,
    InvalidChampion,
    ChampionUnavailable,
    TradeNotFound,
    TradeNotReceived,
    InvalidSelection,
    ConfirmationMismatch,
}

impl WriteGuardError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingSession => "champ-select session is missing",
            Self::InvalidPhase => "champ-select phase is not writable",
            Self::MissingLocalPlayer => "local player is missing",
            Self::ActionNotFound => "champ-select action is missing",
            Self::WrongActor => "champ-select action belongs to another player",
            Self::ActionNotActive => "champ-select action is not in progress",
            Self::ActionCompleted => "champ-select action is already completed",
            Self::TypeMismatch => "champ-select action type does not match",
            Self::InvalidChampion => "champion id is invalid",
            Self::ChampionUnavailable => "champion is banned or already picked",
            Self::TradeNotFound => "trade or swap is missing",
            Self::TradeNotReceived => "trade or swap is not pending",
            Self::InvalidSelection => "local selection is missing",
            Self::ConfirmationMismatch => "write confirmation did not match requested state",
        }
    }
}

fn number(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn phase(session: &Value) -> &str {
    session
        .get("timer")
        .and_then(|timer| timer.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn actions(session: &Value) -> impl Iterator<Item = &Value> {
    session
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
}

fn champion_unavailable(session: &Value, champion_id: u64) -> bool {
    let banned = ["myTeamBans", "theirTeamBans"].iter().any(|key| {
        session
            .get("bans")
            .and_then(|bans| bans.get(*key))
            .and_then(Value::as_array)
            .is_some_and(|bans| bans.iter().any(|id| id.as_u64() == Some(champion_id)))
    });
    if banned {
        return true;
    }
    ["myTeam", "theirTeam"].iter().any(|key| {
        session
            .get(*key)
            .and_then(Value::as_array)
            .is_some_and(|team| {
                team.iter().any(|player| {
                    player.get("championId").and_then(Value::as_u64) == Some(champion_id)
                })
            })
    })
}

fn validate_action(session: &Value, action_id: u64, data: &Value) -> Result<(), WriteGuardError> {
    if phase(session) != "BAN_PICK" {
        return Err(WriteGuardError::InvalidPhase);
    }
    let local_cell = session
        .get("localPlayerCellId")
        .and_then(Value::as_u64)
        .ok_or(WriteGuardError::MissingLocalPlayer)?;
    let action = actions(session)
        .find(|action| number(action, "id") == Some(action_id))
        .ok_or(WriteGuardError::ActionNotFound)?;
    if number(action, "actorCellId") != Some(local_cell) {
        return Err(WriteGuardError::WrongActor);
    }
    if action.get("isInProgress").and_then(Value::as_bool) != Some(true) {
        return Err(WriteGuardError::ActionNotActive);
    }
    if action.get("completed").and_then(Value::as_bool) == Some(true) {
        return Err(WriteGuardError::ActionCompleted);
    }
    let action_type = action
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let requested_type = data.get("type").and_then(Value::as_str).unwrap_or_default();
    if action_type != requested_type {
        return Err(WriteGuardError::TypeMismatch);
    }
    let champion_id = number(data, "championId").ok_or(WriteGuardError::InvalidChampion)?;
    if champion_id == 0 || champion_unavailable(session, champion_id) {
        return Err(WriteGuardError::ChampionUnavailable);
    }
    if data.get("completed").and_then(Value::as_bool) == Some(true)
        && number(action, "championId") != Some(champion_id)
    {
        return Err(WriteGuardError::TypeMismatch);
    }
    Ok(())
}

fn validate_trade(session: &Value, collection: &str, trade_id: u64) -> Result<(), WriteGuardError> {
    if !matches!(phase(session), "BAN_PICK" | "FINALIZATION") {
        return Err(WriteGuardError::InvalidPhase);
    }
    let trade = session
        .get(collection)
        .and_then(Value::as_array)
        .and_then(|trades| {
            trades
                .iter()
                .find(|trade| number(trade, "id") == Some(trade_id))
        })
        .ok_or(WriteGuardError::TradeNotFound)?;
    if trade.get("state").and_then(Value::as_str) != Some("RECEIVED") {
        return Err(WriteGuardError::TradeNotReceived);
    }
    Ok(())
}

fn validate_selection(session: &Value) -> Result<(), WriteGuardError> {
    if !matches!(phase(session), "BAN_PICK" | "FINALIZATION") {
        return Err(WriteGuardError::InvalidPhase);
    }
    let local_cell = session
        .get("localPlayerCellId")
        .and_then(Value::as_u64)
        .ok_or(WriteGuardError::MissingLocalPlayer)?;
    session
        .get("myTeam")
        .and_then(Value::as_array)
        .and_then(|team| {
            team.iter()
                .find(|player| number(player, "cellId") == Some(local_cell))
        })
        .ok_or(WriteGuardError::InvalidSelection)?;
    Ok(())
}

pub fn validate_write(
    session: &Value,
    path: &str,
    data: Option<&Value>,
) -> Result<(), WriteGuardError> {
    if session.is_null() {
        return Err(WriteGuardError::MissingSession);
    }
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if let Some(index) = segments.iter().position(|segment| *segment == "actions") {
        let action_id = segments
            .get(index + 1)
            .and_then(|value| value.parse().ok())
            .ok_or(WriteGuardError::ActionNotFound)?;
        return validate_action(
            session,
            action_id,
            data.ok_or(WriteGuardError::TypeMismatch)?,
        );
    }
    if let Some(index) = segments.iter().position(|segment| *segment == "trades") {
        let trade_id = segments
            .get(index + 1)
            .and_then(|value| value.parse().ok())
            .ok_or(WriteGuardError::TradeNotFound)?;
        if segments.get(index + 2) == Some(&"accept") {
            return validate_trade(session, "trades", trade_id);
        }
    }
    if let Some(index) = segments.iter().position(|segment| *segment == "swaps") {
        let swap_id = segments
            .get(index + 1)
            .and_then(|value| value.parse().ok())
            .ok_or(WriteGuardError::TradeNotFound)?;
        if segments.get(index + 2) == Some(&"accept") {
            return validate_trade(session, "pickOrderSwaps", swap_id);
        }
    }
    if segments.last() == Some(&"my-selection") {
        return validate_selection(session);
    }
    Ok(())
}

pub fn confirm_write(
    session: &Value,
    path: &str,
    data: Option<&Value>,
) -> Result<(), WriteGuardError> {
    if session.is_null() {
        return Err(WriteGuardError::MissingSession);
    }
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if let Some(index) = segments.iter().position(|segment| *segment == "actions") {
        let action_id = segments
            .get(index + 1)
            .and_then(|value| value.parse().ok())
            .ok_or(WriteGuardError::ActionNotFound)?;
        let data = data.ok_or(WriteGuardError::TypeMismatch)?;
        let action = actions(session)
            .find(|action| number(action, "id") == Some(action_id))
            .ok_or(WriteGuardError::ActionNotFound)?;
        if number(action, "championId") != number(data, "championId") {
            return Err(WriteGuardError::ConfirmationMismatch);
        }
        if data.get("completed").and_then(Value::as_bool) == Some(true)
            && action.get("completed").and_then(Value::as_bool) != Some(true)
        {
            return Err(WriteGuardError::ConfirmationMismatch);
        }
        return Ok(());
    }
    for (collection, path_segment) in [("trades", "trades"), ("pickOrderSwaps", "swaps")] {
        if let Some(index) = segments.iter().position(|segment| *segment == path_segment) {
            if segments.get(index + 2) != Some(&"accept") {
                return Ok(());
            }
            let id = segments
                .get(index + 1)
                .and_then(|value| value.parse().ok())
                .ok_or(WriteGuardError::TradeNotFound)?;
            let trade = session
                .get(collection)
                .and_then(Value::as_array)
                .and_then(|trades| trades.iter().find(|trade| number(trade, "id") == Some(id)))
                .ok_or(WriteGuardError::TradeNotFound)?;
            if trade.get("state").and_then(Value::as_str) == Some("RECEIVED") {
                return Err(WriteGuardError::ConfirmationMismatch);
            }
            return Ok(());
        }
    }
    if segments.last() == Some(&"my-selection") {
        let local_cell = session
            .get("localPlayerCellId")
            .and_then(Value::as_u64)
            .ok_or(WriteGuardError::MissingLocalPlayer)?;
        let player = session
            .get("myTeam")
            .and_then(Value::as_array)
            .and_then(|team| {
                team.iter()
                    .find(|player| number(player, "cellId") == Some(local_cell))
            })
            .ok_or(WriteGuardError::InvalidSelection)?;
        if let Some(data) = data {
            for key in ["spell1Id", "spell2Id", "selectedSkinId", "wardSkinId"] {
                if let Some(expected) = number(data, key) {
                    if number(player, key) != Some(expected) {
                        return Err(WriteGuardError::ConfirmationMismatch);
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> Value {
        json!({
            "localPlayerCellId": 0,
            "timer": {"phase": "BAN_PICK"},
            "myTeam": [{"cellId": 0, "championId": 0, "championPickIntent": 0}],
            "theirTeam": [{"cellId": 5, "championId": 0}],
            "bans": {"myTeamBans": [2], "theirTeamBans": []},
            "actions": [[{"id": 10, "actorCellId": 0, "type": "pick", "isInProgress": true, "completed": false, "championId": 1}]],
            "trades": [{"id": 20, "state": "RECEIVED"}],
            "pickOrderSwaps": [{"id": 21, "state": "RECEIVED"}]
        })
    }

    #[test]
    fn accepts_owned_active_pick() {
        assert!(validate_write(
            &session(),
            "/lol-champ-select/v1/session/actions/10",
            Some(&json!({"type": "pick", "championId": 1}))
        )
        .is_ok());
    }

    #[test]
    fn rejects_stale_or_foreign_actions() {
        let mut data = session();
        data["actions"][0][0]["actorCellId"] = json!(5);
        assert_eq!(
            validate_write(
                &data,
                "/lol-champ-select/v1/session/actions/10",
                Some(&json!({"type": "pick", "championId": 1}))
            ),
            Err(WriteGuardError::WrongActor)
        );
        let mut data = session();
        data["actions"][0][0]["completed"] = json!(true);
        assert_eq!(
            validate_write(
                &data,
                "/lol-champ-select/v1/session/actions/10",
                Some(&json!({"type": "pick", "championId": 1}))
            ),
            Err(WriteGuardError::ActionCompleted)
        );
    }

    #[test]
    fn rejects_banned_or_picked_champions() {
        assert_eq!(
            validate_write(
                &session(),
                "/lol-champ-select/v1/session/actions/10",
                Some(&json!({"type": "pick", "championId": 2}))
            ),
            Err(WriteGuardError::ChampionUnavailable)
        );
        let mut data = session();
        data["theirTeam"][0]["championId"] = json!(3);
        assert_eq!(
            validate_write(
                &data,
                "/lol-champ-select/v1/session/actions/10",
                Some(&json!({"type": "pick", "championId": 3}))
            ),
            Err(WriteGuardError::ChampionUnavailable)
        );
    }

    #[test]
    fn confirms_written_action_state() {
        let mut data = session();
        data["actions"][0][0]["championId"] = json!(3);
        data["actions"][0][0]["completed"] = json!(true);
        assert!(confirm_write(
            &data,
            "/lol-champ-select/v1/session/actions/10",
            Some(&json!({"type": "pick", "championId": 3, "completed": true}))
        )
        .is_ok());
        data["actions"][0][0]["completed"] = json!(false);
        assert_eq!(
            confirm_write(
                &data,
                "/lol-champ-select/v1/session/actions/10",
                Some(&json!({"type": "pick", "championId": 3, "completed": true}))
            ),
            Err(WriteGuardError::ConfirmationMismatch)
        );
    }

    #[test]
    fn validates_trade_and_swap_state() {
        assert!(validate_write(
            &session(),
            "/lol-champ-select/v1/session/trades/20/accept",
            None
        )
        .is_ok());
        assert!(validate_write(
            &session(),
            "/lol-champ-select/v1/session/swaps/21/accept",
            None
        )
        .is_ok());
        let mut data = session();
        data["trades"][0]["state"] = json!("ACCEPTED");
        assert_eq!(
            validate_write(&data, "/lol-champ-select/v1/session/trades/20/accept", None),
            Err(WriteGuardError::TradeNotReceived)
        );
    }
}
