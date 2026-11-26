use serde_json::{json, Map, Value};

fn id_key(id: &Value) -> String {
    id.as_i64()
        .map(|value| value.to_string())
        .or_else(|| id.as_u64().map(|value| value.to_string()))
        .unwrap_or_else(|| id.to_string())
}

fn insert_map(target: &mut Map<String, Value>, id: &Value, value: Value) {
    target.insert(id_key(id), value);
}

#[derive(Default)]
pub struct CatalogBuilder {
    items: Map<String, Value>,
    spells: Map<String, Value>,
    runes: Map<String, Value>,
    queues: Map<String, Value>,
    champs: Map<String, Value>,
    champ_aliases: Map<String, Value>,
    champion_skins: Map<String, Value>,
    skin_augments: Map<String, Value>,
    perks: Value,
    cherry_augments: Map<String, Value>,
}

impl CatalogBuilder {
    pub fn add_items(&mut self, items: Value) {
        for item in items.as_array().into_iter().flatten() {
            if let Some(id) = item.get("id") {
                insert_map(
                    &mut self.items,
                    id,
                    json!({ "iconPath": item.get("iconPath").cloned().unwrap_or(Value::Null) }),
                );
            }
        }
    }

    pub fn add_spells(&mut self, spells: Value) {
        if let Some(spells) = spells.as_array() {
            for item in spells.iter().take(spells.len().saturating_sub(3)) {
                if let Some(id) = item.get("id") {
                    insert_map(
                        &mut self.spells,
                        id,
                        json!({
                            "name": item.get("name").cloned().unwrap_or(Value::Null),
                            "iconPath": item.get("iconPath").cloned().unwrap_or(Value::Null),
                        }),
                    );
                }
            }
        }
    }

    pub fn add_runes(&mut self, runes: Value) {
        for item in runes.as_array().into_iter().flatten() {
            if let Some(id) = item.get("id") {
                insert_map(
                    &mut self.runes,
                    id,
                    json!({
                        "icon": item.get("iconPath").cloned().unwrap_or(Value::Null),
                        "name": item.get("name").cloned().unwrap_or(Value::Null),
                        "desc": item.get("longDesc").cloned().unwrap_or(Value::Null),
                    }),
                );
            }
        }
    }

    pub fn add_queues(&mut self, queues: Value) {
        for queue in queues.as_array().into_iter().flatten() {
            let Some(id) = queue.get("id") else {
                continue;
            };
            let name = queue
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .or_else(|| queue.get("description").and_then(Value::as_str))
                .unwrap_or_default();
            insert_map(
                &mut self.queues,
                id,
                json!({
                    "mapId": queue.get("mapId").cloned().unwrap_or(Value::Null),
                    "name": name,
                    "gameMode": queue.get("gameMode").cloned().unwrap_or(Value::Null),
                }),
            );
        }
    }

    pub fn add_champions(&mut self, champions: Value) {
        for champion in champions.as_array().into_iter().flatten() {
            let Some(id) = champion.get("id") else {
                continue;
            };
            let Some(name) = champion.get("name").and_then(Value::as_str) else {
                continue;
            };
            insert_map(&mut self.champs, id, json!(name));
            if let Some(alias) = champion.get("alias").and_then(Value::as_str) {
                self.champ_aliases
                    .insert(alias.to_ascii_lowercase(), id.clone());
            }
            self.champion_skins
                .insert(name.to_string(), json!({ "id": id, "skins": {} }));
        }
    }

    pub fn add_skins(&mut self, skins: Value) {
        let Some(skins) = skins.as_object() else {
            return;
        };
        for skin in skins.values() {
            let Some(id) = skin.get("id") else {
                continue;
            };
            let Some(champion_id) = id.as_i64().map(|value| value / 1000) else {
                continue;
            };
            let Some(champion_name) = self
                .champs
                .get(&champion_id.to_string())
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            if let Some(tiers) = skin
                .get("questSkinInfo")
                .and_then(|value| value.get("tiers"))
                .and_then(Value::as_array)
            {
                for tier in tiers {
                    self.insert_skin(champion_id, &champion_name, tier);
                }
            } else {
                self.insert_skin(champion_id, &champion_name, skin);
            }
        }
    }

    fn insert_skin(&mut self, champion_id: i64, champion_name: &str, skin: &Value) {
        let Some(id) = skin.get("id") else {
            return;
        };
        let Some(name) = skin.get("name").and_then(Value::as_str) else {
            return;
        };
        let entry = self
            .champion_skins
            .entry(champion_name.to_string())
            .or_insert_with(|| json!({ "id": champion_id, "skins": {} }));
        if let Some(skin_map) = entry.get_mut("skins").and_then(Value::as_object_mut) {
            skin_map.insert(
                name.to_string(),
                json!({
                    "skinId": id,
                    "splashPath": skin.get("splashPath").cloned().unwrap_or(Value::Null),
                    "uncenteredSplashPath": skin.get("uncenteredSplashPath").cloned().unwrap_or(Value::Null),
                }),
            );
        }
        if let Some(content_id) = skin
            .get("skinAugments")
            .and_then(|value| value.get("augments"))
            .and_then(Value::as_array)
            .and_then(|augments| augments.first())
            .and_then(|augment| augment.get("contentId"))
        {
            insert_map(&mut self.skin_augments, id, content_id.clone());
        }
    }

    pub fn add_perks(&mut self, perks: Value) {
        self.perks = perks;
    }

    pub fn add_augments(&mut self, augments: Value) {
        for augment in augments.as_array().into_iter().flatten() {
            if let Some(id) = augment.get("id") {
                insert_map(&mut self.cherry_augments, id, augment.clone());
            }
        }
    }

    pub fn build(self) -> Value {
        json!({
            "items": self.items,
            "summonerSpells": self.spells,
            "runes": self.runes,
            "champs": self.champs,
            "champAliases": self.champ_aliases,
            "championSkins": self.champion_skins,
            "skinAugments": self.skin_augments,
            "queues": self.queues,
            "perks": self.perks,
            "cherryAugments": self.cherry_augments,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_snapshot(
    items: Value,
    spells: Value,
    runes: Value,
    queues: Value,
    champions: Value,
    skins: Value,
    perks: Value,
    augments: Value,
) -> Value {
    let mut builder = CatalogBuilder::default();
    builder.add_items(items);
    builder.add_spells(spells);
    builder.add_runes(runes);
    builder.add_queues(queues);
    builder.add_champions(champions);
    builder.add_skins(skins);
    builder.add_perks(perks);
    builder.add_augments(augments);
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_compact_catalog_maps() {
        let snapshot = build_snapshot(
            json!([{ "id": 1, "iconPath": "/item.png" }]),
            json!([{ "id": 4, "iconPath": "/flash.png" }]),
            json!([{ "id": 5, "iconPath": "/rune.png", "name": "Rune", "longDesc": "Desc" }]),
            json!([{ "id": 420, "mapId": 11, "name": "", "description": "Ranked", "gameMode": "CLASSIC" }]),
            json!([{ "id": 1, "name": "Annie", "alias": "Annie" }]),
            json!({ "1000": { "id": 1000, "name": "Classic Annie" } }),
            json!({ "styles": [] }),
            json!([]),
        );
        assert_eq!(snapshot["items"]["1"]["iconPath"], "/item.png");
        assert_eq!(snapshot["champs"]["1"], "Annie");
        assert_eq!(snapshot["queues"]["420"]["name"], "Ranked");
    }

    #[test]
    fn quest_skin_tiers_are_flattened_with_augments() {
        let snapshot = build_snapshot(
            json!([]),
            json!([]),
            json!([]),
            json!([]),
            json!([{ "id": 1, "name": "Annie", "alias": "Annie" }]),
            json!({
                "1000": {
                    "id": 1000,
                    "name": "Quest Annie",
                    "questSkinInfo": {
                        "tiers": [{
                            "id": 1001,
                            "name": "Prestige Annie",
                            "splashPath": "/prestige.jpg",
                            "uncenteredSplashPath": "/prestige-uncentered.jpg",
                            "skinAugments": { "augments": [{ "contentId": "augment-1" }] }
                        }]
                    }
                }
            }),
            json!({ "styles": [] }),
            json!([]),
        );
        let skins = &snapshot["championSkins"]["Annie"]["skins"];
        assert_eq!(skins["Prestige Annie"]["skinId"], 1001);
        assert_eq!(skins["Prestige Annie"]["splashPath"], "/prestige.jpg");
        assert!(skins.get("Quest Annie").is_none());
        assert_eq!(snapshot["skinAugments"]["1001"], "augment-1");
    }
}
