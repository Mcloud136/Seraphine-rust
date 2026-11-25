import sys
import tempfile
import types
import unittest
import shutil
from pathlib import Path
from unittest.mock import patch

from app.common.update import runUpdater


class UpdaterTests(unittest.TestCase):
    def test_launches_native_updater_without_shell(self):
        root = Path(tempfile.mkdtemp(prefix="seraphine-update-test-"))
        self.addCleanup(shutil.rmtree, root, ignore_errors=True)
        source = root / "temp"
        source.mkdir()
        (source / "update-manifest.json").write_text("{}", encoding="utf-8")
        updater = root / "seraphine-updater.exe"
        updater.write_bytes(b"updater")
        config = types.ModuleType("app.common.config")
        config.LOCAL_PATH = str(root)
        with patch.dict(sys.modules, {"app.common.config": config}), patch(
            "app.common.update.locate_sidecar", return_value=updater
        ), patch("app.common.update.subprocess.Popen") as popen, patch.dict(
            "os.environ", {"SERAPHINE_INSTALL_DIR": str(root / "install")}
        ):
            runUpdater()

        args, kwargs = popen.call_args
        self.assertEqual(args[0][0], str(updater))
        self.assertIn("--source", args[0])
        self.assertIn(str(source), args[0])
        self.assertNotIn("shell", kwargs)
        self.assertEqual(kwargs["stdin"], -3)


if __name__ == "__main__":
    unittest.main()
