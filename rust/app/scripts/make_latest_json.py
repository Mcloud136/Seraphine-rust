import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description="Build a Tauri updater latest.json")
    parser.add_argument("--installer", required=True, type=Path)
    parser.add_argument("--signature", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--url", required=True, help="Public HTTPS URL of the installer")
    parser.add_argument("--notes", default="")
    parser.add_argument("--pub-date", required=True, help="RFC 3339 timestamp")
    parser.add_argument("--platform", default="windows-x86_64")
    parser.add_argument("--output", default=Path("latest.json"), type=Path)
    args = parser.parse_args()

    signature = args.signature.read_text(encoding="utf-8").strip()
    payload = {
        "version": args.version,
        "notes": args.notes,
        "pub_date": args.pub_date,
        "platforms": {
            args.platform: {
                "signature": signature,
                "url": args.url,
            }
        },
    }
    args.output.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(
        f"wrote {args.output} version={args.version} "
        f"installer={args.installer.name} "
        f"sha256={hashlib.sha256(args.installer.read_bytes()).hexdigest()[:16]}…"
    )


if __name__ == "__main__":
    main()