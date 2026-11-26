"""ARAM balance data: local loading + optional feed updates.

Data layers (read priority):
  1. cache   %APPDATA%/Seraphine/AramBuff.json  (updated by the feed, atomic)
  2. bundled app/resource/data/AramBuff.json    (shipped, read-only)

The feed is a small HTTPS manifest (see scripts/update_aram_data.py) published
by CI (Gitee/GitHub release asset or raw file). Updates never block startup:
callers load local data first, then optionally call `maybe_update`.

Manifest:
  {"schema":1,"version":"16.23.1","gameVersion":"16.23",
   "file":"AramBuff.json","size":21882,"sha256":"...","champions":135}
"""

import hashlib
import json
import os
import re
import tempfile
import time
from pathlib import Path
from urllib.parse import urljoin, urlparse

SCHEMA_VERSION = 1
MIN_CHAMPIONS = 100
MAX_MANIFEST_BYTES = 64 * 1024
MAX_DATA_BYTES = 2 * 1024 * 1024
MIN_ATTEMPT_INTERVAL = 600.0
REFRESH_INTERVAL = 24 * 3600.0
ALLOWED_HOSTS = (
    "gitee.com",
    "github.com",
    "raw.githubusercontent.com",
    "objects.githubusercontent.com",
)
USER_AGENT = "Seraphine-ARAM-data/1"
VERSION_RE = re.compile(r"^\d+\.\d+(\.\d+)?$")
SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")
INT_FIELDS = ("zcsh", "sdsh", "zlxl", "hdxn", "jnjs", "renxing")


def _log(level, message):
    try:
        from app.common.logger import logger
        getattr(logger, level)(message, "AramData")
    except Exception:
        pass


def _local_root():
    try:
        from app.common.config import LOCAL_PATH
        return Path(LOCAL_PATH)
    except Exception:
        return Path(os.getenv("APPDATA") or tempfile.gettempdir()) / "Seraphine"


def bundled_path():
    return Path(__file__).resolve().parents[1] / "resource" / "data" / "AramBuff.json"


def cache_path(root=None):
    return (Path(root) if root is not None else _local_root()) / "AramBuff.json"


def state_path(root=None):
    return (Path(root) if root is not None else _local_root()) / "AramBuff.meta.json"


def validate_data(data):
    if not isinstance(data, dict):
        raise ValueError("ARAM data is not an object")
    version = data.get("version")
    if not isinstance(version, str) or not VERSION_RE.match(version):
        raise ValueError("invalid ARAM data version: %r" % (version,))
    champions = data.get("champions")
    if not isinstance(champions, dict) or len(champions) < MIN_CHAMPIONS:
        raise ValueError("ARAM data has too few champions")
    for champion_id, entry in champions.items():
        if not str(champion_id).isdigit() or not isinstance(entry, dict):
            raise ValueError("invalid ARAM champion entry: %r" % (champion_id,))
        catname = entry.get("catname")
        if not isinstance(catname, str) or not catname:
            raise ValueError("ARAM champion %s has no catname" % champion_id)
        for key in INT_FIELDS:
            value = entry.get(key)
            if not isinstance(value, str):
                raise ValueError("ARAM champion %s field %s is not a string"
                                 % (champion_id, key))
            int(value)
        if not isinstance(entry.get("description"), str):
            raise ValueError("ARAM champion %s has no description" % champion_id)


def validate_manifest(manifest):
    if not isinstance(manifest, dict):
        raise ValueError("manifest is not an object")
    if manifest.get("schema") != SCHEMA_VERSION:
        raise ValueError("unsupported manifest schema")
    version = manifest.get("version")
    if not isinstance(version, str) or not VERSION_RE.match(version):
        raise ValueError("invalid manifest version")
    file_name = manifest.get("file")
    if (not isinstance(file_name, str) or Path(file_name).name != file_name
            or not file_name.lower().endswith(".json")):
        raise ValueError("invalid manifest file name")
    size = manifest.get("size")
    if not isinstance(size, int) or not 0 < size <= MAX_DATA_BYTES:
        raise ValueError("invalid manifest size")
    digest = manifest.get("sha256")
    if not isinstance(digest, str) or not SHA256_RE.match(digest):
        raise ValueError("invalid manifest sha256")
    champions = manifest.get("champions")
    if not isinstance(champions, int) or champions < MIN_CHAMPIONS:
        raise ValueError("invalid manifest champion count")


def _atomic_write(path, payload):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(prefix=".tmp-aram-", dir=str(path.parent))
    try:
        with os.fdopen(handle, "wb") as stream:
            stream.write(payload)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def load_data(root=None):
    """Return validated ARAM data, preferring cache over the bundled snapshot."""
    cache = cache_path(root)
    for index, path in enumerate((cache, bundled_path())):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
            validate_data(data)
        except FileNotFoundError:
            continue
        except Exception as error:
            _log("warning", "invalid ARAM data at %s: %s" % (path, error))
            continue
        if index == 1:
            try:
                _atomic_write(cache, path.read_bytes())
            except Exception as error:
                _log("warning", "failed to seed ARAM cache: %s" % error)
        return data
    return None


def _read_state(root=None):
    try:
        state = json.loads(state_path(root).read_text(encoding="utf-8"))
        return state if isinstance(state, dict) else {}
    except Exception:
        return {}


def _write_state(root=None, **values):
    state = _read_state(root)
    state.update(values)
    try:
        _atomic_write(state_path(root),
                      (json.dumps(state, indent=2) + "\n").encode("utf-8"))
    except Exception as error:
        _log("warning", "failed to persist ARAM update state: %s" % error)


def auto_update_enabled():
    override = os.environ.get("SERAPHINE_ARAM_AUTO_UPDATE")
    if override is not None:
        return override.strip().lower() not in ("0", "false", "off", "no")
    try:
        from app.common.config import cfg
        return bool(cfg.get(cfg.aramDataAutoUpdate))
    except Exception:
        return True


def feed_url():
    url = os.environ.get("SERAPHINE_ARAM_DATA_URL", "").strip()
    if url:
        return url
    try:
        from app.common.config import cfg
        return str(cfg.get(cfg.aramDataFeedUrl) or "").strip()
    except Exception:
        return ""


def is_allowed_url(url):
    parsed = urlparse(url)
    if parsed.scheme != "https" or not parsed.hostname:
        return False
    host = parsed.hostname.lower()
    return any(host == allowed or host.endswith("." + allowed)
               for allowed in ALLOWED_HOSTS)


def _normalized_version(value):
    if not value:
        return ()
    return tuple(str(value).split(".")[:2])


def should_check(state, game_version, now=None, force=False):
    if force:
        return True
    now = time.time() if now is None else now
    last_attempt = float(state.get("lastAttemptAt") or 0)
    if now - last_attempt < MIN_ATTEMPT_INTERVAL:
        return False
    last_success = float(state.get("lastSuccessAt") or 0)
    if now - last_success < REFRESH_INTERVAL:
        local_game = state.get("dataGameVersion")
        if not game_version or not local_game:
            return False
        return _normalized_version(game_version) != _normalized_version(local_game)
    return True


def verify_download(manifest, payload):
    if len(payload) != int(manifest["size"]):
        raise ValueError("ARAM data size mismatch")
    digest = hashlib.sha256(payload).hexdigest()
    if digest.lower() != str(manifest["sha256"]).lower():
        raise ValueError("ARAM data checksum mismatch")
    data = json.loads(payload.decode("utf-8"))
    validate_data(data)
    if str(data.get("version")) != str(manifest["version"]):
        raise ValueError("ARAM data version mismatch")
    return data


async def _fetch(session, url, limit):
    async with session.get(url) as response:
        if response.status != 200:
            raise RuntimeError("HTTP %d for %s" % (response.status, url))
        body = bytearray()
        async for chunk in response.content.iter_chunked(16384):
            body.extend(chunk)
            if len(body) > limit:
                raise RuntimeError("response from %s exceeds %d bytes" % (url, limit))
        return bytes(body)


async def maybe_update(game_version=None, force=False, root=None):
    """Check the feed and update the cache. Never raises."""
    if not force and not auto_update_enabled():
        return {"updated": False, "reason": "disabled"}

    url = feed_url()
    if not url:
        return {"updated": False, "reason": "not-configured"}
    if not is_allowed_url(url):
        return {"updated": False, "reason": "invalid-feed-url"}

    state = _read_state(root)
    if not should_check(state, game_version, force=force):
        return {"updated": False, "reason": "throttled"}

    try:
        import aiohttp

        timeout = aiohttp.ClientTimeout(total=25)
        headers = {"User-Agent": USER_AGENT, "Accept": "application/json"}
        async with aiohttp.ClientSession(timeout=timeout, headers=headers) as session:
            manifest = json.loads(
                (await _fetch(session, url, MAX_MANIFEST_BYTES)).decode("utf-8"))
            validate_manifest(manifest)

            local = load_data(root) or {}
            if str(manifest["version"]) == str(local.get("version")):
                now = time.time()
                _write_state(root, lastAttemptAt=now, lastSuccessAt=now,
                             dataVersion=manifest["version"],
                             dataGameVersion=manifest.get("gameVersion"))
                return {"updated": False, "reason": "up-to-date",
                        "version": local.get("version")}

            data_url = urljoin(url, manifest["file"])
            if not is_allowed_url(data_url):
                raise RuntimeError("data host is not allowed")
            payload = await _fetch(session, data_url, MAX_DATA_BYTES)
            data = verify_download(manifest, payload)
    except Exception as error:
        _write_state(root, lastAttemptAt=time.time())
        _log("warning", "ARAM data update failed: %s" % error)
        return {"updated": False, "reason": "error", "error": str(error)}

    try:
        _atomic_write(cache_path(root), payload)
    except Exception as error:
        _write_state(root, lastAttemptAt=time.time())
        return {"updated": False, "reason": "error", "error": str(error)}

    now = time.time()
    _write_state(root, lastAttemptAt=now, lastSuccessAt=now,
                 dataVersion=data["version"],
                 dataGameVersion=data.get("gameVersion"))
    _log("info", "ARAM data updated to %s" % data["version"])
    return {"updated": True, "data": data, "version": data["version"]}