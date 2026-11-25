import json
import os

from app.common.config import LOCAL_PATH
from app.common.logger import logger


class AramBuff:
    """
    #### Data usage has been authorized
    Powered by: ARAM Home
    Site: http://www.jddld.com
    """
    ARAM_CFG_PATH = f"{LOCAL_PATH}/AramBuff.json"
    URL = "http://www.jddld.com"
    TAG = "AramBuff"
    APPID = 1
    APP_SECRET = os.environ.get("SERAPHINE_ARAM_APPSECRET")
    data = None

    @classmethod
    async def checkAndUpdate(cls):
        from app.backend.community import (
            get_sidecar_client,
            load_aram_buffs,
            refresh_aram_buffs,
        )

        client = get_sidecar_client()
        if client is not None:
            result = await refresh_aram_buffs(client)
            AramBuff.data = result.get("data") if result else load_aram_buffs()
            return

        m = cls()
        if m.__needUpdate():
            await m.__update()
        elif AramBuff.data is None:
            AramBuff.data = load_aram_buffs()

    def __needUpdate(self):
        """
        Check if the cached data matches the current version, if not, try to update

        It's best to call this after the game starts, otherwise when multiple clients exist,
        `cfg.lolFolder` may not be accurate (different versions between regions)

        @return :
        - `True` -> needs update
        - `False` -> no update needed

        TODO: Historical version data query interface not yet provided
        """
        try:
            from app.common.util import getLolClientVersion

            lolVersion = getLolClientVersion()
        except:
            return True

        if not os.path.exists(self.ARAM_CFG_PATH):
            return True

        with open(self.ARAM_CFG_PATH, 'r') as f:
            try:
                AramBuff.data = json.loads(f.read())
            except:
                return True

            # Compatible with older versions of JSON
            if AramBuff.data.get('version') == None:
                return True

            return AramBuff.getDataVersion() != lolVersion

    @classmethod
    def getDataVersion(cls):
        return AramBuff.data.get("version")

    async def __update(self):
        import aiohttp

        if not self.APP_SECRET:
            logger.warning("ARAM appsecret is not configured", self.TAG)
            return

        url = f'{self.URL}/index.php'
        params = {
            'appid': self.APPID,
            'appsecret': self.APP_SECRET,
            's': 'news',
            'c': 'search',
            'api_call_function': 'module_list',
            'pagesize': '200'
        }

        try:
            async with aiohttp.ClientSession() as session:
                res = await session.get(url, params=params, proxy=None, ssl=False)
                data = await res.json()
        except:
            logger.warning(f"Getting Aram buff failed", self.TAG)
            return

        if data.get('code') != 1:
            logger.warning(f"Update Aram buff failed, data: {data}", self.TAG)
            return

        try:
            data: dict = data['data']

            version = data.pop('banben')
            champions = {item['heroid']: item for item in data.values()}

            AramBuff.data = {
                'champions': champions,
                'version': version
            }

            with open(self.ARAM_CFG_PATH, 'w') as f:
                json.dump(AramBuff.data, f)

        except:
            logger.warning(f"Parse Aram buff failed, data: {data}", self.TAG)
            return

    @classmethod
    def isAvailable(cls) -> bool:
        return AramBuff.data != None

    @classmethod
    def getInfoByChampionId(cls, championId):
        if not AramBuff.isAvailable():
            return None

        return AramBuff.data['champions'].get(str(championId))
