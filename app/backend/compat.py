import asyncio
import subprocess
from collections import deque
from pathlib import Path
from urllib.parse import quote

from app.common.signals import signalBus
from app.lol.match_history import extract_match_history_games

from .cache_paths import ensure_cache_root
from .catalog import CatalogManager
from .sidecar_client import SidecarClient


class SidecarConnector:
    def __init__(self, app_version="1.3.0", forward_events=True):
        self.client = SidecarClient(app_version=app_version)
        self.forward_events = forward_events
        self.pid = 0
        self.port = None
        self.server = ""
        self.manager = None
        self.event_task = None
        self.callStack = deque(maxlen=10)

    @property
    def token(self):
        return "<redacted>"

    @property
    def lcuSess(self):
        return None

    async def start(self, pid):
        self.pid = pid
        await self.client.start()
        proxy = None
        try:
            from app.common.config import cfg

            if cfg.get(cfg.enableOpggProxy):
                proxy = f"http://{cfg.get(cfg.opggProxyAddr)}"
        except ImportError:
            pass
        connection = await self.client.lcu_connect(
            pid,
            proxy=proxy,
            cache_root=str(ensure_cache_root() / "game"),
        )
        self.port = connection.get("port")
        self.server = connection.get("server", "")
        self.client.epoch = connection.get("epoch", 0)
        await self.load_catalog()
        self.event_task = asyncio.create_task(
            self._forward_events() if self.forward_events else self._drain_events()
        )
        return connection

    async def autoStart(self):
        from app.common.util import getLolClientPid, getTasklistPath

        await self.start(getLolClientPid(getTasklistPath()))

    async def getLoginSummonerByPidAsync(self, pid):
        return await self.client.lcu_summoner_by_pid(pid)

    async def _drain_events(self):
        async for _ in self.client.events_stream():
            pass

    async def _forward_events(self):
        async for event in self.client.events_stream():
            topic = event.get("topic")
            data = event.get("data")
            if topic == "gameflow.changed":
                signalBus.gameStatusChanged.emit(data)
            elif topic == "champselect.changed":
                signalBus.champSelectChanged.emit(data)
            elif topic == "summoner.changed":
                signalBus.currentSummonerProfileChanged.emit(data)

    async def close(self):
        if self.event_task is not None:
            self.event_task.cancel()
            await asyncio.gather(self.event_task, return_exceptions=True)
            self.event_task = None
        await self.client.close(forget=True)
        self.pid = 0
        self.port = None
        self.server = ""

    async def health(self):
        return await self.client.health()

    async def diagnostics(self):
        return await self.client.diagnostics()

    async def process_list(self):
        return await self.client.process_list()

    async def getRuneIcon(self, runeId):
        if runeId == 0:
            return "app/resource/images/rune-0.png"
        return await self.client.asset_get(
            "rune icons",
            f"{runeId}.png",
            path=self.manager.getRuneIconPath(runeId),
            max_bytes=512 * 1024,
        )

    async def getProfileIcon(self, iconId):
        return await self.client.asset_get(
            "profile icons",
            f"{iconId}.jpg",
            path=self.manager.getSummonerProfileIconPath(iconId),
            max_bytes=512 * 1024,
        )

    async def getItemIcon(self, itemId):
        if itemId == 0:
            return "app/resource/images/item-0.png"
        return await self.client.asset_get(
            "item icons",
            f"{itemId}.png",
            path=self.manager.getItemIconPath(itemId),
            max_bytes=512 * 1024,
        )

    async def getAugmentIcon(self, augmentId):
        return await self.client.asset_get(
            "augment icons",
            f"{augmentId}.png",
            path=self.manager.getAugmentsIconPath(augmentId),
            max_bytes=512 * 1024,
        )

    async def getChampionSplashes(self, skinInfo, isCentered):
        skinId = skinInfo["skinId"]
        suffix = "centered" if isCentered else "uncentered"
        path = skinInfo["splashPath" if isCentered else "uncenteredSplashPath"]
        return await self.client.asset_get(
            "splashes",
            f"{skinId}_{suffix}.jpg",
            path=path,
            max_bytes=8 * 1024 * 1024,
        )

    async def getSummonerSpellIcon(self, spellId):
        return await self.client.asset_get(
            "summoner spell icons",
            f"{spellId}.png",
            path=self.manager.getSummonerSpellIconPath(spellId),
            max_bytes=512 * 1024,
        )

    async def getChampionIcon(self, championId):
        if championId in (-1, 0):
            return "app/resource/images/champion-0.png"
        return await self.client.asset_get(
            "champion icons",
            f"{championId}.png",
            path=self.manager.getChampionIconPath(championId),
            max_bytes=512 * 1024,
        )

    async def getCurrentSummoner(self):
        result = await self.client.lcu_get("/lol-summoner/v1/current-summoner")
        if "summonerId" not in result:
            raise ReferenceError()
        return result

    async def getCurrentSummonerProfile(self):
        return await self.client.lcu_get(
            "/lol-summoner/v1/current-summoner/summoner-profile"
        )

    async def getInstallFolder(self):
        return await self.client.lcu_get("/data-store/v1/install-dir")

    async def getSummonerByName(self, name):
        return await self.client.lcu_get(
            f"/lol-summoner/v1/summoners?name={quote(str(name))}"
        )

    async def getSummonerByPuuid(self, puuid):
        from app.lol.exceptions import SummonerNotFound

        result = await self.client.lcu_get(
            f"/lol-summoner/v2/summoners/puuid/{quote(str(puuid))}"
        )
        if result.get("errorCode") and result.get("httpStatus") == 400:
            raise SummonerNotFound()
        return result

    async def getSummonerById(self, summonerId):
        return await self.client.lcu_get(
            f"/lol-summoner/v1/summoners/{summonerId}"
        )

    async def getSummonerGamesByPuuid(
        self, puuid, begIndex=0, endIndex=4
    ):
        from app.lol.exceptions import SummonerGamesNotFound

        result = await self.client.lcu_get(
            f"/lol-match-history/v1/products/lol/{puuid}/matches"
            f"?begIndex={int(begIndex)}&endIndex={int(endIndex)}"
        )
        if "games" not in result:
            raise SummonerGamesNotFound()
        return extract_match_history_games(result)

    async def getSummonerGamesByPuuidSlowly(
        self, puuid, begIndex=0, endIndex=4
    ):
        return await self.getSummonerGamesByPuuid(puuid, begIndex, endIndex)

    async def getTftGamesByPuuid(self, puuid, count=20):
        from app.lol.exceptions import SummonerGamesNotFound

        result = await self.client.lcu_get(
            f"/lol-match-history/v1/products/tft/{puuid}/matches"
            f"?begin=0&count={int(count)}"
        )
        if "games" not in result:
            raise SummonerGamesNotFound()
        return extract_match_history_games(result)

    async def getGameDetailByGameId(self, gameId):
        return await self.client.lcu_get(
            f"/lol-match-history/v1/games/{gameId}"
        )

    async def getRankedStatsByPuuid(self, puuid):
        from app.lol.exceptions import SummonerRankInfoNotFound

        result = await self.client.lcu_get(
            f"/lol-ranked/v1/ranked-stats/{puuid}"
        )
        if "errorCode" in result:
            raise SummonerRankInfoNotFound()
        return result

    async def getGameflowSession(self):
        return await self.client.lcu_get("/lol-gameflow/v1/session")

    async def getChampSelectSession(self):
        return await self.client.lcu_get("/lol-champ-select/v1/session")

    async def getGameQueues(self):
        return await self.client.lcu_get("/lol-game-queues/v1/queues")

    async def getCurrentChampion(self):
        return await self.client.lcu_get(
            "/lol-champ-select/v1/current-champion"
        )

    async def getSkinCarousel(self):
        return await self.client.lcu_get(
            "/lol-champ-select/v1/skin-carousel-skins"
        )

    async def getFriends(self):
        return await self.client.lcu_get("/lol-chat/v1/friends")

    async def getGameStatus(self):
        return await self.client.lcu_get(
            "/lol-gameflow/v1/gameflow-phase"
        )

    async def getMapSide(self):
        result = await self.client.lcu_get(
            "/lol-champ-select/v1/pin-drop-notification"
        )
        return result.get("mapSide", "")

    async def getReadyCheckStatus(self):
        return await self.client.lcu_get(
            "/lol-matchmaking/v1/ready-check"
        )

    async def getCurrentRunePage(self):
        return await self.client.lcu_get("/lol-perks/v1/currentpage")

    async def getRuneInventory(self):
        return await self.client.lcu_get("/lol-perks/v1/inventory")

    async def getRunePages(self):
        return await self.client.lcu_get("/lol-perks/v1/pages")

    async def getConversation(self):
        return await self.client.lcu_get("/lol-chat/v1/conversations")

    async def getConversations(self):
        return await self.getConversation()

    async def getChatMe(self):
        return await self.client.lcu_get("/lol-chat/v1/me")

    async def getHelp(self):
        return await self.client.lcu_get("/help")

    async def getClientZoom(self):
        return await self.client.lcu_get("/riotclient/zoom-scale")

    async def getReplayMetadata(self, gameId):
        return await self.client.lcu_get(
            f"/lol-replays/v1/metadata/{gameId}"
        )

    async def getReplayPath(self):
        return await self.client.lcu_get("/lol-replays/v1/rofls/path")

    async def getSummonerGamesByPuuidViaSGP(self, puuid, begIdx, endIdx):
        return await self.client.sgp_get(
            f"/match-history-query/v1/products/lol/player/{puuid}/SUMMARY",
            {
                "startIndex": int(begIdx),
                "count": int(endIdx) - int(begIdx) + 1,
            },
        )

    async def getSummonerGamingInfoByPuuidViaSgp(self, puuid):
        return await self.client.sgp_get(
            f"/gsm/v1/ledge/spectator/region/{self.server.lower()}/puuid/{puuid}"
        )

    async def getRankedStatsByPuuidViaSGP(self, puuid):
        from app.lol.exceptions import SummonerRankInfoNotFound

        result = await self.client.sgp_get(
            f"/leagues-ledge/v2/rankedStats/puuid/{puuid}"
        )
        if "errorCode" in result:
            raise SummonerRankInfoNotFound()
        return result

    async def getSummonerByPuuidViaSGP(self, puuid):
        return await self.client.sgp_get(
            f"/summoner-ledge/v1/regions/{self.server.lower()}/summoners/puuid/{puuid}"
        )

    async def _write(self, method, path, data=None, operation_id=None):
        return await self.client.lcu_write(
            method, path, data=data, operation_id=operation_id
        )

    async def setProfileBackground(self, skinId):
        return await self._write(
            "POST",
            "/lol-summoner/v1/current-summoner/summoner-profile",
            {"key": "backgroundSkinId", "value": skinId},
        )

    async def setProfileBackgroundAugments(self, contentId):
        return await self._write(
            "POST",
            "/lol-summoner/v1/current-summoner/summoner-profile",
            {"key": "backgroundSkinAugments", "value": contentId},
        )

    async def setOnlineStatus(self, message):
        return await self._write("PUT", "/lol-chat/v1/me", {"statusMessage": message})

    async def setTierShowed(self, queue, tier, division):
        return await self._write(
            "PUT",
            "/lol-chat/v1/me",
            {
                "lol": {
                    "rankedLeagueQueue": queue,
                    "rankedLeagueTier": tier,
                    "rankedLeagueDivision": division,
                }
            },
        )

    async def reconnect(self):
        return await self._write("POST", "/lol-gameflow/v1/reconnect")

    async def removeTokens(self):
        reference = await self.getChatMe()
        return await self._write(
            "POST",
            "/lol-challenges/v1/update-player-preferences/",
            {
                "challengeIds": [],
                "bannerAccent": reference["lol"].get("bannerIdSelected"),
            },
        )

    async def setProfileIcon(self, iconId):
        return await self._write(
            "PUT",
            "/lol-summoner/v1/current-summoner/icon",
            {"profileIconId": iconId},
        )

    async def removePrestigeCrest(self):
        reference = await self.client.lcu_get(
            "/lol-regalia/v2/current-summoner/regalia"
        )
        return await self._write(
            "PUT",
            "/lol-regalia/v2/current-summoner/regalia",
            {
                "preferredCrestType": "prestige",
                "preferredBannerType": reference.get("preferredBannerType"),
                "selectedPrestigeCrest": 22,
            },
        )

    async def create5v5PracticeLobby(self, lobbyName, password):
        return await self._write(
            "POST",
            "/lol-lobby/v2/lobby",
            {
                "customGameLobby": {
                    "configuration": {
                        "gameMode": "PRACTICETOOL",
                        "gameMutator": "",
                        "gameServerRegion": "",
                        "mapId": 11,
                        "mutators": {"id": 1},
                        "spectatorPolicy": "AllAllowed",
                        "teamSize": 5,
                    },
                    "lobbyName": lobbyName,
                    "lobbyPassword": password,
                },
                "isCustom": True,
            },
        )

    async def setOnlineAvailability(self, availability):
        return await self._write(
            "PUT", "/lol-chat/v1/me", {"availability": availability}
        )

    async def acceptMatchMaking(self):
        return await self._write(
            "POST", "/lol-matchmaking/v1/ready-check/accept"
        )

    async def acceptTrade(self, id):
        await self._write(
            "POST",
            f"/lol-champ-select/v1/session/trades/{id}/accept",
            operation_id=f"trade-{id}-accept",
        )
        return await self._write(
            "POST",
            f"/lol-champ-select/v1/ongoing-trade/{id}/clear",
            operation_id=f"trade-{id}-clear",
        )

    async def acceptSwap(self, id):
        await self._write(
            "POST",
            f"/lol-champ-select/v1/session/swaps/{id}/accept",
            operation_id=f"swap-{id}-accept",
        )
        return await self._write(
            "POST",
            f"/lol-champ-select/v1/ongoing-swap/{id}/clear",
            operation_id=f"swap-{id}-clear",
        )

    async def benchSwap(self, championId):
        return await self._write(
            "POST", f"/lol-champ-select/v1/session/bench/swap/{championId}"
        )

    async def reroll(self):
        return await self._write(
            "POST", "/lol-champ-select/v1/session/my-selection/reroll"
        )

    async def selectChampion(self, actionsId, championId, completed=None):
        data = {"championId": championId, "type": "pick"}
        if completed:
            data["completed"] = True
        return await self._write(
            "PATCH", f"/lol-champ-select/v1/session/actions/{actionsId}", data
        )

    async def banChampion(self, actionsId, championId, completed=None):
        data = {"championId": championId, "type": "ban"}
        if completed:
            data["completed"] = completed
        return await self._write(
            "PATCH", f"/lol-champ-select/v1/session/actions/{actionsId}", data
        )

    async def selectConfig(self, skinId, spell1Id=None, spell2Id=None, wardSkinId=None):
        data = {"selectedSkinId": skinId}
        if spell1Id:
            data["spell1Id"] = spell1Id
        if spell2Id:
            data["spell2Id"] = spell2Id
        if wardSkinId:
            data["wardSkinId"] = wardSkinId
        return await self._write(
            "PATCH", "/lol-champ-select/v1/session/my-selection", data
        )

    async def deleteCurrentRunePage(self):
        page = await self.getCurrentRunePage()
        if page.get("isDeletable"):
            return await self._write("DELETE", f"/lol-perks/v1/pages/{page['id']}")
        return None

    async def createRunePage(self, name, primaryId):
        return await self._write(
            "POST",
            "/lol-perks/v1/pages",
            {
                "name": name,
                "primaryStyleId": primaryId,
                "isEditable": True,
                "current": True,
            },
        )

    async def putRunePage(self, id, name, primaryId, secondaryId, perks):
        return await self._write(
            "PUT",
            f"/lol-perks/v1/pages/{id}",
            {
                "id": id,
                "isRecommendationOverride": False,
                "isTemporary": False,
                "name": name,
                "primaryStyleId": primaryId,
                "subStyleId": secondaryId,
                "selectedPerkIds": perks,
                "current": True,
            },
        )

    async def setSummonerSpells(self, spell1Id, spell2Id):
        return await self._write(
            "PATCH",
            "/lol-champ-select/v1/session/my-selection",
            {"spell1Id": spell1Id, "spell2Id": spell2Id},
        )

    async def sendConversationNotify(self, id, message):
        return await self._write(
            "POST",
            f"/lol-chat/v1/conversations/{id}/messages",
            {"body": message, "id": id, "type": "celebration"},
        )

    async def spectate(self, summonerName):
        from app.lol.exceptions import SummonerNotInGame, SummonerNotFound

        info = await self.getSummonerByName(summonerName)
        puuid = info.get("puuid")
        if not puuid:
            raise SummonerNotFound()
        result = await self._write(
            "POST",
            "/lol-spectator/v1/spectate/launch",
            {
                "allowObserveMode": "ALL",
                "dropInSpectateGameId": summonerName,
                "gameQueueType": "",
                "puuid": puuid,
            },
        )
        if result not in (None, ""):
            raise SummonerNotInGame()
        return result

    async def spectateDirectly(self, summonerName):
        from app.lol.exceptions import SummonerNotInGame, SummonerNotFound

        info = await self.getSummonerByName(summonerName)
        puuid = info.get("puuid")
        if not puuid:
            raise SummonerNotFound()
        gaming_info = await self.getSummonerGamingInfoByPuuidViaSgp(puuid)
        credentials = gaming_info.get("playerCredentials")
        if credentials is None:
            raise SummonerNotInGame()
        install_folder = Path(await self.getInstallFolder())
        game_folder = install_folder.parent / "Game"
        parameters = (
            f"spectator {credentials['observerServerIp']}"
            f":{credentials['observerServerPort']}"
            f" {credentials['observerEncryptionKey']}"
            f" {credentials['gameId']} {self.server}"
        )
        return subprocess.Popen(
            ["League of Legends.exe", parameters],
            cwd=str(game_folder),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=True,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )

    async def dodge(self):
        return await self._write(
            "POST",
            "/lol-login/v1/session/invoke",
            {
                "destination": "lcdsServiceProxy",
                "method": "call",
                "args": '["", "teambuilder-draft", "quitV2", ""]',
            },
        )

    async def sendFriendRequest(self, name):
        return await self._write(
            "POST", "/lol-chat/v1/friend-requests", {"name": name}
        )

    async def playAgain(self):
        return await self._write("POST", "/lol-lobby/v2/play-again")

    async def getGameReplay(self, gameId):
        return await self._write(
            "POST",
            f"/lol-replays/v1/rofls/{gameId}/download",
            {"componentType": "replay-button_match-history", "gameId": gameId},
        )

    async def restartClient(self):
        return await self._write("POST", "/riotclient/kill-and-restart-ux")

    def isInTencent(self):
        return self.server.lower() in {
            "tj100",
            "hn1",
            "cq100",
            "gz100",
            "nj100",
            "hn10",
            "tj101",
            "bgp2",
        }

    async def call(self, op, args=None, timeout=15.0):
        return await self.client.call(op, args, timeout)

    async def load_catalog(self):
        import json

        result = await self.client.lcu_catalog(force=True)
        with open(result["path"], "r", encoding="utf-8") as file:
            self.set_catalog(json.load(file))
        await self._build_perk_styles()
        return result

    async def _build_perk_styles(self):
        styles = {}
        for item in self.manager.perks.get("styles", []):
            slots = []
            for slot in item.get("slots", []):
                perks = []
                for rune_id in slot.get("perks", []):
                    perks.append(
                        {
                            "runeId": rune_id,
                            "icon": await self.getRuneIcon(rune_id),
                            "name": self.manager.getRuneName(rune_id),
                            "desc": self.manager.getRuneDesc(rune_id),
                        }
                    )
                slots.append(perks)
            styles[item["id"]] = {
                "name": item["name"],
                "icon": await self.getRuneIcon(item["id"]),
                "slots": slots,
            }
        self.manager.perkStyles = styles

    def set_catalog(self, snapshot):
        self.manager = CatalogManager.from_snapshot(snapshot)
