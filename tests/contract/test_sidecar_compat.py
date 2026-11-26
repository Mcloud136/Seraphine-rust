import asyncio
import unittest
from collections import deque
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock, Mock, patch

from app.backend.compat import SidecarConnector
from app.lol.match_history import extract_match_history_games


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
            return {"gameCount": 1, "games": {"games": [{"gameId": 1}]}}
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

    def test_connector_exposes_legacy_call_stack(self):
        connector = SidecarConnector()
        self.assertEqual(list(connector.callStack), [])

    async def test_call_retries_five_times_and_emits_exception(self):
        connector = SidecarConnector.__new__(SidecarConnector)
        connector.client = FakeClient()
        connector.client.call = AsyncMock(side_effect=RuntimeError("boom"))
        connector.callStack = deque(maxlen=10)
        with patch("app.backend.compat.signalBus") as signals:
            with self.assertRaises(RuntimeError):
                await connector.call("health")
        self.assertEqual(connector.client.call.await_count, 5)
        signals.lcuApiExceptionRaised.emit.assert_called_once()
        self.assertEqual(connector.callStack[-1].op, "health")

    async def test_call_returns_after_transient_failure(self):
        connector = SidecarConnector.__new__(SidecarConnector)
        connector.client = FakeClient()
        connector.client.call = AsyncMock(
            side_effect=[RuntimeError("temporary"), {"ok": True}]
        )
        connector.callStack = deque(maxlen=10)
        self.assertEqual(await connector.call("health"), {"ok": True})
        self.assertEqual(connector.client.call.await_count, 2)

    async def test_summoner_not_found_is_not_retried(self):
        from app.lol.exceptions import SummonerNotFound

        connector = SidecarConnector.__new__(SidecarConnector)
        connector.client = FakeClient()
        connector.client.call = AsyncMock(side_effect=SummonerNotFound())
        connector.callStack = deque(maxlen=10)
        with self.assertRaises(SummonerNotFound):
            await connector.call("getSummonerByName")
        self.assertEqual(connector.client.call.await_count, 1)

    async def test_start_failure_becomes_retry_maximum_attempts(self):
        from app.lol.exceptions import RetryMaximumAttempts

        connector = SidecarConnector.__new__(SidecarConnector)
        connector._start_once = AsyncMock(side_effect=RuntimeError("not ready"))
        connector.callStack = deque(maxlen=10)
        with patch("app.backend.compat.signalBus"):
            with self.assertRaises(RetryMaximumAttempts):
                await connector.start(123)
        self.assertEqual(connector._start_once.await_count, 5)

    async def test_history_methods_return_legacy_lists(self):
        games = await self.connector.getSummonerGamesByPuuid("puuid")
        tft = await self.connector.getTftGamesByPuuid("puuid")
        self.assertEqual(games, [{"gameId": 1}])
        self.assertEqual(tft, [{"json": {}}])

    def test_legacy_match_history_shape_is_supported(self):
        games = [{"gameId": 1}]
        self.assertEqual(extract_match_history_games({"games": games}), games)

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

    async def test_forward_events_ignores_stale_epochs(self):
        class EventClient:
            epoch = 2

            async def events_stream(self):
                yield {"epoch": 1, "topic": "gameflow.changed", "data": "Lobby"}
                yield {"epoch": 2, "topic": "gameflow.changed", "data": "InProgress"}

        connector = SidecarConnector.__new__(SidecarConnector)
        connector.client = EventClient()
        emitted = []
        signals = SimpleNamespace(
            gameStatusChanged=SimpleNamespace(emit=emitted.append),
            champSelectChanged=SimpleNamespace(emit=lambda value: None),
            currentSummonerProfileChanged=SimpleNamespace(emit=lambda value: None),
        )
        with patch("app.backend.compat.signalBus", signals):
            await connector._forward_events()
        self.assertEqual(emitted, ["InProgress"])

    async def test_restart_cancels_previous_event_consumer(self):
        class RestartClient:
            epoch = 1

            async def start(self):
                pass

            async def lcu_connect(self, *args, **kwargs):
                return {"epoch": self.epoch}

            async def events_stream(self):
                while True:
                    await asyncio.sleep(1)
                    yield {}

            async def close(self, forget=False):
                pass

        connector = SidecarConnector.__new__(SidecarConnector)
        connector.client = RestartClient()
        connector.load_catalog = AsyncMock()
        connector.forward_events = False
        connector.event_task = None
        connector.callStack = deque(maxlen=10)
        with patch("app.backend.compat.ensure_cache_root", return_value=Path(".")):
            await connector.start(1)
            first = connector.event_task
            await connector.start(2)
        self.assertIsNot(connector.event_task, first)
        self.assertTrue(first.cancelled() or first.done())
        await connector.close()

    async def test_write_paths_strip_trailing_slash(self):
        await self.connector._write(
            "POST", "/lol-challenges/v1/update-player-preferences/", {"challengeIds": []}
        )
        self.assertEqual(
            self.connector.client.writes[0][1],
            "/lol-challenges/v1/update-player-preferences",
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


class StartupOrderingTests(unittest.IsolatedAsyncioTestCase):
    async def test_opgg_initialization_waits_for_champion_aliases(self):
        from app.view.main_window import MainWindow

        alias_started = asyncio.Event()
        release_alias = asyncio.Event()

        async def refresh_aliases():
            alias_started.set()
            await release_alias.wait()

        window = SimpleNamespace(
            _MainWindow__startConnector=AsyncMock(return_value=True),
            checkAndSwitchTo=Mock(),
            careerInterface=object(),
            isClientProcessRunning=False,
            _MainWindow__changeCareerToCurrentSummoner=AsyncMock(),
            _MainWindow__updateAvatarIconName=AsyncMock(),
            startInterface=SimpleNamespace(hideLoadingPage=Mock()),
            _MainWindow__setLolInstallFolder=Mock(),
            auxiliaryFuncInterface=SimpleNamespace(
                initChampionList=AsyncMock(),
                lockConfigCard=SimpleNamespace(loadNowMode=Mock()),
            ),
            opggWindow=SimpleNamespace(
                initWindow=AsyncMock(), setHomeInterfaceEnabled=Mock()
            ),
            _MainWindow__unlockInterface=Mock(),
            _MainWindow__onGameStatusChanged=AsyncMock(),
        )

        with patch("app.view.main_window.opgg.start", new=AsyncMock()), patch(
            "app.view.main_window.connector.getInstallFolder",
            new=AsyncMock(return_value="C:/League"),
        ), patch(
            "app.view.main_window.connector.getGameStatus",
            new=AsyncMock(return_value=None),
        ), patch(
            "app.view.main_window.AramBuff.checkAndUpdate", new=AsyncMock()
        ), patch(
            "app.view.main_window.ChampionAlias.checkAndUpdate",
            new=refresh_aliases,
        ):
            startup = asyncio.ensure_future(
                MainWindow._MainWindow__onLolClientStarted(window, 123)
            )
            await asyncio.wait_for(alias_started.wait(), timeout=1)
            await asyncio.sleep(0)
            window.opggWindow.initWindow.assert_not_called()
            release_alias.set()
            await asyncio.wait_for(startup, timeout=1)

        window.opggWindow.initWindow.assert_called_once_with()


if __name__ == "__main__":
    unittest.main()
