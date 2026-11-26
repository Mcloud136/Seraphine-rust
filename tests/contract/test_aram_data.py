import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, patch

from app.backend import aram_data

ROOT = Path(__file__).resolve().parents[2]
DATA_PATH = ROOT / "app/resource/data/AramBuff.json"
MANIFEST_PATH = ROOT / "app/resource/data/aram-manifest.json"


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


class ValidationTests(unittest.TestCase):
    def test_bundled_snapshot_and_manifest_are_valid(self):
        data = load_json(DATA_PATH)
        manifest = load_json(MANIFEST_PATH)
        aram_data.validate_data(data)
        aram_data.validate_manifest(manifest)
        self.assertEqual(manifest["size"], DATA_PATH.stat().st_size)
        self.assertEqual(manifest["sha256"],
                         hashlib.sha256(DATA_PATH.read_bytes()).hexdigest())
        self.assertEqual(manifest["version"], data["version"])
        self.assertEqual(manifest["champions"], len(data["champions"]))

    def test_rejects_invalid_data(self):
        good = load_json(DATA_PATH)
        broken = dict(good, version="not-a-version")
        with self.assertRaises(ValueError):
            aram_data.validate_data(broken)
        short = dict(good, champions={"1": good["champions"]["1"]})
        with self.assertRaises(ValueError):
            aram_data.validate_data(short)
        entry_broken = json.loads(json.dumps(good))
        first = next(iter(entry_broken["champions"]))
        entry_broken["champions"][first]["zcsh"] = "95%"
        with self.assertRaises(ValueError):
            aram_data.validate_data(entry_broken)

    def test_rejects_invalid_manifest(self):
        good = load_json(MANIFEST_PATH)
        for patch in (
            {"schema": 2},
            {"file": "../AramBuff.json"},
            {"file": "AramBuff.txt"},
            {"size": 0},
            {"sha256": "zz"},
            {"champions": 1},
        ):
            broken = dict(good, **patch)
            with self.assertRaises(ValueError, msg=patch):
                aram_data.validate_manifest(broken)

    def test_verify_download_detects_tampering(self):
        manifest = load_json(MANIFEST_PATH)
        payload = DATA_PATH.read_bytes()
        data = aram_data.verify_download(manifest, payload)
        self.assertEqual(data["version"], manifest["version"])

        tampered = bytearray(payload)
        tampered[len(tampered) // 2] ^= 0x01
        with self.assertRaises(ValueError):
            aram_data.verify_download(manifest, bytes(tampered))
        with self.assertRaises(ValueError):
            aram_data.verify_download(dict(manifest, size=len(payload) + 1), payload)


class StateAndUrlTests(unittest.TestCase):
    def test_should_check_rules(self):
        now = 1_000_000.0
        self.assertTrue(aram_data.should_check({}, "16.23", now=now))
        self.assertTrue(aram_data.should_check({"lastAttemptAt": 0}, "16.23",
                                               now=now, force=True))
        self.assertFalse(aram_data.should_check({"lastAttemptAt": now - 100},
                                                "16.23", now=now))
        self.assertFalse(aram_data.should_check(
            {"lastAttemptAt": now - 700, "lastSuccessAt": now - 800,
             "dataGameVersion": "16.23"}, "16.23", now=now))
        self.assertTrue(aram_data.should_check(
            {"lastAttemptAt": now - 700, "lastSuccessAt": now - 800,
             "dataGameVersion": "16.22"}, "16.23", now=now))
        self.assertTrue(aram_data.should_check(
            {"lastAttemptAt": now - 700,
             "lastSuccessAt": now - aram_data.REFRESH_INTERVAL - 1,
             "dataGameVersion": "16.23"}, "16.23", now=now))
        self.assertFalse(aram_data.should_check(
            {"lastAttemptAt": now - 700, "lastSuccessAt": now - 800}, None, now=now))

    def test_url_allowlist(self):
        self.assertTrue(aram_data.is_allowed_url(
            "https://gitee.com/owner/repo/releases/download/tag/aram-manifest.json"))
        self.assertTrue(aram_data.is_allowed_url(
            "https://raw.githubusercontent.com/owner/repo/main/aram-manifest.json"))
        self.assertFalse(aram_data.is_allowed_url(
            "http://gitee.com/owner/repo/releases/download/tag/aram-manifest.json"))
        self.assertFalse(aram_data.is_allowed_url(
            "https://gitee.com.evil.example/aram-manifest.json"))


class CacheTests(unittest.TestCase):
    def test_load_data_seeds_cache_from_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            data = aram_data.load_data(root=directory)
            self.assertIsNotNone(data)
            cache = Path(directory) / "AramBuff.json"
            self.assertTrue(cache.is_file())
            self.assertEqual(load_json(cache)["version"], data["version"])
            leftovers = [item.name for item in Path(directory).iterdir()
                         if item.name.startswith(".tmp-aram-")]
            self.assertEqual(leftovers, [])

    def test_cache_takes_priority_over_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            bundled = load_json(DATA_PATH)
            cached = json.loads(json.dumps(bundled))
            cached["version"] = "99.99"
            cached["champions"]["1"]["catname"] = "CACHED"
            (Path(directory) / "AramBuff.json").write_text(
                json.dumps(cached, ensure_ascii=False), encoding="utf-8")
            data = aram_data.load_data(root=directory)
            self.assertEqual(data["version"], "99.99")
            self.assertEqual(data["champions"]["1"]["catname"], "CACHED")

    def test_invalid_cache_falls_back_to_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "AramBuff.json").write_text("{broken", encoding="utf-8")
            data = aram_data.load_data(root=directory)
            self.assertIsNotNone(data)
            self.assertEqual(data["version"], load_json(DATA_PATH)["version"])


@unittest.skipUnless(importlib.util.find_spec("aiohttp"), "aiohttp is required")
class FeedUpdateTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.bundled = load_json(DATA_PATH)
        self.url = "https://gitee.com/owner/repo/releases/download/aram-data-latest/aram-manifest.json"

    def manifest_for(self, data):
        payload = json.dumps(
            data, ensure_ascii=False, separators=(",", ":")
        ).encode("utf-8")
        manifest = {
            "schema": 1,
            "version": data["version"],
            "gameVersion": data.get("gameVersion"),
            "file": "AramBuff.json",
            "size": len(payload),
            "sha256": hashlib.sha256(payload).hexdigest(),
            "champions": len(data["champions"]),
        }
        return payload, json.dumps(manifest).encode("utf-8")

    async def test_success_downloads_validates_and_atomically_replaces_cache(self):
        updated = json.loads(json.dumps(self.bundled))
        updated["version"] = "99.99"
        updated["gameVersion"] = "99.99"
        payload, manifest = self.manifest_for(updated)
        fetch = AsyncMock(side_effect=[manifest, payload])
        with tempfile.TemporaryDirectory() as directory, patch.object(
            aram_data, "feed_url", return_value=self.url
        ), patch.object(aram_data, "_fetch", fetch):
            result = await aram_data.maybe_update(force=True, root=directory)
            self.assertTrue(result["updated"])
            self.assertEqual(load_json(Path(directory) / "AramBuff.json")["version"], "99.99")
            state = load_json(Path(directory) / "AramBuff.meta.json")
            self.assertEqual(state["dataVersion"], "99.99")

    async def test_tampered_download_keeps_previous_cache(self):
        updated = json.loads(json.dumps(self.bundled))
        updated["version"] = "99.99"
        payload, manifest = self.manifest_for(updated)
        tampered = bytearray(payload)
        tampered[len(tampered) // 2] ^= 1
        fetch = AsyncMock(side_effect=[manifest, bytes(tampered)])
        with tempfile.TemporaryDirectory() as directory, patch.object(
            aram_data, "feed_url", return_value=self.url
        ), patch.object(aram_data, "_fetch", fetch):
            result = await aram_data.maybe_update(force=True, root=directory)
            self.assertFalse(result["updated"])
            self.assertEqual(result["reason"], "error")
            self.assertEqual(load_json(Path(directory) / "AramBuff.json")["version"],
                             self.bundled["version"])

    async def test_http_failure_keeps_previous_cache(self):
        fetch = AsyncMock(side_effect=RuntimeError("HTTP 503"))
        with tempfile.TemporaryDirectory() as directory, patch.object(
            aram_data, "feed_url", return_value=self.url
        ), patch.object(aram_data, "_fetch", fetch):
            aram_data.load_data(root=directory)
            result = await aram_data.maybe_update(force=True, root=directory)
            self.assertFalse(result["updated"])
            self.assertIn("503", result["error"])
            self.assertEqual(load_json(Path(directory) / "AramBuff.json")["version"],
                             self.bundled["version"])

    async def test_same_version_does_not_download_data(self):
        _, manifest = self.manifest_for(self.bundled)
        fetch = AsyncMock(return_value=manifest)
        with tempfile.TemporaryDirectory() as directory, patch.object(
            aram_data, "feed_url", return_value=self.url
        ), patch.object(aram_data, "_fetch", fetch):
            result = await aram_data.maybe_update(force=True, root=directory)
        self.assertEqual(result["reason"], "up-to-date")
        self.assertEqual(fetch.await_count, 1)


class GeneratorTests(unittest.TestCase):
    def load_generator(self):
        spec = importlib.util.spec_from_file_location(
            "update_aram_data", ROOT / "scripts/update_aram_data.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_parse_lua_module_maps_aram_stats(self):
        generator = self.load_generator()
        script = ('return { ["Annie"] = { id = 1, title = "Annie", '
                  'stats = { aram = { dmg_dealt = 0.95, dmg_taken = 1.05, '
                  'shielding = 0.9 } } } }')
        champions = generator.build_from_module(
            generator.parse_lua_module(script), {})
        self.assertEqual(champions["1"]["zcsh"], "95")
        self.assertEqual(champions["1"]["sdsh"], "105")
        self.assertEqual(champions["1"]["hdxn"], "90")

    def test_auto_source_falls_back_to_bootstrap(self):
        generator = self.load_generator()
        bootstrap = {
            "_patch": "16.23.1",
            "_gamePatch": "26.23",
        }
        for champion_id in range(1, 131):
            bootstrap[str(champion_id)] = {
                "id": champion_id,
                "name": "Champion %d" % champion_id,
                "stats": {"aram": {"dmg_dealt": 0.95, "dmg_taken": 1.05}},
            }
        with tempfile.TemporaryDirectory() as directory:
            args = type("Args", (), {
                "source": "auto", "no_names": True, "names_url": "",
                "balance_json": None, "version": None,
                "output_dir": directory, "dry_run": False,
            })()
            with patch.object(generator, "fetch_wiki_script",
                              side_effect=RuntimeError("HTTP 403")), \
                    patch.object(generator, "fetch_bootstrap",
                                 return_value=bootstrap), \
                    patch.object(generator, "fetch_champion_names",
                                 return_value={}):
                manifest = generator.generate(args)
            data = json.loads((Path(directory) / "AramBuff.json")
                              .read_text(encoding="utf-8"))
        self.assertEqual(data["version"], "16.23.1")
        self.assertEqual(data["champions"]["1"]["zcsh"], "95")
        self.assertEqual(manifest["champions"], 130)
        self.assertIn("bootstrap via jsdelivr", manifest["source"])

    def test_explicit_wiki_source_does_not_fall_back(self):
        generator = self.load_generator()
        with tempfile.TemporaryDirectory() as directory:
            args = type("Args", (), {
                "source": "wiki", "no_names": True, "names_url": "",
                "balance_json": None, "version": None,
                "output_dir": directory, "dry_run": False,
            })()
            with patch.object(generator, "fetch_wiki_script",
                              side_effect=RuntimeError("HTTP 403")):
                with self.assertRaises(RuntimeError):
                    generator.generate(args)


if __name__ == "__main__":
    unittest.main()