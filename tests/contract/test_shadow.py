import asyncio
import unittest

from app.backend.shadow import ShadowConnector


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


if __name__ == "__main__":
    unittest.main()
