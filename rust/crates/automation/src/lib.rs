use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

pub mod write_guard;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationState {
    Disarmed,
    Armed,
    Waiting,
    Writing,
    Confirming,
    Completed,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct WriteKey {
    pub epoch: u64,
    pub session_id: String,
    pub action_id: u64,
    pub action_type: String,
    pub config_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Pick,
    Ban,
    Swap,
    Trade,
    Spell,
    Complete,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlannedAction {
    pub dry_run: bool,
    pub kind: ActionKind,
    pub action_id: u64,
    pub champion_id: u64,
    pub delay_ms: u64,
    pub reason: String,
    pub key: WriteKey,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ChampionSelection {
    pub is_summoner_spell_setted: bool,
    pub is_champion_showed: bool,
    pub is_champion_banned: bool,
    pub is_champion_picked: bool,
    pub is_champion_picked_completed: bool,
    pub is_skin_picked: bool,
    pub opgg_show_champion_id: Option<u64>,
    pub queue_id: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LedgerEntry {
    pub key: WriteKey,
    pub state: AutomationState,
}

#[derive(Clone, Debug, Serialize)]
pub struct AutomationSnapshot {
    pub state: ChampionSelection,
    pub ledger: Vec<LedgerEntry>,
    pub config_revision: u64,
}

pub struct DryRunAutomation {
    epoch: u64,
    session_id: String,
    config_revision: u64,
    config: Value,
    selection: ChampionSelection,
    ledger: HashMap<WriteKey, AutomationState>,
}

impl DryRunAutomation {
    pub fn new(epoch: u64, session_id: impl Into<String>, config: Value) -> Self {
        Self {
            epoch,
            session_id: session_id.into(),
            config_revision: 0,
            config,
            selection: ChampionSelection::default(),
            ledger: HashMap::new(),
        }
    }

    pub fn reset_session(&mut self, session_id: impl Into<String>) {
        self.session_id = session_id.into();
        self.selection = ChampionSelection::default();
        self.ledger.clear();
    }

    pub fn update_config(&mut self, config: Value) {
        self.config = config;
        self.config_revision = self.config_revision.wrapping_add(1);
    }

    pub fn set_queue_id(&mut self, queue_id: Option<u64>) {
        self.selection.queue_id = queue_id;
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn config_revision(&self) -> u64 {
        self.config_revision
    }

    pub fn snapshot(&self) -> AutomationSnapshot {
        AutomationSnapshot {
            state: self.selection.clone(),
            ledger: self
                .ledger
                .iter()
                .map(|(key, state)| LedgerEntry {
                    key: key.clone(),
                    state: *state,
                })
                .collect(),
            config_revision: self.config_revision,
        }
    }

    pub fn confirm(&mut self, key: &WriteKey, outcome: AutomationState) {
        if matches!(
            outcome,
            AutomationState::Completed | AutomationState::Failed | AutomationState::OutcomeUnknown
        ) {
            self.ledger.insert(key.clone(), outcome);
        }
    }

    pub fn handle_event(&mut self, data: &Value) -> Vec<PlannedAction> {
        let mut plans = Vec::new();
        let phase = data
            .get("timer")
            .and_then(|timer| timer.get("phase"))
            .and_then(Value::as_str)
            .unwrap_or_default();

        if phase == "PLANNING" {
            if let Some(action) = self.plan_spells(data) {
                plans.push(action);
            }
            if let Some(action) = self.plan_show(data) {
                plans.push(action);
            }
        } else if phase == "BAN_PICK" {
            if let Some(action) = self.plan_swap(data) {
                plans.push(action);
                return plans;
            }
            if let Some(action) = self.plan_spells(data) {
                plans.push(action);
            }
            if let Some(action) = self.plan_ban(data) {
                plans.push(action);
                return plans;
            }
            if let Some(action) = self.plan_pick(data) {
                plans.push(action);
                return plans;
            }
            if let Some(action) = self.plan_complete(data) {
                plans.push(action);
            }
        } else if phase == "FINALIZATION" {
            if let Some(action) = self.plan_trade(data) {
                plans.push(action);
            }
        }

        plans
    }

    fn cfg_bool(&self, key: &str) -> bool {
        self.config
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    fn cfg_u64(&self, key: &str) -> u64 {
        self.config.get(key).and_then(Value::as_u64).unwrap_or(0)
    }

    fn cfg_list(&self, key: &str) -> Vec<u64> {
        self.config
            .get(key)
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default()
    }

    fn local_player<'a>(&self, data: &'a Value) -> Option<&'a Value> {
        let cell_id = data.get("localPlayerCellId")?.as_u64()?;
        data.get("myTeam")?
            .as_array()?
            .iter()
            .find(|player| player.get("cellId").and_then(Value::as_u64) == Some(cell_id))
    }

    fn position(data: &Value) -> String {
        data.get("assignedPosition")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    fn candidates(&self, position_key: &str, global_key: &str) -> Vec<u64> {
        let mut candidates = self.cfg_list(position_key);
        candidates.extend(self.cfg_list(global_key));
        candidates
    }

    fn bans(data: &Value) -> Vec<u64> {
        let mut bans = Vec::new();
        if let Some(team_bans) = data
            .get("bans")
            .and_then(|bans| bans.get("myTeamBans"))
            .and_then(Value::as_array)
        {
            bans.extend(team_bans.iter().filter_map(Value::as_u64));
        }
        if let Some(team_bans) = data
            .get("bans")
            .and_then(|bans| bans.get("theirTeamBans"))
            .and_then(Value::as_array)
        {
            bans.extend(team_bans.iter().filter_map(Value::as_u64));
        }
        bans
    }

    fn actions(data: &Value) -> Vec<&Value> {
        data.get("actions")
            .and_then(Value::as_array)
            .map(|groups| {
                groups
                    .iter()
                    .filter_map(Value::as_array)
                    .flatten()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn plan(
        &mut self,
        kind: ActionKind,
        action_id: u64,
        champion_id: u64,
        delay_ms: u64,
        reason: impl Into<String>,
    ) -> Option<PlannedAction> {
        let key = WriteKey {
            epoch: self.epoch,
            session_id: self.session_id.clone(),
            action_id,
            action_type: format!("{kind:?}").to_ascii_lowercase(),
            config_revision: self.config_revision,
        };
        if self.ledger.contains_key(&key) {
            return None;
        }
        self.ledger.insert(key.clone(), AutomationState::Waiting);
        Some(PlannedAction {
            dry_run: true,
            kind,
            action_id,
            champion_id,
            delay_ms,
            reason: reason.into(),
            key,
        })
    }

    fn plan_swap(&mut self, data: &Value) -> Option<PlannedAction> {
        if !self.cfg_bool("autoAcceptCeilSwap") {
            return None;
        }
        let swap = data
            .get("pickOrderSwaps")?
            .as_array()?
            .iter()
            .find(|swap| swap.get("state").and_then(Value::as_str) == Some("RECEIVED"))?;
        let id = swap.get("id")?.as_u64()?;
        self.selection.is_champion_picked_completed = false;
        self.plan(ActionKind::Swap, id, 0, 500, "received pick order swap")
    }

    fn plan_trade(&mut self, data: &Value) -> Option<PlannedAction> {
        if !self.cfg_bool("autoAcceptChampTrade") {
            return None;
        }
        let trade = data
            .get("trades")?
            .as_array()?
            .iter()
            .find(|trade| trade.get("state").and_then(Value::as_str) == Some("RECEIVED"))?;
        let id = trade.get("id")?.as_u64()?;
        self.plan(ActionKind::Trade, id, 0, 500, "received champion trade")
    }

    fn plan_spells(&mut self, data: &Value) -> Option<PlannedAction> {
        if self.selection.is_summoner_spell_setted {
            return None;
        }
        self.selection.is_summoner_spell_setted = true;
        if !self.cfg_bool("enableAutoSetSpells") {
            return None;
        }
        let player = self.local_player(data)?;
        let key = match Self::position(player).as_str() {
            "top" => "autoSetSummonerSpellTop",
            "jungle" => "autoSetSummonerSpellJug",
            "middle" => "autoSetSummonerSpellMid",
            "bottom" => "autoSetSummonerSpellBot",
            "utility" => "autoSetSummonerSpellSup",
            _ => "autoSetSummonerSpell",
        };
        let mut spells = self.cfg_list(key);
        if spells.contains(&54) {
            spells = self.cfg_list("autoSetSummonerSpell");
        }
        if spells.len() < 2 || spells.contains(&54) {
            return None;
        }
        self.plan(
            ActionKind::Spell,
            0,
            spells[0] * 1000 + spells[1],
            0,
            "set summoner spells",
        )
    }

    fn plan_show(&mut self, data: &Value) -> Option<PlannedAction> {
        if self.selection.is_champion_showed || !self.cfg_bool("enableAutoSelectChampion") {
            return None;
        }
        let player = self.local_player(data)?;
        if player
            .get("championId")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            != 0
            || player
                .get("championPickIntent")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                != 0
        {
            self.selection.is_champion_showed = true;
            return None;
        }
        let position_key = match Self::position(player).as_str() {
            "top" => "autoSelectChampionTop",
            "jungle" => "autoSelectChampionJug",
            "middle" => "autoSelectChampionMid",
            "bottom" => "autoSelectChampionBot",
            "utility" => "autoSelectChampionSup",
            _ => "autoSelectChampion",
        };
        let candidates = self.candidates(position_key, "autoSelectChampion");
        let Some(champion_id) = candidates.first().copied() else {
            self.selection.is_champion_showed = true;
            return None;
        };
        let action = Self::actions(data)
            .into_iter()
            .rev()
            .find(|action| {
                action.get("actorCellId").and_then(Value::as_u64)
                    == data.get("localPlayerCellId").and_then(Value::as_u64)
                    && action.get("type").and_then(Value::as_str) == Some("pick")
            })?
            .get("id")?
            .as_u64()?;
        self.selection.is_champion_showed = true;
        self.plan(
            ActionKind::Pick,
            action,
            champion_id,
            0,
            "show champion intent",
        )
    }

    fn plan_pick(&mut self, data: &Value) -> Option<PlannedAction> {
        if self.selection.is_champion_picked || !self.cfg_bool("enableAutoSelectChampion") {
            return None;
        }
        let player = self.local_player(data)?;
        if player
            .get("championId")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            != 0
            || player
                .get("championPickIntent")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                != 0
        {
            self.selection.is_champion_picked = true;
            return None;
        }
        let position_key = match Self::position(player).as_str() {
            "top" => "autoSelectChampionTop",
            "jungle" => "autoSelectChampionJug",
            "middle" => "autoSelectChampionMid",
            "bottom" => "autoSelectChampionBot",
            "utility" => "autoSelectChampionSup",
            _ => "autoSelectChampion",
        };
        let bans = Self::bans(data);
        let mut candidates = self.candidates(position_key, "autoSelectChampion");
        candidates.retain(|champion| !bans.contains(champion));
        let Some(champion_id) = candidates.first().copied() else {
            self.selection.is_champion_picked = true;
            return None;
        };
        let action = Self::actions(data)
            .into_iter()
            .rev()
            .find(|action| {
                action.get("actorCellId").and_then(Value::as_u64)
                    == data.get("localPlayerCellId").and_then(Value::as_u64)
                    && action.get("type").and_then(Value::as_str) == Some("pick")
            })?
            .get("id")?
            .as_u64()?;
        self.selection.is_champion_picked = true;
        self.plan(ActionKind::Pick, action, champion_id, 0, "auto pick")
    }

    fn plan_ban(&mut self, data: &Value) -> Option<PlannedAction> {
        if self.selection.is_champion_banned || !self.cfg_bool("enableAutoBanChampion") {
            return None;
        }
        let player = self.local_player(data)?;
        let position_key = match Self::position(player).as_str() {
            "top" => "autoBanChampionTop",
            "jungle" => "autoBanChampionJug",
            "middle" => "autoBanChampionMid",
            "bottom" => "autoBanChampionBot",
            "utility" => "autoBanChampionSup",
            _ => "autoBanChampion",
        };
        let action = Self::actions(data).into_iter().find(|action| {
            action.get("actorCellId").and_then(Value::as_u64)
                == data.get("localPlayerCellId").and_then(Value::as_u64)
                && action.get("type").and_then(Value::as_str) == Some("ban")
                && action.get("isInProgress").and_then(Value::as_bool) == Some(true)
        })?;
        let action_id = action.get("id")?.as_u64()?;
        let mut candidates = self.candidates(position_key, "autoBanChampion");
        let bans = Self::bans(data);
        candidates.retain(|champion| !bans.contains(champion));
        if self.cfg_bool("pretentBan") {
            if let Some(team) = data.get("myTeam").and_then(Value::as_array) {
                let intents: Vec<u64> = team
                    .iter()
                    .filter_map(|player| player.get("championPickIntent"))
                    .filter_map(Value::as_u64)
                    .collect();
                candidates.retain(|champion| !intents.contains(champion));
            }
        }
        let champion_id = candidates.first().copied()?;
        self.selection.is_champion_banned = true;
        self.plan(
            ActionKind::Ban,
            action_id,
            champion_id,
            self.cfg_u64("autoBanDelay") * 1000,
            "auto ban",
        )
    }

    fn plan_complete(&mut self, data: &Value) -> Option<PlannedAction> {
        if self.selection.is_champion_picked_completed
            || !self.cfg_bool("enableAutoSelectTimeoutCompleted")
        {
            return None;
        }
        let action = Self::actions(data).into_iter().rev().find(|action| {
            action.get("actorCellId").and_then(Value::as_u64)
                == data.get("localPlayerCellId").and_then(Value::as_u64)
                && action.get("type").and_then(Value::as_str) == Some("pick")
                && action.get("isInProgress").and_then(Value::as_bool) == Some(true)
        })?;
        if action.get("completed").and_then(Value::as_bool) == Some(true) {
            self.selection.is_champion_picked_completed = true;
            return None;
        }
        let action_id = action.get("id")?.as_u64()?;
        let champion_id = action.get("championId").and_then(Value::as_u64)?;
        if champion_id == 0 {
            return None;
        }
        let time_left = data
            .get("timer")
            .and_then(|timer| timer.get("adjustedTimeLeftInPhase"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.selection.is_champion_picked_completed = true;
        self.plan(
            ActionKind::Complete,
            action_id,
            champion_id,
            time_left.saturating_sub(4000),
            "complete before timeout",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event() -> Value {
        json!({
            "localPlayerCellId": 0,
            "timer": { "phase": "BAN_PICK", "adjustedTimeLeftInPhase": 12000 },
            "myTeam": [{ "cellId": 0, "assignedPosition": "top", "championId": 0, "championPickIntent": 0 }],
            "bans": { "myTeamBans": [2], "theirTeamBans": [] },
            "actions": [[{ "id": 10, "actorCellId": 0, "type": "pick", "isInProgress": true, "completed": false, "championId": 1 }]],
            "pickOrderSwaps": [],
            "trades": []
        })
    }

    #[test]
    fn plans_pick_without_writing() {
        let mut automation = DryRunAutomation::new(
            1,
            "session",
            json!({ "enableAutoSelectChampion": true, "autoSelectChampionTop": [1, 2] }),
        );
        let plans = automation.handle_event(&event());
        assert_eq!(plans.len(), 1);
        assert!(plans[0].dry_run);
        assert_eq!(plans[0].kind, ActionKind::Pick);
        assert_eq!(plans[0].champion_id, 1);
        assert!(automation.handle_event(&event()).is_empty());
    }

    #[test]
    fn filters_banned_candidates() {
        let mut automation = DryRunAutomation::new(
            1,
            "session",
            json!({ "enableAutoSelectChampion": true, "autoSelectChampionTop": [2, 3] }),
        );
        let plans = automation.handle_event(&event());
        assert_eq!(plans[0].champion_id, 3);
    }

    #[test]
    fn schedules_timeout_completion() {
        let mut automation = DryRunAutomation::new(
            1,
            "session",
            json!({ "enableAutoSelectTimeoutCompleted": true }),
        );
        let plans = automation.handle_event(&event());
        assert_eq!(plans[0].kind, ActionKind::Complete);
        assert_eq!(plans[0].delay_ms, 8000);
    }
}
