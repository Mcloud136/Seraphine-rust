import asyncio
import os
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

from app.backend.sidecar_client import SidecarClient


class SidecarClientTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        root = Path(__file__).resolve().parents[2]
        self.sidecar_path = os.environ.get(
            "SERAPHINE_SIDECAR_PATH",
            str(root / "rust" / "target" / "release" / "seraphine-sidecar.exe"),
        )
        self.client = SidecarClient(sidecar_path=self.sidecar_path)

    async def asyncTearDown(self):
        await self.client.close()

    async def test_health_diagnostics_and_process_list(self):
        await self.client.start()
        health = await self.client.health()
        self.assertEqual(health["protocol"], 1)
        self.assertEqual(health["pid"], self.client.process.pid)
        self.assertLess(health["memory"]["workingSetBytes"], 10 * 1024 * 1024)

        diagnostics = await self.client.diagnostics()
        self.assertGreaterEqual(diagnostics["metrics"]["requests"], 2)
        self.assertGreaterEqual(diagnostics["latencyUs"]["p95"], 0)
        self.assertIn("requestsPerSecond", diagnostics["rates"])

        processes = await self.client.process_list()
        self.assertIn("leagueClientPids", processes)
        self.assertIsInstance(processes["processes"], list)

        command_line = await self.client.process_command_line(os.getpid())
        self.assertEqual(command_line["pid"], os.getpid())
        self.assertNotIn("token", command_line)

        await self.client.set_game_state("in_game")
        diagnostics = await self.client.diagnostics()
        self.assertEqual(diagnostics["scheduler"]["game_state"], "in_game")

    async def test_lcu_writes_are_disabled_by_default(self):
        from app.backend.protocol import RpcError

        await self.client.start()
        with self.assertRaises(RpcError) as raised:
            await self.client.lcu_write("POST", "/lol-gameflow/v1/reconnect")
        self.assertEqual(raised.exception.code, "WriteDisabled")
        diagnostics = await self.client.diagnostics()
        self.assertFalse(diagnostics["writes"]["enabled"])
        self.assertEqual(diagnostics["writes"]["ledgerEntries"], 0)

    async def test_lcu_write_capability_requires_explicit_opt_in(self):
        from app.backend.protocol import RpcError

        with patch.dict(os.environ, {"SERAPHINE_ENABLE_LCU_WRITES": "1"}):
            await self.client.start()
            diagnostics = await self.client.diagnostics()
            self.assertTrue(diagnostics["writes"]["enabled"])
            with self.assertRaises(RpcError) as raised:
                await self.client.lcu_write("POST", "/lol-gameflow/v1/reconnect")
            self.assertEqual(raised.exception.code, "NotConnected")

    async def test_timeout_sends_cancel_and_clears_pending(self):
        client = SidecarClient()
        sent = []

        async def fake_send(message):
            sent.append(message)

        client._send = fake_send
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(client._call("echo", timeout=1), timeout=0.02)

        self.assertTrue(any(message.get("type") == "cancel" for message in sent))
        self.assertEqual(client.pending, {})
        self.assertEqual(client._inflight_bytes, 0)

    async def test_external_cancel_sends_cancel_and_clears_pending(self):
        client = SidecarClient()
        sent = []

        async def fake_send(message):
            sent.append(message)

        client._send = fake_send
        task = asyncio.create_task(client._call("echo", timeout=1))
        await asyncio.sleep(0)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task

        self.assertTrue(any(message.get("type") == "cancel" for message in sent))
        self.assertEqual(client.pending, {})
        self.assertEqual(client._inflight_bytes, 0)

    async def test_stale_response_epoch_is_rejected(self):
        from app.backend.protocol import RpcError, response

        client = SidecarClient()
        future = asyncio.get_running_loop().create_future()
        client.pending[7] = (future, 10, 1)
        client._inflight_bytes = 10
        client._resolve_response(7, response(7, 2, {"value": "new-client"}))

        with self.assertRaises(RpcError) as raised:
            await future
        self.assertEqual(raised.exception.code, "InvalidParams")
        self.assertIn("epoch", raised.exception.message)
        self.assertEqual(client.pending, {})
        self.assertEqual(client._inflight_bytes, 0)

    async def test_matching_response_epoch_resolves_request(self):
        from app.backend.protocol import response

        client = SidecarClient()
        future = asyncio.get_running_loop().create_future()
        client.pending[7] = (future, 10, 1)
        client._inflight_bytes = 10
        client._resolve_response(7, response(7, 1, {"value": "same-client"}))

        self.assertEqual(await future, {"value": "same-client"})
        self.assertEqual(client.pending, {})
        self.assertEqual(client._inflight_bytes, 0)

    async def test_catalog_and_sgp_fail_fast_without_lcu(self):
        from app.backend.protocol import RpcError

        await self.client.start()
        with self.assertRaises(RpcError) as catalog_error:
            await self.client.lcu_catalog()
        self.assertEqual(catalog_error.exception.code, "NotConnected")
        with self.assertRaises(RpcError) as sgp_error:
            await self.client.sgp_get("/test")
        self.assertEqual(sgp_error.exception.code, "NotConnected")

    async def test_heartbeat_keeps_healthy_sidecar_alive(self):
        import asyncio

        client = SidecarClient(
            sidecar_path=self.sidecar_path,
            heartbeat_interval=0.05,
            heartbeat_timeout=0.2,
        )
        try:
            await client.start()
            pid = client.pid
            await asyncio.sleep(0.3)
            await client.health()
            self.assertFalse(client.closed)
            self.assertEqual(client.pid, pid)
        finally:
            await client.close(forget=True)

    async def test_heartbeat_timeout_closes_unresponsive_sidecar(self):
        import asyncio

        class NoPongClient(SidecarClient):
            async def _reader_loop(self):
                while not self.closed:
                    await asyncio.sleep(1)

        client = NoPongClient(
            sidecar_path=self.sidecar_path,
            heartbeat_interval=0.05,
            heartbeat_timeout=0.12,
        )
        try:
            await client.start()
            await asyncio.sleep(0.35)
            self.assertTrue(client.closed)
            self.assertIsNone(client.process)
        finally:
            await client.close(forget=True)

    async def test_restart_reuses_saved_lcu_connection(self):
        client = SidecarClient()
        client.closed = True
        client._connection_params = {"pid": 123, "cacheRoot": "cache"}

        async def fake_start_locked():
            client.closed = False

        client._start_locked = fake_start_locked
        client.call = AsyncMock(return_value={"epoch": 7})
        await client.start()
        client.call.assert_awaited_once_with(
            "lcu.connect", {"pid": 123, "cacheRoot": "cache"}, timeout=60.0
        )
        self.assertEqual(client.epoch, 7)

    async def test_lcu_connect_saves_reconnect_parameters(self):
        client = SidecarClient()
        client.call = AsyncMock(return_value={"epoch": 4})
        await client.lcu_connect(123, proxy="http://127.0.0.1:8080", cache_root="cache")
        self.assertEqual(
            client._connection_params,
            {"pid": 123, "proxy": "http://127.0.0.1:8080", "cacheRoot": "cache"},
        )
        self.assertEqual(client.epoch, 4)

    async def test_unknown_method_returns_rpc_error(self):
        from app.backend.protocol import RpcError

        await self.client.start()
        await self.client.health()
        with self.assertRaises(RpcError) as raised:
            await self.client.call("does.not.exist")
        self.assertEqual(raised.exception.code, "MethodNotFound")
        self.assertEqual(self.client._inflight_bytes, 0)

    async def test_large_request_is_rejected_before_ipc(self):
        await self.client.start()
        with self.assertRaises(Exception) as raised:
            await self.client.call("echo", {"payload": "x" * (256 * 1024)})
        self.assertIn("too large", str(raised.exception))
        self.assertEqual(self.client._inflight_bytes, 0)


if __name__ == "__main__":
    unittest.main()
