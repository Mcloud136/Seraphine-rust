import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from app.backend.compat import SidecarConnector


class FakeClient:
    def __init__(self):
        self.writes = []
        self.reads = []

    async def lcu_write(self, method, path, data=None, operation_id=None):
        self.writes.append((method, path, data, operation_id))
        return {"ok": True}

    async def lcu_summoner_by_pid(self, pid):
        return {"pid": pid, "displayName": "Test"}

    async def lcu_get(self, path):
        self.reads.append(path)
        if path.endswith("/regalia"):
            return {"preferredBannerType": "prestige"}
        if path.endswith("/currentpage"):
            return {"id": 7, "isDeletable": True}
        if "/lol-match-history/v1/products/lol/" in path:
            return {"gameCount": 1, "games": [{"gameId": 1}]}
        if "/lol-match-history/v1/products/tft/" in path:
            return {"gameCount": 1, "games": [{"json": {}}]}
        if "/lol-ranked/v1/ranked-stats/" in path:
            return {"errorCode": "not found"}
        if path == "/lol-summoner/v1/current-summoner":
            return {}
        raise AssertionError(path)


class SidecarCompatTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.connector = SidecarConnector.__new__(SidecarConnector)
        self.connector.client = FakeClient()

    async def test_history_methods_return_legacy_lists(self):
        games = await self.connector.getSummonerGamesByPuuid("puuid")
        tft = await self.connector.getTftGamesByPuuid("puuid")
        self.assertEqual(games, [{"gameId": 1}])
        self.assertEqual(tft, [{"json": {}}])

    async def test_legacy_not_found_exceptions_are_preserved(self):
        from app.lol.exceptions import SummonerRankInfoNotFound

        with self.assertRaises(ReferenceError):
            await self.connector.getCurrentSummoner()
        with self.assertRaises(SummonerRankInfoNotFound):
            await self.connector.getRankedStatsByPuuid("puuid")

    async def test_builds_perk_style_schema_for_opgg_ui(self):
        self.connector.manager = SimpleNamespace(
            perks={
                "styles": [
                    {"id": 8100, "name": "Precision", "slots": [{"perks": [8005]}]}
                ]
            },
            getRuneName=lambda rune_id: "Rune %s" % rune_id,
            getRuneDesc=lambda rune_id: "Desc %s" % rune_id,
        )
        self.connector.getRuneIcon = AsyncMock(
            side_effect=lambda rune_id: "rune-%s.png" % rune_id
        )
        await self.connector._build_perk_styles()
        style = self.connector.manager.perkStyles[8100]
        self.assertEqual(style["icon"], "rune-8100.png")
        self.assertEqual(style["slots"][0][0]["runeId"], 8005)
        self.assertEqual(style["slots"][0][0]["name"], "Rune 8005")

    async def test_summoner_lookup_and_autostart_use_sidecar(self):
        summoner = await self.connector.getLoginSummonerByPidAsync(123)
        self.assertEqual(summoner["displayName"], "Test")
        self.connector.start = AsyncMock()
        with patch("app.common.util.getTasklistPath", return_value="windows-api"), patch(
            "app.common.util.getLolClientPid", return_value=123
        ):
            await self.connector.autoStart()
        self.connector.start.assert_awaited_once_with(123)

    async def test_trade_and_swap_use_stable_step_operation_ids(self):
        await self.connector.acceptTrade(12)
        await self.connector.acceptSwap(34)
        self.assertEqual(
            [entry[3] for entry in self.connector.client.writes],
            ["trade-12-accept", "trade-12-clear", "swap-34-accept", "swap-34-clear"],
        )

    async def test_pick_and_ban_payloads_match_legacy_contract(self):
        await self.connector.selectChampion(10, 1, True)
        await self.connector.banChampion(11, 2, True)
        self.assertEqual(
            self.connector.client.writes[0][2],
            {"championId": 1, "type": "pick", "completed": True},
        )
        self.assertEqual(
            self.connector.client.writes[1][2],
            {"championId": 2, "type": "ban", "completed": True},
        )

    async def test_rune_page_management_uses_expected_endpoints(self):
        await self.connector.deleteCurrentRunePage()
        page = await self.connector.createRunePage("test", 8000)
        self.assertEqual(page, {"ok": True})
        self.assertEqual(self.connector.client.reads, ["/lol-perks/v1/currentpage"])
        self.assertEqual(
            self.connector.client.writes[0][:2], ("DELETE", "/lol-perks/v1/pages/7")
        )
        self.assertEqual(
            self.connector.client.writes[1][1], "/lol-perks/v1/pages"
        )


if __name__ == "__main__":
    unittest.main()
