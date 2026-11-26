import json
import os


def get_sidecar_client():
    try:
        from app.backend.sidecar_client import SidecarClient
        from app.lol.connector import connector
    except ImportError:
        return None
    client = getattr(connector, "client", None)
    return client if isinstance(client, SidecarClient) else None


def get_game_version():
    try:
        from app.common.util import getLolClientVersion

        return getLolClientVersion()
    except Exception:
        return None


async def refresh_champion_aliases(client):
    result = await client.champion_alias_refresh(get_game_version())
    return result if result.get("available") else None


def load_champion_aliases():
    from app.common.config import LOCAL_PATH

    try:
        with open(os.path.join(LOCAL_PATH, "ChampionAlias.json"), encoding="utf-8") as file:
            data = json.load(file)
        return data if isinstance(data.get("champions"), dict) else None
    except (OSError, ValueError, TypeError):
        return None


def load_aram_buffs():
    from app.common.config import LOCAL_PATH

    try:
        with open(os.path.join(LOCAL_PATH, "AramBuff.json"), encoding="utf-8") as file:
            data = json.load(file)
        return data if isinstance(data.get("champions"), dict) else None
    except (OSError, ValueError, TypeError):
        return None


def search_champion_aliases(data, query):
    champions = data.get("champions", data)
    query = query.lower()
    return sorted(
        int(champion_id)
        for champion_id, aliases in champions.items()
        if not query or query in str(aliases).lower()
    )
