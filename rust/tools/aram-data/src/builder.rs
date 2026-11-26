//! Champion entry construction and validation, matching the Python
//! generator's contracts (Chinese labels, fixed field order, neutral filter).

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value};

use crate::lua::LuaValue;

pub type Entry = Vec<(String, String)>;

const RATIO_FIELDS: [(&str, &str); 4] = [
    ("mana_regen", "法力回复"),
    ("energy_regen", "能量回复"),
    ("attack_speed", "攻击速度"),
    ("movement_speed", "移动速度"),
];

#[derive(Default)]
pub struct Stats {
    pub dmg_dealt: f64,
    pub dmg_taken: f64,
    pub healing: f64,
    pub shielding: f64,
    pub ability_haste: f64,
    pub tenacity: f64,
    pub mana_regen: f64,
    pub energy_regen: f64,
    pub attack_speed: f64,
    pub movement_speed: f64,
}

impl Stats {
    fn from_lua(table: &LuaValue) -> Self {
        let number =
            |key: &str, default: f64| table.get(key).and_then(LuaValue::as_num).unwrap_or(default);
        Self {
            dmg_dealt: number("dmg_dealt", 1.0),
            dmg_taken: number("dmg_taken", 1.0),
            healing: number("healing", 1.0),
            shielding: number("shielding", 1.0),
            ability_haste: number("ability_haste", 0.0),
            tenacity: number("tenacity", 1.0),
            mana_regen: number("mana_regen", 1.0),
            energy_regen: number("energy_regen", 1.0),
            attack_speed: number("attack_speed", 1.0),
            movement_speed: number("movement_speed", 1.0),
        }
    }

    fn from_json(table: &Map<String, Value>) -> Self {
        let number =
            |key: &str, default: f64| table.get(key).and_then(Value::as_f64).unwrap_or(default);
        Self {
            dmg_dealt: number("dmg_dealt", 1.0),
            dmg_taken: number("dmg_taken", 1.0),
            healing: number("healing", 1.0),
            shielding: number("shielding", 1.0),
            ability_haste: number("ability_haste", 0.0),
            tenacity: number("tenacity", 1.0),
            mana_regen: number("mana_regen", 1.0),
            energy_regen: number("energy_regen", 1.0),
            attack_speed: number("attack_speed", 1.0),
            movement_speed: number("movement_speed", 1.0),
        }
    }
}

fn percent(ratio: f64) -> i64 {
    (ratio * 100.0).round_ties_even() as i64
}

fn bonus(ratio: f64) -> i64 {
    ((ratio - 1.0) * 100.0).round_ties_even() as i64
}

fn rounded(ratio: f64) -> f64 {
    (ratio * 10_000.0).round_ties_even() / 10_000.0
}

fn build_entry(stats: &Stats, fallback: &str) -> Entry {
    let mut details = Vec::new();
    if rounded(stats.dmg_dealt) != 1.0 {
        details.push(format!("造成伤害 {}%", percent(stats.dmg_dealt)));
    }
    if rounded(stats.dmg_taken) != 1.0 {
        details.push(format!("承受伤害 {}%", percent(stats.dmg_taken)));
    }
    if rounded(stats.healing) != 1.0 {
        details.push(format!("治疗效果 {}%", percent(stats.healing)));
    }
    if rounded(stats.shielding) != 1.0 {
        details.push(format!("护盾效果 {}%", percent(stats.shielding)));
    }
    let haste = stats.ability_haste.round_ties_even() as i64;
    if haste != 0 {
        details.push(format!("技能急速 {haste:+}"));
    }
    if rounded(stats.tenacity) != 1.0 {
        details.push(format!("韧性 {:+}%", bonus(stats.tenacity)));
    }
    for (key, label) in RATIO_FIELDS {
        let ratio = match key {
            "mana_regen" => stats.mana_regen,
            "energy_regen" => stats.energy_regen,
            "attack_speed" => stats.attack_speed,
            _ => stats.movement_speed,
        };
        if rounded(ratio) != 1.0 {
            details.push(format!("{label} {:+}%", bonus(ratio)));
        }
    }
    vec![
        ("catname".into(), fallback.to_string()),
        ("zcsh".into(), percent(stats.dmg_dealt).to_string()),
        ("sdsh".into(), percent(stats.dmg_taken).to_string()),
        ("zlxl".into(), percent(stats.healing).to_string()),
        ("hdxn".into(), percent(stats.shielding).to_string()),
        ("jnjs".into(), haste.to_string()),
        ("renxing".into(), bonus(stats.tenacity).to_string()),
        ("description".into(), details.join("，")),
    ]
}

fn is_neutral(entry: &Entry) -> bool {
    let value = |key: &str| {
        entry
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .unwrap_or_default()
    };
    value("zcsh") == "100"
        && value("sdsh") == "100"
        && value("zlxl") == "100"
        && value("hdxn") == "100"
        && value("jnjs") == "0"
        && value("renxing") == "0"
}

fn title(lua: &LuaValue, key: &str) -> Option<String> {
    lua.get(key)
        .and_then(LuaValue::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub fn build_from_module(
    module: &LuaValue,
    names: &HashMap<String, String>,
) -> BTreeMap<u64, Entry> {
    let mut champions = BTreeMap::new();
    for value in module.values() {
        if !matches!(value, LuaValue::Table(_)) {
            continue;
        }
        let Some(id) = value.get("id").and_then(LuaValue::as_num) else {
            continue;
        };
        let id = id as u64;
        if id == 0 {
            continue;
        }
        let Some(stats) = value
            .get("stats")
            .filter(|stats| matches!(stats, LuaValue::Table(_)))
        else {
            continue;
        };
        let Some(aram) = stats
            .get("aram")
            .filter(|aram| matches!(aram, LuaValue::Table(_)))
        else {
            continue;
        };
        let fallback = names
            .get(&id.to_string())
            .cloned()
            .or_else(|| title(value, "title"))
            .or_else(|| title(value, "apiname"))
            .unwrap_or_else(|| id.to_string());
        let entry = build_entry(&Stats::from_lua(aram), &fallback);
        if is_neutral(&entry) {
            continue;
        }
        champions.insert(id, entry);
    }
    champions
}

pub fn build_from_balance(
    balance: &Value,
    names: &HashMap<String, String>,
) -> BTreeMap<u64, Entry> {
    let mut champions = BTreeMap::new();
    let Some(root) = balance.as_object() else {
        return champions;
    };
    for (key, value) in root {
        if !key.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let Some(value) = value.as_object() else {
            continue;
        };
        let Some(stats) = value
            .get("stats")
            .and_then(Value::as_object)
            .and_then(|stats| stats.get("aram"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let Ok(id) = key.parse::<u64>() else {
            continue;
        };
        let fallback = names
            .get(key)
            .cloned()
            .or_else(|| {
                value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| key.clone());
        let entry = build_entry(&Stats::from_json(stats), &fallback);
        if is_neutral(&entry) {
            continue;
        }
        champions.insert(id, entry);
    }
    champions
}

pub fn validate(version: &str, champions: &BTreeMap<u64, Entry>) -> Result<(), String> {
    if !valid_version(version) {
        return Err(format!("invalid data version: {version:?}"));
    }
    if champions.len() < 120 {
        return Err(format!("too few champions: {}", champions.len()));
    }
    let mut interesting = 0;
    for (id, entry) in champions {
        let field = |key: &str| {
            entry
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
                .unwrap_or_default()
        };
        if field("catname").is_empty() {
            return Err(format!("champion {id} has no catname"));
        }
        for key in ["zcsh", "sdsh", "zlxl", "hdxn", "jnjs", "renxing"] {
            if field(key).parse::<i64>().is_err() {
                return Err(format!("champion {id} field {key} is not an integer"));
            }
        }
        if (
            field("zcsh"),
            field("sdsh"),
            field("zlxl"),
            field("hdxn"),
            field("jnjs"),
            field("renxing"),
        ) != ("100", "100", "100", "100", "0", "0")
        {
            interesting += 1;
        }
    }
    if interesting == 0 {
        return Err("all champion modifiers are neutral".into());
    }
    Ok(())
}

pub fn valid_version(version: &str) -> bool {
    let parts = version.split('.').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        return false;
    }
    parts
        .iter()
        .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::parse_module;

    #[test]
    fn builds_entry_with_expected_fields() {
        let module = parse_module(
            "return { [\"Annie\"] = { id = 1, title = \"Annie\", stats = { aram = { \
             dmg_dealt = 0.95, dmg_taken = 1.05, shielding = 0.9 } } } }",
        )
        .unwrap();
        let champions = build_from_module(&module, &HashMap::new());
        let entry = champions.get(&1).unwrap();
        let field = |key: &str| {
            entry
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
                .unwrap_or_default()
        };
        assert_eq!(field("catname"), "Annie");
        assert_eq!(field("zcsh"), "95");
        assert_eq!(field("sdsh"), "105");
        assert_eq!(field("hdxn"), "90");
        assert!(field("description").contains("造成伤害 95%"));
    }

    #[test]
    fn rounds_like_python() {
        assert_eq!(percent(1.125), 112); // 112.5 ties to even
        assert_eq!(percent(1.135), 114); // 1.135*100 is slightly above 113.5
        assert_eq!(bonus(1.05), 5);
    }

    #[test]
    fn rejects_short_data() {
        assert!(validate("16.23.1", &BTreeMap::new()).is_err());
        assert!(valid_version("16.23"));
        assert!(!valid_version("16"));
        assert!(!valid_version("v16.23"));
    }
}
