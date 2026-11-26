import asyncio
import unittest
from unittest.mock import AsyncMock, patch

from app.backend.shadow import ShadowConnector, normalize_for_comparison


class FakePrimary:
    def __init__(self):
        self.calls = 0

    async def getCurrentSummoner(self):
        self.calls += 1
        return {"summonerId": 1}


class FakeShadow:
    def __init__(self):
        self.calls = 0

    async def getCurrentSummoner(self):
        self.calls += 1
        return {"summonerId": 1}

    async def close(self):
        pass


class ShadowConnectorTests(unittest.IsolatedAsyncioTestCase):
    async def test_read_calls_are_mirrored(self):
        primary = FakePrimary()
        connector = ShadowConnector(primary)
        shadow = FakeShadow()
        connector.shadow = shadow
        connector.shadow_ready = True

        result = await connector.getCurrentSummoner()
        await asyncio.sleep(0)

        self.assertEqual(result, {"summonerId": 1})
        self.assertEqual(primary.calls, 1)
        self.assertEqual(shadow.calls, 1)

    async def test_dynamic_fields_do_not_create_mismatches(self):
        connector = ShadowConnector(FakePrimary())
        connector.shadow = FakeShadow()
        connector.shadow_ready = True
        connector.shadow.getTftGamesByPuuid = AsyncMock(return_value={
            "games": [{"gameId": 1, "time_eliminated": 42}],
        })
        connector.shadow_ready = True
        primary = {"games": [{"gameId": 1, "time_eliminated": 41}]}
        with patch.object(connector, "_record_mismatch") as record:
            await connector._compare(
                "getTftGamesByPuuid",
                connector.shadow.getTftGamesByPuuid,
                ("puuid",), {}, primary,
            )
        record.assert_not_called()

    async def test_non_dynamic_fields_still_create_mismatches(self):
        connector = ShadowConnector(FakePrimary())
        connector.shadow = FakeShadow()
        connector.shadow_ready = True
        connector.shadow.getTftGamesByPuuid = AsyncMock(return_value={
            "games": [{"gameId": 2, "time_eliminated": 41}],
        })
        with patch.object(connector, "_record_mismatch") as record:
            await connector._compare(
                "getTftGamesByPuuid",
                connector.shadow.getTftGamesByPuuid,
                ("puuid",), {}, {"games": [{"gameId": 1, "time_eliminated": 42}]},
            )
        record.assert_called_once()

    def test_volatile_normalization_is_recursive(self):
        primary = {"games": [{"gameId": 1, "time_eliminated": 42}],
                   "nested": {"time_eliminated": 1, "value": 3}}
        shadow = {"games": [{"gameId": 1, "time_eliminated": 41}],
                  "nested": {"value": 3}}
        self.assertEqual(normalize_for_comparison(primary),
                         normalize_for_comparison(shadow))


if __name__ == "__main__":
    unittest.main()
