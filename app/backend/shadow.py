import asyncio
import json
import logging
import os
from datetime import datetime

from .compat import SidecarConnector


logger = logging.getLogger("Shadow")
LOCAL_PATH = os.path.join(os.environ.get("APPDATA", os.path.expanduser("~")), "Seraphine")


READ_OPERATIONS = {
    "getCurrentSummoner",
    "getSummonerByName",
    "getSummonerByPuuid",
    "getSummonerGamesByPuuid",
    "getSummonerGamesByPuuidSlowly",
    "getTftGamesByPuuid",
    "getGameDetailByGameId",
    "getRankedStatsByPuuid",
    "getGameflowSession",
    "getChampSelectSession",
    "getGameStatus",
    "getMapSide",
    "getReadyCheckStatus",
}

# Dynamic fields that legitimately differ between two reads of the same live
# game (for example, a TFT match still in progress).
VOLATILE_FIELDS = frozenset({"time_eliminated"})


def normalize_for_comparison(value, volatile_fields=VOLATILE_FIELDS):
    if isinstance(value, dict):
        return {
            key: normalize_for_comparison(item, volatile_fields)
            for key, item in value.items()
            if key not in volatile_fields
        }
    if isinstance(value, list):
        return [normalize_for_comparison(item, volatile_fields) for item in value]
    return value


class ShadowConnector:
    def __init__(self, primary):
        self.primary = primary
        self.shadow = SidecarConnector(forward_events=False)
        self.shadow_ready = False
        self._comparisons = set()

    def __getattr__(self, name):
        return getattr(self.primary, name)

    async def start(self, pid):
        await self.primary.start(pid)
        try:
            await self.shadow.start(pid)
            self.shadow_ready = True
        except BaseException as error:
            logger.warning("Rust shadow backend failed to start: %s" % error, "Shadow")

    async def close(self):
        await self.shadow.close()
        await self.primary.close()
        self.shadow_ready = False

    async def call(self, op, args=None, timeout=15.0):
        return await self.primary.call(op, args, timeout)

    def _schedule_comparison(self, name, args, kwargs, primary_result):
        if not self.shadow_ready or name in self._comparisons:
            return
        shadow_method = getattr(self.shadow, name, None)
        if shadow_method is None:
            return
        self._comparisons.add(name)
        asyncio.create_task(
            self._compare(name, shadow_method, args, kwargs, primary_result)
        )

    async def _compare(self, name, shadow_method, args, kwargs, primary_result):
        try:
            shadow_result = await shadow_method(*args, **kwargs)
            if normalize_for_comparison(shadow_result) != normalize_for_comparison(primary_result):
                self._record_mismatch(name, shadow_result, primary_result)
        except asyncio.CancelledError:
            raise
        except BaseException as error:
            logger.warning("Rust shadow comparison failed for %s: %s" % (name, error), "Shadow")
        finally:
            self._comparisons.discard(name)

    def _record_mismatch(self, name, shadow_result, primary_result):
        record = {
            "time": datetime.now().isoformat(timespec="seconds"),
            "operation": name,
            "shadowType": type(shadow_result).__name__,
            "primaryType": type(primary_result).__name__,
            "shadowKeys": sorted(shadow_result.keys()) if isinstance(shadow_result, dict) else None,
            "primaryKeys": sorted(primary_result.keys()) if isinstance(primary_result, dict) else None,
        }
        try:
            os.makedirs(LOCAL_PATH, exist_ok=True)
            with open(os.path.join(LOCAL_PATH, "shadow_diff.jsonl"), "a", encoding="utf-8") as file:
                file.write(json.dumps(record, ensure_ascii=False) + "\n")
        except OSError as error:
            logger.warning("Failed to record shadow diff: %s" % error, "Shadow")


def wrap_read_method(name):
    async def wrapper(self, *args, **kwargs):
        result = await getattr(self.primary, name)(*args, **kwargs)
        self._schedule_comparison(name, args, kwargs, result)
        return result

    return wrapper


for _name in READ_OPERATIONS:
    if hasattr(ShadowConnector, _name):
        continue
    setattr(ShadowConnector, _name, wrap_read_method(_name))
