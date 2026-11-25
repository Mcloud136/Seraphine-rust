import json
import os

def _language_name():
    from app.common.config import cfg

    return cfg.language.value.name


def _int_keys(data):
    result = {}
    for key, value in (data or {}).items():
        try:
            key = int(key)
        except (TypeError, ValueError):
            pass
        result[key] = value
    return result


class CatalogManager:
    def __init__(self, snapshot=None):
        snapshot = snapshot or {}
        self.items = _int_keys(snapshot.get("items"))
        self.spells = _int_keys(snapshot.get("summonerSpells"))
        self.runes = _int_keys(snapshot.get("runes"))
        self.champs = _int_keys(snapshot.get("champs"))
        self.champAliases = snapshot.get("champAliases", {})
        self.champions = snapshot.get("championSkins", {})
        self.skinAugments = _int_keys(snapshot.get("skinAugments"))
        self.queues = _int_keys(snapshot.get("queues"))
        self.perks = snapshot.get("perks", {})
        self.perkStyles = None
        self.cherryAugments = _int_keys(snapshot.get("cherryAugments"))

    @classmethod
    def from_snapshot(cls, snapshot):
        return cls(snapshot)

    def getItemIconPath(self, iconId):
        if iconId != 0:
            try:
                return self.items[iconId]["iconPath"]
            except KeyError:
                pass
        return "/lol-game-data/assets/ASSETS/Items/Icons2D/gp_ui_placeholder.png"

    def getSummonerSpellIconPath(self, spellId):
        if spellId != 0:
            return self.spells[spellId]["iconPath"]
        return "/lol-game-data/assets/data/spells/icons2d/summoner_empty.png"

    def getRuneIconPath(self, runeId):
        try:
            return self.runes[runeId]["icon"]
        except KeyError:
            for item in self.perks.get("styles", []):
                if item["id"] == runeId:
                    return item["iconPath"]
        return None

    def getRuneName(self, runeId):
        return self.runes[runeId]["name"]

    def getRuneDesc(self, runeId):
        return self.runes[runeId]["desc"]

    def getSummonerProfileIconPath(self, iconId):
        return f"/lol-game-data/assets/v1/profile-icons/{iconId}.jpg"

    def getChampionIconPath(self, championId):
        return f"/lol-game-data/assets/v1/champion-icons/{championId}.png"

    def getMapNameById(self, mapId):
        maps = {
            -1: ("特殊地图", "Special map"),
            11: ("召唤师峡谷", "Summoner's Rift"),
            12: ("嚎哭深渊", "Howling Abyss"),
            21: ("极限闪击", "Nexus Blitz"),
            30: ("斗魂竞技场", "Arena"),
            453: ("经典召唤师峡谷", "League Classic"),
        }
        key = mapId if mapId in maps else -1
        language = _language_name()
        index = 0 if language == "CHINESE_SIMPLIFIED" else 1
        return maps[key][index]

    def getNameMapByQueueId(self, queueId):
        if queueId == 0:
            language = _language_name()
            return {
                "name": "Custom" if language == "ENGLISH" else
                "Personalizado" if language == "PORTUGUESE" else
                "自定义"
            }

        data = self.queues[queueId]
        mapId = data["mapId"]
        name = data["name"]
        path = "app/resource/i18n/gamemodes.json"
        if os.path.isfile(path):
            with open(path, encoding="utf-8") as file:
                translate = json.load(file)
            if name in translate:
                language = _language_name()
                translated = translate[name]
                if isinstance(translated, dict):
                    if language == "ENGLISH":
                        name = translated["en"]
                    elif language == "PORTUGUESE":
                        name = translated["pt"]
                elif language == "ENGLISH":
                    name = translated

        return {"map": self.getMapNameById(mapId), "name": name}

    def getGameModeByQueueId(self, queueId):
        data = self.queues.get(queueId)
        return data["gameMode"] if data else None

    def getChampionIdByAlias(self, alias):
        return self.champAliases.get(alias.lower(), -1)

    def getMapIconByMapId(self, mapId, win):
        result = "victory" if win else "defeat"
        if mapId == 11:
            mapName = "sr"
        elif mapId == 12:
            mapName = "ha"
        elif mapId == 30:
            mapName = "arena"
        else:
            mapName = "other"
        return f"app/resource/images/{mapName}-{result}.png"

    def getChampionList(self):
        return list(self.champions.keys())

    def getChampionIdList(self):
        return list(self.champs.keys())

    def getChampions(self):
        return self.champs

    def getSkinListByChampionName(self, championName):
        try:
            return list(self.champions[championName]["skins"].items())
        except KeyError:
            return []

    def getSkinIdByChampionAndSkinName(self, championName, skinName):
        return self.champions[championName]["skins"][skinName]["skinId"]

    def getChampionIdByName(self, championName):
        return self.champions[championName]["id"]

    def getChampionNameById(self, championId):
        return self.champs.get(championId)

    def getSkinAugments(self, skinId):
        return self.skinAugments.get(skinId)

    def getPerkStyles(self):
        return self.perkStyles

    def getAugmentsIconPath(self, augmentId):
        try:
            return self.cherryAugments[augmentId]["augmentSmallIconPath"]
        except KeyError:
            return "/lol-game-data/assets/ASSETS/Items/Icons2D/gp_ui_placeholder.png"

    def getAugmentsName(self, augmentId):
        return self.cherryAugments[augmentId]["nameTRA"]

    def getSummonerSpellList(self):
        return [1, 3, 4, 6, 7, 11, 12, 13, 14, 21, 32, 54]
