use serde_json::{json, Value};

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn number(value: &Value, key: &str) -> Value {
    field(value, key).clone()
}

fn integer(value: &Value, key: &str) -> u64 {
    field(value, key).as_u64().unwrap_or(0)
}

fn text(value: &Value, key: &str) -> String {
    field(value, key).as_str().unwrap_or_default().to_string()
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}

fn champion_icon(id: u64) -> String {
    format!("@champion:{id}")
}

fn champion_name(id: u64) -> String {
    format!("@champion_name:{id}")
}

fn item_icon(id: u64) -> String {
    format!("@item:{id}")
}

fn spell_icon(id: u64) -> String {
    format!("@spell:{id}")
}

fn rune_icon(id: u64) -> String {
    format!("@rune:{id}")
}

fn augment_icon(id: u64) -> String {
    format!("@augment:{id}")
}

fn augment_name(id: u64) -> String {
    format!("@augment_name:{id}")
}

fn classic_rune_icon(url: &str, key: &str) -> String {
    format!("@classic_rune:{url}|{key}")
}

fn legacy_mastery_icon(url: &str, key: u64) -> String {
    format!("@legacy_mastery:{url}|{key}")
}

fn tier_item(item: &Value, position: Option<&str>, stats: &Value) -> Value {
    let id = integer(item, "id");
    let tier_data = field(stats, "tier_data");
    let counters = if let Some(position) = position {
        item.get("positions")
            .and_then(Value::as_array)
            .unwrap_or(&Vec::new())
            .iter()
            .find(|entry| field(entry, "name").as_str() == Some(position))
            .map(|entry| {
                array(field(entry, "counters"))
                    .iter()
                    .map(|counter| {
                        let counter_id = integer(counter, "champion_id");
                        json!({
                            "championId": counter_id,
                            "icon": champion_icon(counter_id),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    json!({
        "championId": id,
        "name": champion_name(id),
        "icon": champion_icon(id),
        "winRate": number(stats, "win_rate"),
        "pickRate": number(stats, "pick_rate"),
        "banRate": number(stats, "ban_rate"),
        "kda": number(stats, "kda"),
        "tier": field(tier_data, "tier").clone(),
        "rank": field(tier_data, "rank").clone(),
        "position": position,
        "counters": counters,
    })
}

pub fn parse_ranked_tier_list(raw: &Value) -> Value {
    let mut result: std::collections::BTreeMap<String, Vec<Value>> =
        ["TOP", "JUNGLE", "MID", "ADC", "SUPPORT"]
            .into_iter()
            .map(|position| (position.to_string(), Vec::new()))
            .collect();

    for item in array(field(raw, "data")) {
        for position in array(field(item, "positions")) {
            let name = text(position, "name");
            let stats = field(position, "stats");
            if let Some(entries) = result.get_mut(&name) {
                entries.push(tier_item(item, Some(&name), stats));
            }
        }
    }

    for entries in result.values_mut() {
        entries.sort_by(|left, right| {
            field(left, "rank")
                .as_f64()
                .unwrap_or(f64::MAX)
                .partial_cmp(&field(right, "rank").as_f64().unwrap_or(f64::MAX))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    json!(result)
}

pub fn parse_other_tier_list(raw: &Value) -> Value {
    let mut result = Vec::new();
    for item in array(field(raw, "data")) {
        let stats = field(item, "average_stats");
        if stats.is_null() || field(stats, "rank").is_null() {
            continue;
        }
        result.push(tier_item(item, None, stats));
    }
    result.sort_by(|left, right| {
        field(left, "rank")
            .as_f64()
            .unwrap_or(f64::MAX)
            .partial_cmp(&field(right, "rank").as_f64().unwrap_or(f64::MAX))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    json!(result)
}

fn with_stats(items: &[Value]) -> Vec<&Value> {
    items
        .iter()
        .filter(|item| !item.get("play").is_none())
        .collect()
}

fn item_entry(item: &Value) -> Value {
    json!({
        "icons": array(field(item, "ids"))
            .iter()
            .map(|id| item_icon(id.as_u64().unwrap_or(0)))
            .collect::<Vec<_>>(),
        "play": number(item, "play"),
        "win": number(item, "win"),
        "pickRate": number(item, "pick_rate"),
    })
}

fn arena_item_entry(item: &Value) -> Value {
    let play = field(item, "play").as_f64().unwrap_or(0.0);
    json!({
        "icons": array(field(item, "ids"))
            .iter()
            .map(|id| item_icon(id.as_u64().unwrap_or(0)))
            .collect::<Vec<_>>(),
        "play": number(item, "play"),
        "win": number(item, "win"),
        "pickRate": number(item, "pick_rate"),
        "averatePlace": ratio(field(item, "total_place").as_f64().unwrap_or(0.0), play),
        "firstRate": ratio(field(item, "first_place").as_f64().unwrap_or(0.0), play),
    })
}

fn skills(data: &Value, require_stats: bool) -> Value {
    let masteries = with_stats(array(field(data, "skill_masteries")));
    let orders = with_stats(array(field(data, "skills")));
    if require_stats && (masteries.is_empty() || orders.is_empty()) {
        return Value::Null;
    }
    if masteries.is_empty() || orders.is_empty() {
        return Value::Null;
    }
    let order = orders[0];
    json!({
        "masteries": field(masteries[0], "ids").clone(),
        "order": field(order, "order").clone(),
        "play": number(order, "play"),
        "win": number(order, "win"),
        "pickRate": number(order, "pick_rate"),
    })
}

fn other_summary(data: &Value, position: &str) -> Value {
    let summary = field(data, "summary");
    let id = integer(summary, "id");
    let stats = if position != "none" {
        array(field(summary, "positions"))
            .iter()
            .find(|entry| field(entry, "name").as_str() == Some(position))
            .map(|entry| field(entry, "stats"))
            .unwrap_or(&Value::Null)
    } else {
        field(summary, "average_stats")
    };
    let stats = if stats.is_null() { &Value::Null } else { stats };
    let tier_data = field(stats, "tier_data");

    json!({
        "name": champion_name(id),
        "championId": id,
        "icon": champion_icon(id),
        "position": position,
        "winRate": number(stats, "win_rate"),
        "pickRate": number(stats, "pick_rate"),
        "banRate": number(stats, "ban_rate"),
        "kda": number(stats, "kda"),
        "tier": if position == "none" { number(stats, "tier") } else { field(tier_data, "tier").clone() },
        "rank": if position == "none" { number(stats, "rank") } else { field(tier_data, "rank").clone() },
    })
}

fn parse_counters(data: &Value) -> Value {
    let mut strong = Vec::new();
    let mut weak = Vec::new();
    for counter in array(field(data, "counters")) {
        let id = integer(counter, "champion_id");
        let play = field(counter, "play").as_f64().unwrap_or(0.0);
        let win_rate = ratio(field(counter, "win").as_f64().unwrap_or(0.0), play);
        let entry = json!({
            "championId": id,
            "name": champion_name(id),
            "icon": champion_icon(id),
            "play": number(counter, "play"),
            "win": number(counter, "win"),
            "winRate": win_rate,
        });
        if win_rate >= 0.5 {
            strong.push(entry);
        } else {
            weak.push(entry);
        }
    }
    strong.sort_by(|left, right| {
        field(right, "winRate")
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&field(left, "winRate").as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    weak.sort_by(|left, right| {
        field(left, "winRate")
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&field(right, "winRate").as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    json!({ "strongAgainst": strong, "weakAgainst": weak })
}

fn parse_perks(data: &Value) -> Value {
    let mut result = Vec::new();
    for perk in array(field(data, "runes")) {
        let primary_id = integer(perk, "primary_page_id");
        let secondary_id = integer(perk, "secondary_page_id");
        let mut ids = array(field(perk, "primary_rune_ids")).to_vec();
        ids.extend(array(field(perk, "secondary_rune_ids")).to_vec());
        ids.extend(array(field(perk, "stat_mod_ids")).to_vec());
        result.push(json!({
            "primaryId": primary_id,
            "primaryIcon": rune_icon(primary_id),
            "secondaryId": secondary_id,
            "secondaryIcon": rune_icon(secondary_id),
            "perks": ids,
            "icons": ids.iter().map(|id| rune_icon(id.as_u64().unwrap_or(0))).collect::<Vec<_>>(),
            "play": number(perk, "play"),
            "win": number(perk, "win"),
            "pickRate": number(perk, "pick_rate"),
        }));
    }
    json!(result)
}

fn parse_classic_runes(data: &Value) -> Value {
    let Some(page) = array(field(data, "classic_runes")).first() else {
        return json!([]);
    };
    let runes = array(field(page, "runes"))
        .iter()
        .map(|rune| {
            let id = rune.get("id").cloned().unwrap_or_else(|| json!(0));
            let key = field(rune, "source_token")
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| id.to_string());
            json!({
                "icon": classic_rune_icon(text(rune, "icon_url").as_str(), key.as_str()),
                "name": text(rune, "name"),
                "tooltip": text(rune, "tooltip"),
                "count": integer(rune, "count"),
            })
        })
        .collect::<Vec<_>>();
    json!([{
        "runes": runes,
        "play": number(page, "play"),
        "win": number(page, "win"),
        "pickRate": number(page, "pick_rate"),
    }])
}

fn parse_legacy_masteries(data: &Value) -> Value {
    let Some(page) = array(field(data, "legacy_masteries")).first() else {
        return json!([]);
    };
    let masteries = array(field(page, "masteries"))
        .iter()
        .map(|mastery| {
            let id = integer(mastery, "id");
            json!({
                "id": id,
                "icon": legacy_mastery_icon(text(mastery, "icon_url").as_str(), id),
                "name": text(mastery, "name"),
                "description": text(mastery, "description"),
                "tree": text(mastery, "tree"),
                "rank": integer(mastery, "rank"),
                "maxRank": integer(mastery, "max_rank"),
            })
        })
        .collect::<Vec<_>>();
    json!(masteries)
}

fn parse_other_augments(data: &Value) -> Value {
    let groups = array(field(data, "augment_group"));
    let has_data = groups
        .iter()
        .any(|group| !array(field(group, "augments")).is_empty());
    if !has_data {
        return Value::Null;
    }
    let mut result = Vec::new();
    for group in groups {
        let mut entries = Vec::new();
        for augment in array(field(group, "augments")) {
            let id = integer(augment, "id");
            entries.push(json!({
                "id": id,
                "icon": augment_icon(id),
                "name": augment_name(id),
                "win": number(augment, "win"),
                "play": number(augment, "play"),
                "pickRate": number(augment, "pick_rate"),
            }));
        }
        result.push(entries);
    }
    json!(result)
}

pub fn parse_other_champion_build(raw: &Value, position: &str) -> Value {
    let data = field(raw, "data");
    let boots = with_stats(array(field(data, "boots")));
    let start_items = with_stats(array(field(data, "starter_items")));
    let core_items = with_stats(array(field(data, "core_items")));
    let last_items = with_stats(array(field(data, "last_items")));

    json!({
        "summary": other_summary(data, position),
        "summonerSpells": with_stats(array(field(data, "summoner_spells")))
            .iter()
            .map(|spell| json!({
                "ids": field(spell, "ids").clone(),
                "icons": array(field(spell, "ids"))
                    .iter()
                    .map(|id| spell_icon(id.as_u64().unwrap_or(0)))
                    .collect::<Vec<_>>(),
                "win": number(spell, "win"),
                "play": number(spell, "play"),
                "pickRate": number(spell, "pick_rate"),
            }))
            .collect::<Vec<_>>(),
        "championSkills": skills(data, true),
        "items": {
            "boots": boots.iter().take(3).map(|item| item_entry(item)).collect::<Vec<_>>(),
            "startItems": start_items.iter().take(3).map(|item| item_entry(item)).collect::<Vec<_>>(),
            "coreItems": core_items.iter().take(5).map(|item| item_entry(item)).collect::<Vec<_>>(),
            "lastItems": last_items.iter().take(16).map(|item| {
                let id = array(field(item, "ids")).first().and_then(Value::as_u64).unwrap_or(0);
                item_icon(id)
            }).collect::<Vec<_>>(),
        },
        "counters": parse_counters(data),
        "perks": parse_perks(data),
        "classicRunes": parse_classic_runes(data),
        "legacyMasteries": parse_legacy_masteries(data),
        "augments": parse_other_augments(data),
    })
}

pub fn parse_arena_champion_build(raw: &Value) -> Value {
    let data = field(raw, "data");
    let summary = field(data, "summary");
    let id = integer(summary, "id");
    let stats = field(summary, "average_stats");
    let play = field(stats, "play").as_f64().unwrap_or(0.0);

    let group = |key: &str, limit: usize| {
        array(field(data, key))
            .iter()
            .take(limit)
            .map(arena_item_entry)
            .collect::<Vec<_>>()
    };

    let augments = array(field(data, "augment_group"))
        .iter()
        .map(|group| {
            array(field(group, "augments"))
                .iter()
                .map(|augment| {
                    let id = integer(augment, "id");
                    json!({
                        "id": id,
                        "icon": augment_icon(id),
                        "name": augment_name(id),
                        "win": number(augment, "win"),
                        "play": number(augment, "play"),
                        "totalPlace": number(augment, "total_place"),
                        "firstPlace": number(augment, "first_place"),
                        "pickRate": number(augment, "pick_rate"),
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let synergies = array(field(data, "synergies"))
        .iter()
        .map(|synergy| {
            let id = integer(synergy, "champion_id");
            json!({
                "championId": id,
                "icon": champion_icon(id),
                "name": champion_name(id),
                "win": number(synergy, "win"),
                "play": number(synergy, "play"),
                "totalPlace": number(synergy, "total_place"),
                "firstPlace": number(synergy, "first_place"),
                "pickRate": number(synergy, "pick_rate"),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "summary": {
            "name": champion_name(id),
            "icon": champion_icon(id),
            "championId": id,
            "play": number(stats, "play"),
            "winRate": ratio(field(stats, "win").as_f64().unwrap_or(0.0), play),
            "firstRate": ratio(field(stats, "first_place").as_f64().unwrap_or(0.0), play),
            "averagePlace": ratio(field(stats, "total_place").as_f64().unwrap_or(0.0), play),
            "pickRate": number(stats, "pick_rate"),
            "banRate": number(stats, "ban_rate"),
            "tier": number(stats, "tier"),
            "position": "none",
        },
        "championSkills": skills(data, false),
        "items": {
            "boots": group("boots", 3),
            "startItems": group("starter_items", 3),
            "prismItems": group("prism_items", 3),
            "coreItems": group("core_items", 5),
            "lastItems": array(field(data, "last_items")).iter().take(16).map(|item| {
                let id = array(field(item, "ids")).first().and_then(Value::as_u64).unwrap_or(0);
                item_icon(id)
            }).collect::<Vec<_>>(),
        },
        "augments": augments,
        "synergies": synergies,
    })
}

pub async fn resolve_icons(value: &mut Value, resolver: &mut dyn IconResolver) {
    match value {
        Value::String(text) if text.starts_with('@') => {
            let spec = std::mem::take(text);
            *text = resolver.resolve(spec).await;
        }
        Value::Array(items) => {
            for item in items {
                Box::pin(resolve_icons(item, resolver)).await;
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                Box::pin(resolve_icons(item, resolver)).await;
            }
        }
        _ => {}
    }
}

#[async_trait::async_trait]
pub trait IconResolver {
    async fn resolve(&mut self, spec: String) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_other_build_contract() {
        let raw = json!({
            "data": {
                "summary": {
                    "id": 1,
                    "average_stats": { "win_rate": 0.51, "tier": "A", "rank": 1 }
                },
                "summoner_spells": [{ "ids": [4, 12], "win": 10, "play": 20, "pick_rate": 0.5 }],
                "skill_masteries": [{ "ids": ["Q", "W"], "play": 1 }],
                "skills": [{ "order": ["Q", "W"], "win": 1, "play": 1, "pick_rate": 1.0 }],
                "boots": [],
                "starter_items": [],
                "core_items": [],
                "last_items": [],
                "counters": [],
                "runes": [],
                "classic_runes": [],
                "legacy_masteries": [],
                "augment_group": []
            }
        });
        let parsed = parse_other_champion_build(&raw, "none");
        assert_eq!(parsed["summary"]["championId"], 1);
        assert_eq!(parsed["summary"]["icon"], "@champion:1");
        assert_eq!(parsed["summonerSpells"][0]["icons"][0], "@spell:4");
        assert!(parsed["augments"].is_null());
    }

    #[test]
    fn arena_average_place_keeps_existing_spelling() {
        let raw = json!({
            "data": {
                "summary": { "id": 1, "average_stats": { "play": 2, "win": 1, "first_place": 1, "total_place": 5 } },
                "skill_masteries": [{ "ids": [], "play": 1 }],
                "skills": [{ "order": [], "win": 1, "play": 1, "pick_rate": 1.0 }],
                "boots": [{ "ids": [1], "play": 2, "win": 1, "first_place": 1, "total_place": 3, "pick_rate": 0.5 }],
                "starter_items": [],
                "prism_items": [],
                "core_items": [],
                "last_items": [],
                "augment_group": [],
                "synergies": []
            }
        });
        let parsed = parse_arena_champion_build(&raw);
        assert_eq!(parsed["summary"]["averagePlace"], 2.5);
        assert_eq!(parsed["items"]["boots"][0]["averatePlace"], 1.5);
    }
}
