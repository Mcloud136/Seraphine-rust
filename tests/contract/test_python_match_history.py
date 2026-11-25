import asyncio
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from app.lol import connector as connector_module
from app.lol.connector import LolClientConnector


class PythonMatchHistoryTests(unittest.IsolatedAsyncioTestCase):
    async def test_current_and_legacy_history_shapes_return_lists(self):
        connector = LolClientConnector()
        expected = [{"gameId": 1}]
        payloads = [
            {"games": expected},
            {"games": {"gameCount": 1, "games": expected}},
        ]
        with patch.object(
            connector_module.connector, "semaphore", asyncio.Semaphore(1)
        ):
            for method_name in (
                "getSummonerGamesByPuuid",
                "getSummonerGamesByPuuidSlowly",
            ):
                for payload in payloads:
                    with self.subTest(method=method_name, payload=payload):
                        response = SimpleNamespace(
                            json=AsyncMock(return_value=payload)
                        )
                        with patch.object(
                            connector,
                            "_LolClientConnector__get",
                            AsyncMock(return_value=response),
                        ):
                            games = await getattr(connector, method_name)(
                                "puuid", 0, 19
                            )
                        self.assertEqual(games, expected)


if __name__ == "__main__":
    unittest.main()
