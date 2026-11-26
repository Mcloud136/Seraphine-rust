from app.common.config import LOCAL_PATH
from app.common.logger import logger

TAG = "AramBuff"


class AramBuff:
    """
    ARAM (Howling Abyss) balance modifiers.

    Data comes from a CI-generated snapshot of the League of Legends Wiki
    (Module:ChampionData/data, CC BY-SA 3.0), bundled with the app and
    optionally refreshed from a configured HTTPS feed (see app/backend/aram_data.py).
    """
    ARAM_CFG_PATH = f"{LOCAL_PATH}/AramBuff.json"
    TAG = TAG
    data = None
    last_warning = None

    @classmethod
    async def checkAndUpdate(cls):
        from app.backend import aram_data

        local = aram_data.load_data()
        if local:
            cls.data = local

        game_version = cls.__gameVersion()
        if aram_data.auto_update_enabled():
            result = await aram_data.maybe_update(game_version=game_version)
            if result.get("updated") and result.get("data"):
                cls.data = result["data"]
            elif result.get("reason") == "error":
                cls.last_warning = result.get("error")

        try:
            from app.backend.community import get_sidecar_client, load_aram_buffs
            client = get_sidecar_client()
        except Exception:
            client = None

        if client is None:
            if cls.data is None:
                from app.backend.community import load_aram_buffs
                cls.data = load_aram_buffs()
            return

        try:
            result = await client.aram_buff_refresh(game_version)
        except Exception as error:
            logger.warning(f"Sidecar ARAM refresh failed: {error}", cls.TAG)
            cls.data = load_aram_buffs() or cls.data
            return

        if result.get("available") and result.get("data"):
            cls.data = result["data"]
        else:
            cls.data = load_aram_buffs() or cls.data

    @classmethod
    def __gameVersion(cls):
        try:
            from app.common.util import getLolClientVersion
            return getLolClientVersion()
        except Exception:
            return None

    @classmethod
    def isAvailable(cls) -> bool:
        return cls.data is not None

    @classmethod
    def getDataVersion(cls):
        if not cls.data:
            return None
        return cls.data.get("version")

    @classmethod
    def getInfoByChampionId(cls, championId):
        if not cls.isAvailable():
            return None

        return cls.data['champions'].get(str(championId))