use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const CHAMPION_ALIAS_URL: &str =
    "https://game.gtimg.cn/images/lol/act/img/js/heroList/hero_list.js";
pub const ARAM_URL: &str = "http://www.jddld.com/index.php";

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn push_alias(aliases: &mut String, alias: &str) {
    let alias = alias.trim();
    if !alias.is_empty() && !aliases.split(',').any(|item| item == alias) {
        if !aliases.is_empty() {
            aliases.push(',');
        }
        aliases.push_str(alias);
    }
}

pub fn parse_champion_aliases(raw: &Value) -> Result<Value, String> {
    let heroes = raw
        .get("hero")
        .and_then(Value::as_array)
        .ok_or_else(|| "hero list is missing".to_string())?;
    let version = raw
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| "alias version is missing".to_string())?;
    let mut champions = Map::new();
    for hero in heroes {
        let Some(id) = hero
            .get("heroId")
            .and_then(Value::as_u64)
            .or_else(|| text(hero, "heroId").parse().ok())
        else {
            continue;
        };
        let mut aliases = text(hero, "keywords").to_lowercase();
        push_alias(&mut aliases, &text(hero, "alias").to_lowercase());
        champions.insert(id.to_string(), Value::String(aliases));
    }
    for (id, alias) in [
        ("901", "小火龙"),
        ("950", "狗"),
        ("902", "丁真"),
        ("897", "黑龙"),
    ] {
        if let Some(value) = champions.get(id).and_then(Value::as_str) {
            let mut aliases = value.to_string();
            push_alias(&mut aliases, alias);
            champions.insert(id.to_string(), Value::String(aliases));
        }
    }
    if champions.is_empty() {
        return Err("hero list is empty".to_string());
    }
    Ok(json!({ "version": version, "champions": champions }))
}

pub fn search_champion_aliases(data: &Value, query: &str) -> Result<Vec<u64>, String> {
    let champions = data
        .get("champions")
        .and_then(Value::as_object)
        .ok_or_else(|| "champion aliases are missing".to_string())?;
    let query = query.to_lowercase();
    let mut matches = Vec::new();
    for (id, aliases) in champions {
        if query.is_empty()
            || aliases
                .as_str()
                .is_some_and(|aliases| aliases.to_lowercase().contains(&query))
        {
            if let Ok(id) = id.parse() {
                matches.push(id);
            }
        }
    }
    matches.sort_unstable();
    Ok(matches)
}

pub fn parse_aram_buffs(raw: &Value) -> Result<Value, String> {
    if raw.get("code").and_then(Value::as_i64) != Some(1) {
        return Err("ARAM source rejected the request".to_string());
    }
    let data = raw
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "ARAM data is missing".to_string())?;
    let version = data
        .get("banben")
        .and_then(Value::as_str)
        .ok_or_else(|| "ARAM version is missing".to_string())?;
    let mut champions = BTreeMap::new();
    for item in data.values() {
        let Some(id) = item
            .get("heroid")
            .and_then(Value::as_u64)
            .or_else(|| text(item, "heroid").parse().ok())
        else {
            continue;
        };
        let required = [
            "catname",
            "zcsh",
            "sdsh",
            "zlxl",
            "hdxn",
            "jnjs",
            "renxing",
            "description",
        ];
        if required
            .iter()
            .any(|key| !item.get(*key).is_some_and(|v| !v.is_null()))
        {
            continue;
        }
        champions.insert(id.to_string(), item.clone());
    }
    if champions.is_empty() {
        return Err("ARAM data is empty".to_string());
    }
    Ok(json!({ "version": version, "champions": champions }))
}

pub fn aram_info(data: &Value, champion_id: u64) -> Option<Value> {
    Some(data.get("champions")?.get(champion_id.to_string())?.clone())
}

pub fn validate_cached(data: &Value, champions_key: &str) -> Result<(), String> {
    if !data
        .get("version")
        .and_then(Value::as_str)
        .is_some_and(|v| !v.is_empty())
        || !data.get(champions_key).and_then(Value::as_object).is_some()
    {
        return Err("invalid community cache".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_preserve_contract_and_custom_names() {
        let raw = json!({
            "version": "16.24",
            "hero": [
                {"heroId": "1", "keywords": "安妮,Annie", "alias": "黑暗之女"},
                {"heroId": 901, "keywords": "Smolder", "alias": "斯莫德"}
            ]
        });
        let data = parse_champion_aliases(&raw).unwrap();
        assert_eq!(data["version"], "16.24");
        assert_eq!(data["champions"]["1"], "安妮,annie,黑暗之女");
        assert!(data["champions"]["901"]
            .as_str()
            .unwrap()
            .contains("小火龙"));
        assert_eq!(search_champion_aliases(&data, "ANN").unwrap(), vec![1]);
        assert_eq!(search_champion_aliases(&data, "").unwrap(), vec![1, 901]);
    }

    #[test]
    fn aram_parser_preserves_required_fields() {
        let item = json!({
            "heroid": "75",
            "catname": "沙漠皇帝",
            "zcsh": "10%", "sdsh": "10%", "zlxl": "1", "hdxn": "2",
            "jnjs": "3", "renxing": "4", "description": "test"
        });
        let data = parse_aram_buffs(&json!({
            "code": 1, "data": {"banben": "16.24", "first": item}
        }))
        .unwrap();
        assert_eq!(data["version"], "16.24");
        assert_eq!(aram_info(&data, 75).unwrap()["catname"], "沙漠皇帝");
        assert!(aram_info(&data, 1).is_none());
    }

    #[test]
    fn rejects_invalid_community_data() {
        assert!(parse_champion_aliases(&json!({})).is_err());
        assert!(parse_aram_buffs(&json!({"code": 503})).is_err());
        assert!(validate_cached(&json!({"version": "1"}), "champions").is_err());
    }
}
