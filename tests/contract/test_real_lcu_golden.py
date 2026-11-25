import asyncio
import os
import unittest
import urllib3
from pathlib import Path

import psutil
import requests

from app.backend.sidecar_client import SidecarClient

urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)


@unittest.skipUnless(
    os.environ.get("SERAPHINE_TEST_LCU_PID"),
    "set SERAPHINE_TEST_LCU_PID to run real LCU golden diff",
)
class RealLcuGoldenDiffTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        root = Path(__file__).resolve().parents[2]
        self.client = SidecarClient(
            sidecar_path=os.environ.get(
                "SERAPHINE_SIDECAR_PATH",
                str(root / "rust" / "target" / "release" / "seraphine-sidecar.exe"),
            )
        )
        await self.client.start()
        self.pid = int(os.environ["SERAPHINE_TEST_LCU_PID"])
        command_line = psutil.Process(self.pid).cmdline()
        self.port = next(
            int(value.split("=", 1)[1])
            for value in command_line
            if value.startswith("--app-port=")
        )
        self.token = next(
            value.split("=", 1)[1]
            for value in command_line
            if value.startswith("--remoting-auth-token=")
        )
        self.base_url = f"https://127.0.0.1:{self.port}"
        await self.client.lcu_connect(self.pid)

    async def asyncTearDown(self):
        await self.client.close()

    def python_get(self, path):
        response = requests.get(
            self.base_url + path,
            auth=("riot", self.token),
            verify=False,
            timeout=5,
        )
        response.raise_for_status()
        return response.json()

    async def test_lcu_read_endpoints_match(self):
        summoner = self.python_get("/lol-summoner/v1/current-summoner")
        puuid = summoner["puuid"]
        paths = [
            "/lol-summoner/v1/current-summoner",
            f"/lol-match-history/v1/products/lol/{puuid}/matches?begIndex=0&endIndex=1",
            f"/lol-ranked/v1/ranked-stats/{puuid}",
            "/lol-gameflow/v1/gameflow-phase",
        ]
        for path in paths:
            with self.subTest(path=path):
                self.assertEqual(
                    await self.client.lcu_get(path),
                    self.python_get(path),
                )


if __name__ == "__main__":
    unittest.main()
