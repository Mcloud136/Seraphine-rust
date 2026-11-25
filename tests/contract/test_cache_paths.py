import tempfile
import unittest
from pathlib import Path
from unittest import mock

from app.backend.cache_paths import ensure_cache_root, get_cache_root


class CachePathTests(unittest.TestCase):
    def test_migrates_legacy_cache_without_overwriting(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = root / "app" / "resource" / "game" / "champion icons"
            legacy.mkdir(parents=True)
            (legacy / "1.png").write_bytes(b"legacy")
            cache = root / "cache"
            cache.mkdir()
            (cache / "champion icons").mkdir()
            (cache / "champion icons" / "1.png").write_bytes(b"new")

            with mock.patch.dict(
                "os.environ", {"SERAPHINE_CACHE_DIR": str(cache)}
            ):
                result = ensure_cache_root(root)

            self.assertEqual(result, cache)
            self.assertEqual(
                (cache / "champion icons" / "1.png").read_bytes(),
                b"new",
            )


if __name__ == "__main__":
    unittest.main()
