import asyncio
import json
import os
import struct
import subprocess
import time
import uuid
from collections import deque

from . import protocol
from .supervisor import JobObject, locate_sidecar


class SidecarClient:
    def __init__(
        self,
        app_version="1.3.0",
        sidecar_path=None,
        event_queue_size=None,
        heartbeat_interval=15.0,
        heartbeat_timeout=30.0,
    ):
        self.app_version = app_version
        self.sidecar_path = sidecar_path
        self.event_queue_size = event_queue_size or protocol.MAX_EVENT_QUEUE
        self.heartbeat_interval = heartbeat_interval
        self.heartbeat_timeout = heartbeat_timeout
        self._lifecycle_lock = asyncio.Lock()
        self._ready = asyncio.Event()
        self._ready.set()
        self._reconnecting = None
        self._generation = 0
        self._connection_params = None
        self._close_task = None
        self.restart_count = 0
        self.process = None
        self.job = None
        self.stdin = None
        self.stdout = None
        self.stderr = None
        self.reader_task = None
        self.heartbeat_task = None
        self.stderr_task = None
        self.write_lock = asyncio.Lock()
        self.pending = {}
        self._pending_slots = asyncio.Semaphore(protocol.MAX_PENDING_REQUESTS)
        self.events = asyncio.Queue(maxsize=self.event_queue_size)
        self.stderr_lines = deque(maxlen=64)
        self.closed = True
        self.epoch = 0
        self.capabilities = []
        self.pid = None
        self._next_id = 1
        self._last_pong = 0.0
        self._inflight_bytes = 0
        self._event_bytes = 0
        self.dropped_events = 0

    async def start(self):
        if self._close_task is not None and not self._close_task.done():
            await asyncio.shield(self._close_task)
        current_task = asyncio.current_task()
        if (
            not self.closed
            and self._reconnecting is not None
            and self._reconnecting is not current_task
        ):
            await self._ready.wait()
            return self
        wait_for_ready = False
        started = False
        async with self._lifecycle_lock:
            if not self.closed:
                wait_for_ready = (
                    self._reconnecting is not None
                    and self._reconnecting is not current_task
                )
            else:
                if self.process is not None:
                    await self._close_locked(forget=False)
                    self.restart_count += 1
                await self._start_locked()
                started = True
                self._generation += 1
                generation = self._generation
                self._reconnecting = current_task
                self._ready.clear()

        if wait_for_ready:
            await self._ready.wait()
            return self
        if not started:
            return self

        try:
            if self._connection_params is not None:
                connection = await self.call(
                    "lcu.connect", self._connection_params, timeout=60.0
                )
                self.epoch = connection.get("epoch", 0)
        except BaseException:
            await self.close()
            raise
        finally:
            if self._generation == generation:
                self._reconnecting = None
                self._ready.set()
        return self

    async def _start_locked(self):
        path = self.sidecar_path or locate_sidecar()
        if not os.path.isfile(path):
            raise FileNotFoundError(str(path))

        creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
        self.job = JobObject()
        self.process = await asyncio.create_subprocess_exec(
            str(path),
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            creationflags=creationflags,
        )
        self._assign_job()
        self.stdin = self.process.stdin
        self.stdout = self.process.stdout
        self.stderr = self.process.stderr
        self.closed = False
        self.stderr_task = asyncio.create_task(self._stderr_loop())

        try:
            hello = await asyncio.wait_for(self._read_message(), timeout=5.0)
            self._validate_message(hello)
            if hello.get("type") != protocol.HELLO:
                raise protocol.ProtocolError("sidecar did not send hello")
            self.capabilities = list(hello.get("capabilities", []))
            self.pid = hello.get("data", {}).get("pid") or self.process.pid
            await self._send(
                protocol.hello(
                    [
                        protocol.CAPABILITY_HEALTH,
                        protocol.CAPABILITY_PROCESS_LIST,
                        protocol.CAPABILITY_DIAGNOSTICS,
                        protocol.CAPABILITY_CHAMPION_ALIAS,
                        protocol.CAPABILITY_ARAM_BUFF,
                    ],
                    self.app_version,
                    self.epoch,
                )
            )
            acknowledgement = await asyncio.wait_for(self._read_message(), timeout=5.0)
            if acknowledgement.get("type") != protocol.HELLO:
                raise protocol.ProtocolError("sidecar did not acknowledge hello")
            self.epoch = max(self.epoch, acknowledgement.get("epoch") or 0)
            self._last_pong = time.monotonic()
        except BaseException as error:
            await asyncio.sleep(0.05)
            returncode = self.process.returncode if self.process else None
            stderr = list(self.stderr_lines)
            await self._close_locked(forget=False)
            raise protocol.ProtocolError(
                "sidecar handshake failed: %s; returncode=%s; stderr=%s"
                % (error, returncode, stderr)
            ) from error

        self.reader_task = asyncio.create_task(self._reader_loop())
        self.heartbeat_task = asyncio.create_task(self._heartbeat_loop())

    def _assign_job(self):
        if not self.job or not self.job.handle:
            return
        try:
            transport = self.process._transport
            subprocess_handle = transport.get_extra_info("subprocess").handle
            self.job.assign(int(subprocess_handle))
            self.job.set_below_normal(int(subprocess_handle))
        except (AttributeError, TypeError, OSError):
            pass

    async def _read_exactly(self, size):
        data = bytearray()
        while len(data) < size:
            chunk = await self.stdout.readexactly(size - len(data))
            if not chunk:
                raise asyncio.IncompleteReadError(bytes(data), size)
            data.extend(chunk)
        return bytes(data)

    async def _read_message(self):
        prefix = await self._read_exactly(4)
        length = struct.unpack("<I", prefix)[0]
        if length <= 0 or length > protocol.MAX_FRAME_SIZE:
            raise protocol.ProtocolError("invalid frame length: %s" % length)
        payload = await self._read_exactly(length)
        try:
            message = json.loads(payload.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise protocol.ProtocolError(str(error))
        self._validate_message(message)
        kind = message.get("type")
        if kind == protocol.RESPONSE and length > protocol.MAX_RESPONSE_BYTES:
            raise protocol.ProtocolError("response is too large")
        if kind == protocol.EVENT and length > protocol.MAX_EVENT_BYTES:
            raise protocol.ProtocolError("event is too large")
        return message

    @staticmethod
    def _validate_message(message):
        if not isinstance(message, dict):
            raise protocol.ProtocolError("message must be an object")
        if message.get("version") != protocol.PROTOCOL_VERSION:
            raise protocol.ProtocolError(
                "unsupported protocol version: %s" % message.get("version")
            )
        if message.get("type") not in {
            protocol.HELLO,
            protocol.REQUEST,
            protocol.RESPONSE,
            protocol.EVENT,
            protocol.CANCEL,
            protocol.PING,
            protocol.PONG,
            protocol.SHUTDOWN,
        }:
            raise protocol.ProtocolError("unknown message type")

    @staticmethod
    def _message_size(message):
        return len(
            json.dumps(message, ensure_ascii=False, separators=(",", ":")).encode(
                "utf-8"
            )
        )

    async def _send(self, message):
        if self.closed or self.stdin is None:
            raise protocol.ProtocolError("sidecar is not connected")
        self._validate_message(message)
        payload = json.dumps(
            message,
            ensure_ascii=False,
            separators=(",", ":"),
        ).encode("utf-8")
        if len(payload) > protocol.MAX_FRAME_SIZE:
            raise protocol.ProtocolError("frame is too large")
        if message.get("type") == protocol.REQUEST and len(payload) > protocol.MAX_REQUEST_BYTES:
            raise protocol.ProtocolError("request is too large")
        frame = struct.pack("<I", len(payload)) + payload
        async with self.write_lock:
            self.stdin.write(frame)
            await self.stdin.drain()

    def _take_pending(self, request_id):
        pending = self.pending.pop(request_id, None)
        if pending is None:
            return None
        future, size, expected_epoch = pending
        self._inflight_bytes = max(0, self._inflight_bytes - size)
        return future, expected_epoch

    async def call(self, op, args=None, timeout=15.0):
        await self.start()
        return await asyncio.wait_for(self._call(op, args, timeout), timeout=timeout)

    async def _cancel_request(self, request_id):
        try:
            await asyncio.wait_for(
                self._send(protocol.cancel(request_id)), timeout=0.5
            )
        except (asyncio.CancelledError, Exception):
            self.closed = True
            self._fail_pending(protocol.ProtocolError("failed to cancel sidecar request"))
            self._schedule_close()

    async def _call(self, op, args=None, timeout=15.0):
        async with self._pending_slots:
            if len(self.pending) >= protocol.MAX_PENDING_REQUESTS:
                raise protocol.ProtocolError("too many pending requests")
            request_id = self._next_id
            self._next_id = (self._next_id + 1) & 0x7FFFFFFF
            future = asyncio.get_running_loop().create_future()
            deadline_ms = max(1, int(timeout * 1000))
            request_message = protocol.request(
                request_id,
                self.epoch,
                op,
                {} if args is None else args,
                deadline_ms,
            )
            request_size = self._message_size(request_message)
            if request_size > protocol.MAX_REQUEST_BYTES:
                raise protocol.ProtocolError("request is too large")
            if self._inflight_bytes + request_size > protocol.MAX_IPC_INFLIGHT_BYTES:
                raise protocol.ProtocolError("IPC in-flight byte budget exceeded")
            self.pending[request_id] = (future, request_size, request_message["epoch"])
            self._inflight_bytes += request_size
            try:
                await self._send(request_message)
                return await future
            except asyncio.CancelledError:
                await asyncio.shield(self._cancel_request(request_id))
                raise
            finally:
                self._take_pending(request_id)

    async def health(self):
        return await self.call("health")

    async def diagnostics(self):
        return await self.call("diagnostics")

    async def process_list(self):
        return await self.call("process.list")

    async def process_command_line(self, pid):
        return await self.call("process.command_line", {"pid": pid})

    async def set_game_state(self, state):
        return await self.call("scheduler.set_game_state", {"state": state})

    async def lcu_connect(self, pid, proxy=None, cache_root=None):
        args = {"pid": pid}
        if proxy:
            args["proxy"] = proxy
        if cache_root:
            args["cacheRoot"] = str(cache_root)
        connection = await self.call("lcu.connect", args, timeout=60.0)
        self._connection_params = args
        self.epoch = connection.get("epoch", self.epoch)
        return connection

    async def lcu_get(self, path):
        return await self.call("lcu.get", {"path": path})

    async def lcu_summoner_by_pid(self, pid):
        return await self.call(
            "lcu.summoner_by_pid", {"pid": pid}, timeout=10.0
        )

    async def lcu_write(self, method, path, data=None, operation_id=None):
        args = {
            "method": method,
            "path": path,
            "operationId": operation_id or uuid.uuid4().hex,
        }
        if data is not None:
            args["data"] = data
        result = await self.call("lcu.write", args, timeout=10.0)
        return result.get("data")

    async def lcu_catalog(self, force=False):
        return await self.call(
            "lcu.catalog", {"force": force}, timeout=60.0
        )

    async def sgp_get(self, path, params=None):
        return await self.call(
            "sgp.get",
            {"path": path, "params": params or {}},
            timeout=20.0,
        )

    async def asset_get(
        self,
        folder,
        name,
        path=None,
        url=None,
        max_bytes=2 * 1024 * 1024,
        force=True,
    ):
        args = {
            "folder": folder,
            "name": name,
            "maxBytes": max_bytes,
            "force": force,
        }
        if path is not None:
            args["path"] = path
        if url is not None:
            args["url"] = url
        result = await self.call("asset.get", args, timeout=30.0)
        return result["path"]

    async def external_get(self, url):
        return await self.call("external.get", {"url": url}, timeout=20.0)

    async def configure_automation(self, session_id, config, queue_id=None):
        args = {"sessionId": session_id, "config": config}
        if queue_id is not None:
            args["queueId"] = queue_id
        return await self.call("automation.configure", args)

    async def automation_dry_run(self, event):
        return await self.call("automation.dry_run", event)

    async def opgg_positions(self, region, champion_id, tier, mode="ranked"):
        return await self.call(
            "opgg.positions",
            {
                "region": region,
                "mode": mode,
                "tier": tier,
                "championId": champion_id,
            },
            timeout=30.0,
        )

    async def opgg_tier_list(self, region, mode, tier):
        return await self.call(
            "opgg.tier_list",
            {"region": region, "mode": mode, "tier": tier},
            timeout=60.0,
        )

    async def opgg_champion_build(
        self, region, mode, champion_id, position, tier
    ):
        return await self.call(
            "opgg.champion_build",
            {
                "region": region,
                "mode": mode,
                "tier": tier,
                "championId": champion_id,
                "position": position,
            },
            timeout=60.0,
        )

    async def champion_alias_refresh(self, game_version=None, force=False):
        args = {"force": force}
        if game_version:
            args["gameVersion"] = game_version
        return await self.call("champion_alias.refresh", args, timeout=30.0)

    async def champion_alias_search(self, query):
        return await self.call(
            "champion_alias.search", {"query": query}, timeout=5.0
        )

    async def aram_buff_refresh(self, game_version=None, force=False):
        args = {"force": force}
        if game_version:
            args["gameVersion"] = game_version
        return await self.call("aram_buff.refresh", args, timeout=30.0)

    async def aram_buff_get(self, champion_id):
        return await self.call(
            "aram_buff.get", {"championId": champion_id}, timeout=5.0
        )

    async def events_stream(self):
        while True:
            event = await self.events.get()
            self._event_bytes = max(0, self._event_bytes - self._message_size(event))
            yield event

    def _schedule_close(self):
        if self._close_task is not None and not self._close_task.done():
            return

        async def close_when_ready():
            try:
                await self.close()
            finally:
                self._close_task = None

        self._close_task = asyncio.create_task(close_when_ready())

    def _resolve_response(self, request_id, message):
        pending = self._take_pending(request_id)
        if pending is None:
            return
        future, expected_epoch = pending
        if future.done():
            return
        if message.get("epoch") != expected_epoch:
            future.set_exception(protocol.RpcError(
                "InvalidParams", "stale response epoch", retryable=True
            ))
            return
        if message.get("error") is not None:
            future.set_exception(protocol.RpcError.from_dict(message["error"]))
        else:
            future.set_result(message.get("data"))

    async def _reader_loop(self):
        try:
            while True:
                message = await self._read_message()
                kind = message.get("type")
                if kind == protocol.RESPONSE:
                    self._resolve_response(message.get("id"), message)
                elif kind == protocol.EVENT:
                    event_size = self._message_size(message)
                    if (
                        self.events.full()
                        or self._event_bytes + event_size > protocol.MAX_EVENT_QUEUE_BYTES
                    ):
                        self.dropped_events += 1
                        continue
                    self.events.put_nowait(message)
                    self._event_bytes += event_size
                elif kind == protocol.PING:
                    await self._send(protocol.pong(message.get("nonce", 0)))
                elif kind == protocol.PONG:
                    self._last_pong = time.monotonic()
        except (asyncio.IncompleteReadError, ConnectionError, protocol.ProtocolError):
            self.closed = True
            self._fail_pending(
                protocol.ProtocolError("sidecar connection closed")
            )
            self._schedule_close()
        except asyncio.CancelledError:
            raise
        except BaseException as error:
            self.closed = True
            self._fail_pending(error)
            self._schedule_close()

    async def _heartbeat_loop(self):
        try:
            while not self.closed:
                await asyncio.sleep(self.heartbeat_interval)
                if time.monotonic() - self._last_pong >= self.heartbeat_timeout:
                    self.closed = True
                    self._fail_pending(
                        protocol.ProtocolError("sidecar heartbeat timed out")
                    )
                    self._schedule_close()
                    return
                await self._send(protocol.ping(int(time.monotonic() * 1000)))
        except asyncio.CancelledError:
            raise
        except (protocol.ProtocolError, ConnectionError):
            self._schedule_close()

    async def _stderr_loop(self):
        try:
            while True:
                line = await self.stderr.readline()
                if not line:
                    return
                self.stderr_lines.append(line.decode("utf-8", errors="replace").rstrip())
        except asyncio.CancelledError:
            return

    def _fail_pending(self, error):
        while self.pending:
            _, pending = self.pending.popitem()
            future, _, _ = pending
            if not future.done():
                future.set_exception(error)
        self._inflight_bytes = 0

    async def close(self, forget=False):
        async with self._lifecycle_lock:
            await self._close_locked(forget=forget)

    async def _close_locked(self, forget=False):
        if self.stdin is not None and self.process is not None:
            self.closed = False
            try:
                await self._send(protocol.shutdown())
            except (protocol.ProtocolError, ConnectionError, BrokenPipeError):
                pass
        self.closed = True
        if self.stdin is not None and not self.stdin.is_closing():
            self.stdin.close()
            try:
                await self.stdin.wait_closed()
            except (ConnectionError, BrokenPipeError):
                pass
        if self.process is not None:
            try:
                await asyncio.wait_for(self.process.wait(), timeout=2.0)
            except asyncio.TimeoutError:
                self.process.terminate()
                try:
                    await asyncio.wait_for(self.process.wait(), timeout=2.0)
                except asyncio.TimeoutError:
                    self.process.kill()
                    await self.process.wait()
            transport = getattr(self.process, "_transport", None)
            if transport is not None:
                transport.close()
            await asyncio.sleep(0)
        current_task = asyncio.current_task()
        tasks = [
            task
            for task in (self.reader_task, self.heartbeat_task, self.stderr_task)
            if task is not None and task is not current_task
        ]
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        self._fail_pending(protocol.ProtocolError("sidecar closed"))
        while not self.events.empty():
            self.events.get_nowait()
        self._event_bytes = 0
        if self.job is not None:
            self.job.close()
            self.job = None
        self.process = None
        self.stdin = None
        self.stdout = None
        self.stderr = None
        self.reader_task = None
        self.heartbeat_task = None
        self.stderr_task = None
        self.capabilities = []
        self.pid = None
        self._last_pong = 0.0
        self._generation += 1
        self._reconnecting = None
        self._ready.set()
        if forget:
            self._connection_params = None
            self.epoch = 0
