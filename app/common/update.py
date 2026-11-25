import os
import subprocess
import sys
from pathlib import Path

from app.backend.supervisor import locate_sidecar


def runUpdater():
    from app.common.config import LOCAL_PATH

    source = Path(LOCAL_PATH) / "temp"
    manifest = source / "update-manifest.json"
    if not manifest.is_file():
        raise FileNotFoundError(str(manifest))

    target = Path(
        os.environ.get("SERAPHINE_INSTALL_DIR", Path(sys.executable).resolve().parent)
    )
    status = Path(LOCAL_PATH) / "update-status.json"
    updater = locate_sidecar().with_name("seraphine-updater.exe")
    if not updater.is_file():
        raise FileNotFoundError(str(updater))

    subprocess.Popen(
        [
            str(updater),
            "--source",
            str(source),
            "--target",
            str(target),
            "--status",
            str(status),
        ],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        close_fds=True,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
