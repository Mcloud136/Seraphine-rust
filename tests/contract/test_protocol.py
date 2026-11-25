import json
import struct
import unittest

from app.backend import protocol


class ProtocolTests(unittest.TestCase):
    def test_request_uses_rust_field_names(self):
        message = protocol.request(7, 3, "health", {}, 1000)
        self.assertEqual(message["version"], 1)
        self.assertEqual(message["type"], "request")
        self.assertEqual(message["deadline_ms"], 1000)
        self.assertNotIn("deadlineMs", message)

    def test_hello_uses_rust_field_names(self):
        message = protocol.hello(["health"], "1.3.0", epoch=9)
        self.assertEqual(message["app_version"], "1.3.0")
        self.assertEqual(message["epoch"], 9)
        self.assertNotIn("appVersion", message)

    def test_rpc_error_round_trip(self):
        error = protocol.RpcError("MethodNotFound", "missing", True, "lcu")
        restored = protocol.RpcError.from_dict(error.to_dict())
        self.assertEqual(restored.code, error.code)
        self.assertEqual(restored.message, error.message)
        self.assertTrue(restored.retryable)
        self.assertEqual(restored.source, error.source)

    def test_frame_length_is_little_endian(self):
        payload = json.dumps(protocol.ping(42), separators=(",", ":")).encode()
        frame = struct.pack("<I", len(payload)) + payload
        self.assertEqual(struct.unpack("<I", frame[:4])[0], len(payload))
        self.assertEqual(json.loads(frame[4:]), protocol.ping(42))


if __name__ == "__main__":
    unittest.main()
