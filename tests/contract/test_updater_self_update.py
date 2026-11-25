import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path


class UpdaterSelfUpdateTests(unittest.TestCase):
    def test_release_updater_replaces_itself_and_launches_app(self):
        root = Path(__file__).resolve().parents[2]
        updater = root / "rust" / "target" / "release" / "seraphine-updater.exe"
        if not updater.is_file():
            self.skipTest("release updater is not built")
        temporary = Path(tempfile.mkdtemp(prefix="seraphine-self-update-test-"))
        self.addCleanup(shutil.rmtree, temporary, ignore_errors=True)
        source = temporary / "source"
        target = temporary / "target"
        (source / "_internal").mkdir(parents=True)
        (target / "_internal").mkdir(parents=True)
        shutil.copy2(updater, target / "_internal" / "seraphine-updater.exe")
        shutil.copy2(updater, source / "Seraphine.exe")
        shutil.copy2(updater, source / "_internal" / "seraphine-updater.exe")
        files = []
        for path in source.rglob("*"):
            if path.is_file():
                content = path.read_bytes()
                files.append(
                    {
                        "path": path.relative_to(source).as_posix(),
                        "size": len(content),
                        "sha256": hashlib.sha256(content).hexdigest(),
                    }
                )
        (source / "update-manifest.json").write_text(
            json.dumps({"version": "test", "files": files}), encoding="utf-8"
        )
        status = temporary / "status.json"
        environment = os.environ.copy()
        environment["APPDATA"] = str(temporary / "appdata")
        environment["TEMP"] = str(temporary / "temp")
        environment["TMP"] = environment["TEMP"]
        subprocess.run(
            [
                str(target / "_internal" / "seraphine-updater.exe"),
                "--source",
                str(source),
                "--target",
                str(target),
                "--status",
                str(status),
            ],
            check=True,
            timeout=30,
            env=environment,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        for _ in range(100):
            if status.exists():
                result = json.loads(status.read_text(encoding="utf-8"))
                if result.get("status") == "completed":
                    break
            time.sleep(0.05)
        else:
            result = json.loads(status.read_text(encoding="utf-8")) if status.exists() else None
            self.fail("updater did not complete self-update: %s" % result)
        self.assertEqual(
            (target / "_internal" / "seraphine-updater.exe").read_bytes(),
            updater.read_bytes(),
        )
        self.assertFalse((target / "_internal" / "seraphine-updater.exe.new").exists())


if __name__ == "__main__":
    unittest.main()
