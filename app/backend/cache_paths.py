import os
import shutil
from pathlib import Path


def get_legacy_cache_root(project_root=None):
    root = Path(project_root or os.getcwd())
    return root / "app" / "resource" / "game"


def get_cache_root(project_root=None):
    override = os.environ.get("SERAPHINE_CACHE_DIR")
    if override:
        return Path(override).expanduser().resolve()
    local = Path(os.environ.get("LOCALAPPDATA", str(Path.home() / "AppData" / "Local")))
    return local / "Seraphine" / "Cache"


def ensure_cache_root(project_root=None, migrate_legacy=True):
    cache_root = get_cache_root(project_root)
    cache_root.mkdir(parents=True, exist_ok=True)
    if migrate_legacy:
        legacy = get_legacy_cache_root(project_root)
        if legacy.is_dir() and legacy.resolve() != cache_root.resolve():
            for source in legacy.rglob("*"):
                if not source.is_file():
                    continue
                relative = source.relative_to(legacy)
                destination = cache_root / relative
                if destination.exists():
                    continue
                destination.parent.mkdir(parents=True, exist_ok=True)
                try:
                    shutil.copy2(source, destination)
                except OSError:
                    continue
    return cache_root
