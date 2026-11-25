import sys
import tempfile
import types
import unittest
from unittest.mock import patch

from app.backend.community import search_champion_aliases

_config = types.ModuleType("app.common.config")
_config.LOCAL_PATH = tempfile.gettempdir()
_config.cfg = types.SimpleNamespace()
_previous_config = sys.modules.get("app.common.config")
_previous_logger = sys.modules.get("app.common.logger")
_logger = types.ModuleType("app.common.logger")
_logger.logger = types.SimpleNamespace(
    info=lambda *args, **kwargs: None,
    warning=lambda *args, **kwargs: None,
)
sys.modules["app.common.config"] = _config
sys.modules["app.common.logger"] = _logger
try:
    from app.lol.aram import AramBuff
    from app.lol.champions import ChampionAlias
finally:
    if _previous_config is None:
        sys.modules.pop("app.common.config", None)
    else:
        sys.modules["app.common.config"] = _previous_config
    if _previous_logger is None:
        sys.modules.pop("app.common.logger", None)
    else:
        sys.modules["app.common.logger"] = _previous_logger


class CommunityTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        ChampionAlias.data = None
        AramBuff.data = None

    async def test_champion_alias_uses_sidecar_data_without_duplicate_aliases(self):
        class Client:
            async def champion_alias_refresh(self, game_version):
                self.version = game_version
                return {
                    "available": True,
                    "data": {
                        "version": "16.24",
                        "champions": {"1": "annie,黑暗之女", "901": "smolder,小火龙"},
                    },
                }

        client = Client()
        with patch("app.backend.community.get_sidecar_client", return_value=client), patch(
            "app.backend.community.get_game_version", return_value="16.24"
        ):
            await ChampionAlias.checkAndUpdate()

        self.assertEqual(client.version, "16.24")
        self.assertEqual(ChampionAlias.getChampionIdsByAliasFuzzily("ANN"), [1])
        self.assertEqual(ChampionAlias.getChampionIdsByAliasFuzzily(""), [1, 901])
        self.assertEqual(
            ChampionAlias.getChampionsAlias()["901"], "smolder,小火龙"
        )

    async def test_aram_unavailable_falls_back_to_local_cache(self):
        class Client:
            async def aram_buff_refresh(self, game_version):
                return {"available": False, "data": None}

        cached = {
            "version": "16.24",
            "champions": {"75": {"catname": "沙漠皇帝"}},
        }
        with patch("app.backend.community.get_sidecar_client", return_value=Client()), patch(
            "app.backend.community.get_game_version", return_value="16.24"
        ), patch("app.backend.community.load_aram_buffs", return_value=cached):
            await AramBuff.checkAndUpdate()

        self.assertEqual(AramBuff.getInfoByChampionId(75)["catname"], "沙漠皇帝")
        AramBuff.data = {"version": "16.25", "champions": {}}
        self.assertIsNone(AramBuff.getInfoByChampionId(75))

    def test_alias_search_is_stateless_and_case_insensitive(self):
        data = {"champions": {"2": "aram,亚连", "1": "annie,安妮"}}
        self.assertEqual(search_champion_aliases(data, "RAM"), [2])
        self.assertEqual(search_champion_aliases(data, "ann"), [1])
        self.assertEqual(search_champion_aliases(data, "missing"), [])
        self.assertEqual(search_champion_aliases(data, ""), [1, 2])


if __name__ == "__main__":
    unittest.main()
