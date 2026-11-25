import json
import os

from app.common.config import LOCAL_PATH
from app.common.logger import logger


class ChampionAlias:
    CHAMPION_CFG_PATH = f"{LOCAL_PATH}/ChampionAlias.json"
    URL = 'https://game.gtimg.cn/images/lol/act/img/js/heroList/hero_list.js'
    TAG = "ChampionAlias"

    data = None

    @classmethod
    async def checkAndUpdate(cls):
        from app.backend.community import (
            get_sidecar_client,
            load_champion_aliases,
            refresh_champion_aliases,
        )

        client = get_sidecar_client()
        if client is not None:
            result = await refresh_champion_aliases(client)
            ChampionAlias.data = result.get("data") if result else load_champion_aliases()
            return

        m = cls()
        if m.__needUpdate():
            await m.__update()
        elif ChampionAlias.data is None:
            ChampionAlias.data = load_champion_aliases()

    async def __update(self):
        import aiohttp

        logger.info("Update champions alias", self.TAG)

        try:
            async with aiohttp.ClientSession() as session:
                res = await session.get(self.URL, proxy=None, ssl=False)

                # Don't know why this doesn't work:
                # res = await res.json()

                s = str(await res.read(), encoding='utf-8')
                res = json.loads(s)

            champions = {}

            for champion in res['hero']:
                championId = champion['heroId']
                championAlias = champion['alias']
                keywords = champion['keywords']
                key = f"{keywords.lower()},{championAlias.lower()}"

                champions[championId] = key

            ChampionAlias.data = {
                'champions': champions,
                'version': res['version']
            }

            with open(self.CHAMPION_CFG_PATH, 'w') as f:
                json.dump(ChampionAlias.data, f)
        except:
            logger.info("Update champions alias FAILED", self.TAG)
            ChampionAlias.data = None

    def __needUpdate(self):
        try:
            from app.common.util import getLolClientVersion

            lolVersion = getLolClientVersion()
        except:
            return True

        if not os.path.exists(self.CHAMPION_CFG_PATH):
            return True

        with open(self.CHAMPION_CFG_PATH, 'r') as f:
            try:
                ChampionAlias.data = json.loads(f.read())
            except:
                return True

        return ChampionAlias.getDataVersion() != lolVersion

    @staticmethod
    def computeDict(dic: dict, key, fun: callable):
        dic[key] = fun(key, dic.get(key))

    @classmethod
    def getChampionsAlias(cls) -> dict:
        champions = ChampionAlias.data['champions']
        for championId, alias in {
            "901": "小火龙",
            "950": "狗",
            "902": "丁真",
            "897": "黑龙",
        }.items():
            if championId in champions:
                aliases = [item for item in champions[championId].split(',') if item]
                if alias not in aliases:
                    aliases.append(alias)
                champions[championId] = ','.join(aliases)
        return champions

    @classmethod
    def isAvailable(cls) -> bool:
        return ChampionAlias.data != None

    @classmethod
    def getDataVersion(cls) -> str:
        return ChampionAlias.data.get("version")

    @classmethod
    def getChampionIdsByAliasFuzzily(cls, alias):
        from app.backend.community import search_champion_aliases

        return search_champion_aliases(ChampionAlias.getChampionsAlias(), alias)
