import asyncio
import json
import os
import shutil
import ssl
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from app.backend import protocol
from app.backend.protocol import RpcError
from app.backend.sidecar_client import SidecarClient


class MockLcuHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    session = {}
    write_attempts = 0
    apply_write = True
    slow_started = threading.Event()
    release_slow = threading.Event()
    slow_requests = 0
    write_started = threading.Event()
    release_write = threading.Event()

    def log_message(self, *args):
        pass

    def send_json(self, status, value):
        payload = json.dumps(value).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        if self.path == "/entitlements/v1/token":
            self.send_json(200, {"accessToken": "mock-sgp-token"})
        elif self.path == "/lol-champ-select/v1/session":
            self.send_json(200, self.session)
        elif self.path == "/slow":
            type(self).slow_requests += 1
            type(self).slow_started.set()
            type(self).release_slow.wait(5)
            self.send_json(200, {"result": "slow"})
        elif self.path == "/fast":
            self.send_json(200, {"result": "fast"})
        else:
            self.send_json(426, {"error": "websocket mock not implemented"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        data = json.loads(self.rfile.read(length) or b"{}")
        if self.path == "/lol-champ-select/v1/session/actions/10":
            type(self).write_started.set()
            type(self).release_write.wait(5)
            type(self).write_attempts += 1
            if type(self).apply_write:
                action = self.session["actions"][0][0]
                action["championId"] = data.get("championId", 0)
                action["completed"] = bool(data.get("completed", False))
            self.send_json(500, {"error": "mock write response lost"})
            return
        self.send_json(404, {"error": "not found"})


class MockLcuServer(ThreadingHTTPServer):
    request_queue_size = 128

    def handle_error(self, request, client_address):
        pass


class LcuWriteConfirmationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        if shutil.which("openssl") is None:
            self.skipTest("openssl is required for the mock LCU HTTPS server")
        self.directory = tempfile.TemporaryDirectory()
        self.cert = Path(self.directory.name) / "cert.pem"
        self.key = Path(self.directory.name) / "key.pem"
        subprocess.run(
            [
                "openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                "-keyout", str(self.key), "-out", str(self.cert), "-days", "1",
                "-subj", "/CN=localhost",
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        MockLcuHandler.session = {
            "localPlayerCellId": 0,
            "timer": {"phase": "BAN_PICK"},
            "myTeam": [{"cellId": 0, "championId": 0, "championPickIntent": 0}],
            "theirTeam": [{"cellId": 5, "championId": 0}],
            "bans": {"myTeamBans": [2], "theirTeamBans": []},
            "actions": [[{
                "id": 10,
                "actorCellId": 0,
                "type": "pick",
                "isInProgress": True,
                "completed": False,
                "championId": 1,
            }]],
            "trades": [],
            "pickOrderSwaps": [],
        }
        MockLcuHandler.write_attempts = 0
        MockLcuHandler.apply_write = True
        MockLcuHandler.slow_started.clear()
        MockLcuHandler.release_slow.clear()
        MockLcuHandler.slow_requests = 0
        MockLcuHandler.write_started.clear()
        MockLcuHandler.release_write.set()
        self.server = MockLcuServer(("127.0.0.1", 0), MockLcuHandler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(self.cert, self.key)
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.server_thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.server_thread.start()
        port = self.server.server_address[1]
        self.dummy = subprocess.Popen(
            [
                sys.executable, "-c", "import time; time.sleep(120)",
                f"--app-port={port}", "--remoting-auth-token=mock-lcu-token",
                "--rso_platform_id=HN1",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self.previous_appdata = os.environ.get("APPDATA")
        os.environ["APPDATA"] = self.directory.name
        self.client = self.make_client()

    def make_client(self):
        return SidecarClient(
            sidecar_path=str(Path("rust/target/release/seraphine-sidecar.exe").resolve())
        )

    async def restart_client(self):
        await self.client.close()
        self.client = self.make_client()
        await self.client.start()
        await asyncio.wait_for(self.client.lcu_connect(self.dummy.pid), timeout=10)

    async def asyncSetUp(self):
        await self.client.start()
        await asyncio.wait_for(self.client.lcu_connect(self.dummy.pid), timeout=10)
        await self.client.configure_automation("session-1", {})

    async def asyncTearDown(self):
        await self.client.close()
        self.dummy.terminate()
        try:
            self.dummy.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.dummy.kill()
            self.dummy.wait()
        self.server.shutdown()
        self.server.server_close()
        if self.previous_appdata is None:
            os.environ.pop("APPDATA", None)
        else:
            os.environ["APPDATA"] = self.previous_appdata
        self.directory.cleanup()

    async def test_http_write_error_is_confirmed_by_readback(self):
        args = {
            "method": "POST",
            "path": "/lol-champ-select/v1/session/actions/10",
            "data": {"type": "pick", "championId": 1, "completed": True},
            "operationId": "mock-operation-1",
        }
        result = await asyncio.wait_for(self.client.call("lcu.write", args), timeout=10)
        self.assertEqual(result["state"], "completed")
        self.assertTrue(result["confirmedAfterError"])

        replay = await self.client.call("lcu.write", args)
        self.assertTrue(replay["replayed"])
        diagnostics = await self.client.diagnostics()
        self.assertEqual(diagnostics["writes"]["ledgerEntries"], 1)
        self.assertEqual(diagnostics["writes"]["persistedGuards"], 1)
        self.assertEqual(MockLcuHandler.write_attempts, 1)
        guard_payload = (Path(self.directory.name) / "Seraphine" / "write-guards.json").read_text()
        self.assertNotIn("mock-operation-1", guard_payload)
        self.assertNotIn("session-1", guard_payload)

        changed = dict(args)
        changed["data"] = {"type": "pick", "championId": 2, "completed": True}
        with self.assertRaises(RpcError) as raised:
            await self.client.call("lcu.write", changed)
        self.assertEqual(raised.exception.code, "OperationConflict")
        self.assertEqual(MockLcuHandler.write_attempts, 1)

        await self.client.configure_automation("session-2", {})
        rescoped = await self.client.call("lcu.write", args)
        self.assertTrue(rescoped["replayed"])
        self.assertEqual(MockLcuHandler.write_attempts, 1)

        for invalid_path in [
            "/lol-champ-select/v1/session",
            "/lol-champ-select/v1/session/actions/10?completed=true",
            "/lol-champ-select/v1/session/actions/10/",
            "/lol-champ-select/v1/session/../session/actions/10",
        ]:
            invalid = dict(args)
            invalid["path"] = invalid_path
            invalid["operationId"] = "invalid-path-operation"
            with self.assertRaises(RpcError) as rejected:
                await self.client.call("lcu.write", invalid)
            self.assertEqual(rejected.exception.code, "InvalidParams")
        self.assertEqual(MockLcuHandler.write_attempts, 1)

        await self.restart_client()
        persisted_replay = await self.client.call("lcu.write", args)
        self.assertTrue(persisted_replay["replayed"])
        self.assertEqual(MockLcuHandler.write_attempts, 1)

    async def wait_for_slow_request(self):
        for _ in range(200):
            if MockLcuHandler.slow_started.is_set():
                return
            await asyncio.sleep(0.01)
        self.fail("slow LCU request did not start")

    async def test_read_requests_run_concurrently(self):
        slow = asyncio.create_task(
            self.client.call("lcu.get", {"path": "/slow"}, timeout=5)
        )
        await self.wait_for_slow_request()
        fast = await asyncio.wait_for(
            self.client.call("lcu.get", {"path": "/fast"}, timeout=1),
            timeout=1,
        )
        self.assertEqual(fast, {"result": "fast"})
        MockLcuHandler.release_slow.set()
        self.assertEqual(await slow, {"result": "slow"})

    async def test_read_concurrency_is_bounded(self):
        MockLcuHandler.release_slow.clear()
        MockLcuHandler.slow_requests = 0
        tasks = [
            asyncio.create_task(self.client.call("lcu.get", {"path": "/slow"}, timeout=5))
            for _ in range(8)
        ]
        try:
            for _ in range(300):
                if MockLcuHandler.slow_requests >= 2:
                    break
                await asyncio.sleep(0.01)
            self.assertGreaterEqual(MockLcuHandler.slow_requests, 2)
            self.assertLess(MockLcuHandler.slow_requests, 9)
            ninth = asyncio.create_task(
                self.client.call("lcu.get", {"path": "/slow"}, timeout=5)
            )
            await asyncio.sleep(0.1)
            self.assertFalse(ninth.done())
        finally:
            MockLcuHandler.release_slow.set()
            await asyncio.gather(*tasks, return_exceptions=True)
            if "ninth" in locals():
                await asyncio.gather(ninth, return_exceptions=True)

    async def test_pending_requests_wait_for_capacity(self):
        MockLcuHandler.release_slow.clear()
        MockLcuHandler.slow_requests = 0
        tasks = [
            asyncio.create_task(self.client.call("lcu.get", {"path": "/slow"}, timeout=10))
            for _ in range(64)
        ]
        waiting = None
        try:
            for _ in range(300):
                if len(self.client.pending) >= 64:
                    break
                await asyncio.sleep(0.01)
            self.assertEqual(len(self.client.pending), 64)
            waiting = asyncio.create_task(
                self.client.call("lcu.get", {"path": "/slow"}, timeout=10)
            )
            await asyncio.sleep(0.05)
            self.assertFalse(waiting.done())
        finally:
            MockLcuHandler.release_slow.set()
            await asyncio.gather(*tasks, return_exceptions=True)
            if waiting is not None:
                await asyncio.gather(waiting, return_exceptions=True)

    async def test_cancel_stops_a_slow_read(self):
        MockLcuHandler.release_slow.clear()
        pending = asyncio.create_task(
            self.client.call("lcu.get", {"path": "/slow"}, timeout=5)
        )
        await self.wait_for_slow_request()
        for _ in range(100):
            if self.client.pending:
                break
            await asyncio.sleep(0.01)
        request_id = next(iter(self.client.pending))
        await self.client._send(protocol.cancel(request_id))
        with self.assertRaises(RpcError) as raised:
            await pending
        self.assertEqual(raised.exception.code, "Canceled")
        MockLcuHandler.release_slow.set()

    async def test_deadline_stops_a_slow_read(self):
        MockLcuHandler.release_slow.clear()
        MockLcuHandler.slow_started.clear()
        request_id = self.client._next_id
        self.client._next_id = (request_id + 1) & 0x7FFFFFFF
        future = asyncio.get_running_loop().create_future()
        request = protocol.request(
            request_id,
            self.client.epoch,
            "lcu.get",
            {"path": "/slow"},
            deadline_ms=100,
        )
        size = self.client._message_size(request)
        self.client.pending[request_id] = (future, size, self.client.epoch)
        self.client._inflight_bytes += size
        await self.client._send(request)
        with self.assertRaises(RpcError) as raised:
            await asyncio.wait_for(future, timeout=1)
        self.assertEqual(raised.exception.code, "DeadlineExceeded")
        MockLcuHandler.release_slow.set()

    async def test_cancel_after_write_starts_does_not_abort_write(self):
        MockLcuHandler.release_write.clear()
        MockLcuHandler.write_started.clear()
        args = {
            "method": "POST",
            "path": "/lol-champ-select/v1/session/actions/10",
            "data": {"type": "pick", "championId": 1, "completed": True},
            "operationId": "mock-operation-canceled-write",
        }
        pending = asyncio.create_task(self.client.call("lcu.write", args, timeout=5))
        for _ in range(200):
            if MockLcuHandler.write_started.is_set():
                break
            await asyncio.sleep(0.01)
        self.assertTrue(MockLcuHandler.write_started.is_set())
        for _ in range(100):
            if self.client.pending:
                break
            await asyncio.sleep(0.01)
        request_id = next(iter(self.client.pending))
        await self.client._send(protocol.cancel(request_id))
        with self.assertRaises(RpcError) as raised:
            await pending
        self.assertEqual(raised.exception.code, "OutcomeUnknown")

        MockLcuHandler.release_write.set()
        await self.client.diagnostics()
        replay = await self.client.call("lcu.write", args)
        self.assertTrue(replay["replayed"])
        self.assertEqual(MockLcuHandler.write_attempts, 1)

    async def test_health_is_not_blocked_during_slow_write(self):
        MockLcuHandler.release_write.clear()
        MockLcuHandler.write_started.clear()
        args = {
            "method": "POST",
            "path": "/lol-champ-select/v1/session/actions/10",
            "data": {"type": "pick", "championId": 1, "completed": True},
            "operationId": "mock-operation-slow-health",
        }
        pending = asyncio.create_task(self.client.call("lcu.write", args, timeout=10))
        try:
            for _ in range(200):
                if MockLcuHandler.write_started.is_set():
                    break
                await asyncio.sleep(0.01)
            self.assertTrue(MockLcuHandler.write_started.is_set())
            health = await asyncio.wait_for(self.client.health(), timeout=0.5)
            self.assertEqual(health["protocol"], 1)
        finally:
            MockLcuHandler.release_write.set()
        result = await pending
        self.assertEqual(result["state"], "completed")

    async def test_unknown_outcome_blocks_retry(self):
        MockLcuHandler.apply_write = False
        args = {
            "method": "POST",
            "path": "/lol-champ-select/v1/session/actions/10",
            "data": {"type": "pick", "championId": 1, "completed": True},
            "operationId": "mock-operation-unknown",
        }
        with self.assertRaises(RpcError) as raised:
            await self.client.call("lcu.write", args)
        self.assertEqual(raised.exception.code, "OutcomeUnknown")
        with self.assertRaises(RpcError) as replayed:
            await self.client.call("lcu.write", args)
        self.assertEqual(replayed.exception.code, "OutcomeUnknown")
        self.assertEqual(MockLcuHandler.write_attempts, 1)

        await self.restart_client()
        with self.assertRaises(RpcError) as persisted:
            await self.client.call("lcu.write", args)
        self.assertEqual(persisted.exception.code, "OutcomeUnknown")
        self.assertEqual(MockLcuHandler.write_attempts, 1)


if __name__ == "__main__":
    unittest.main()
