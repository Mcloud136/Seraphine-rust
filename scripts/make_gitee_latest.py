"""Rewrite an updater manifest so it downloads the installer from Gitee.

The signature and version stay unchanged; only the platform URL is replaced.
"""

import argparse
import json
from pathlib import Path


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True)
    parser.add_argument("--url", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args(argv)

    manifest = json.loads(Path(args.input).read_text(encoding="utf-8"))
    platforms = manifest.get("platforms") or {}
    if "windows-x86_64" not in platforms:
        raise SystemExit("manifest has no windows-x86_64 platform")
    platforms["windows-x86_64"]["url"] = args.url
    Path(args.output).write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())