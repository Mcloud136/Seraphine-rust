import argparse
import asyncio
import hashlib
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

from app.backend.sidecar_client import SidecarClient


def verify_manifest(root):
    manifest = json.loads((root / "update-manifest.json").read_text(encoding="utf-8"))
    files = manifest["files"]
    assert files, "manifest is empty"
    listed = set()
    for entry in files:
        relative = entry["path"]
        assert "\\" not in relative and not relative.startswith("/")
        assert ".." not in Path(relative).parts
        path = root / relative
        assert path.is_file(), path
        assert path.stat().st_size == entry["size"], path
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        assert digest == entry["sha256"], path
        listed.add(relative)
    actual = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file()
        and path.name not in {
            "update-manifest.json",
            "filelist.txt",
            "unins000.dat",
            "unins000.exe",
        }
        and "/app/resource/game/" not in path.relative_to(root).as_posix()
        and not path.relative_to(root).as_posix().startswith("ThirdPartyLicenses/")
    }
    assert actual == listed, (actual - listed, listed - actual)
    print(f"manifest verified: {len(files)} files")


async def verify_sidecar(root):
    client = SidecarClient(sidecar_path=str(root / "_internal" / "seraphine-sidecar.exe"))
    try:
        await client.start()
        health = await client.health()
        assert health["protocol"] == 1
        assert health["pid"]
        print("packaged sidecar handshake verified")
    finally:
        await client.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("--skip-ui", action="store_true")
    args = parser.parse_args()
    root = args.root.resolve()
    verify_manifest(root)

    version = subprocess.run(
        [str(root / "Seraphine.exe"), "--version"],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=20,
        check=True,
    )
    assert version.stdout.strip(), version
    print("packaged app version verified:", version.stdout.strip())

    asyncio.run(verify_sidecar(root))
    if args.skip_ui:
        return

    with tempfile.TemporaryDirectory(prefix="seraphine-smoke-") as appdata:
        env = dict(os.environ)
        env.pop("QT_QPA_PLATFORM", None)
        env["APPDATA"] = appdata
        process = subprocess.Popen(
            [str(root / "Seraphine.exe")],
            cwd=root,
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            deadline = time.monotonic() + 20
            initialized = False
            while time.monotonic() < deadline:
                assert process.poll() is None, "packaged UI exited early"
                log_directory = Path(appdata) / "Seraphine" / "log"
                for log_file in log_directory.glob("*.log"):
                    if "Seraphine initialized" in log_file.read_text(
                        encoding="utf-8", errors="replace"
                    ):
                        initialized = True
                        break
                if initialized:
                    break
                time.sleep(0.25)
            logs = "\n".join(
                log_file.read_text(encoding="utf-8", errors="replace")
                for log_file in log_directory.glob("*.log")
            )
            assert initialized, f"packaged UI did not finish startup:\n{logs[-8000:]}"
            print("packaged UI reached the main interface")
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


if __name__ == "__main__":
    main()
