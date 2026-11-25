import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from app.backend.opgg import resolve_tier_placeholders


class OpggPlaceholderTests(unittest.IsolatedAsyncioTestCase):
    async def test_resolves_champion_fields_once_per_champion(self):
        connector = SimpleNamespace(
            manager=SimpleNamespace(
                getChampionNameById=lambda champion_id: f"Champion {champion_id}"
            ),
            getChampionIcon=AsyncMock(
                side_effect=lambda champion_id: f"icon-{champion_id}.png"
            ),
        )
        payload = {
            "data": [
                {
                    "name": "@champion_name:1",
                    "icon": "@champion:1",
                    "counters": [{"icon": "@champion:2"}],
                },
                {"name": "@champion_name:1", "icon": "@champion:1"},
            ],
            "version": "16.23",
        }
        with patch("app.lol.connector.connector", connector):
            result = await resolve_tier_placeholders(payload)
        self.assertEqual(result["data"][0]["name"], "Champion 1")
        self.assertEqual(result["data"][0]["icon"], "icon-1.png")
        self.assertEqual(result["data"][0]["counters"][0]["icon"], "icon-2.png")
        self.assertEqual(connector.getChampionIcon.await_count, 2)


if __name__ == "__main__":
    unittest.main()
