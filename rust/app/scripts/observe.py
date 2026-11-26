"""Summarize Seraphine Rust logs for the seven-day observation gate.

Reads %LOCALAPPDATA%\\Seraphine\\app\\log\\*.log (or --log-dir) and reports the
signals that matter for retiring the Python fallback:

- panics / fatal startup errors
- LCU connect/disconnect/reconnect counts
- write failures (auto accept / ban / pick / spells / complete)
- ARAM update failures and successful updates
- warning/error volume by day

Usage:
    python rust/app/scripts/observe.py [--log-dir PATH] [--since 2026-11-27] [--json]
"""

import argparse
import json
import os
import re
import sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

LINE = re.compile(
    r"^(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})\.\d+Z\s+\[(?P<level>\w+)\]\s+(?P<body>.*)$"
)
CATEGORIES = (
    ("panic", re.compile(r"panic:")),
    ("connected", re.compile(r"lcu connected")),
    ("disconnected", re.compile(r"lcu disconnected")),
    ("session_error", re.compile(r"session error")),
    ("accepted", re.compile(r"auto accepted match")),
    ("accept_failed", re.compile(r"auto accept (failed|skipped)")),
    ("ban", re.compile(r"auto banned champion")),
    ("ban_failed", re.compile(r"auto ban failed")),
    ("pick", re.compile(r"auto picked champion")),
    ("pick_failed", re.compile(r"auto pick failed")),
    ("spells", re.compile(r"auto set summoner spells")),
    ("spells_failed", re.compile(r"auto set (spells|summoner spells) failed")),
    ("completed", re.compile(r"auto completed champion selection")),
    ("complete_failed", re.compile(r"auto complete failed")),
    ("aram_updated", re.compile(r"aram://updated|aram update")),
    ("aram_failed", re.compile(r"aram update failed")),
    ("catalog_refresh_failed", re.compile(r"catalog refresh failed")),
)


def default_log_dir() -> Path:
    local = os.environ.get("LOCALAPPDATA") or str(Path.home() / "AppData" / "Local")
    return Path(local) / "Seraphine" / "app" / "log"


def parse_since(value: str | None):
    if not value:
        return None
    for template in ("%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d"):
        try:
            return datetime.strptime(value, template)
        except ValueError:
            continue
    raise SystemExit(f"invalid --since: {value!r} (use YYYY-MM-DD[THH:MM[:SS]])")


def scan(log_dir: Path, since: str | None):
    since_dt = parse_since(since)
    counts = Counter()
    days = Counter()
    failures = []
    first = None
    last = None
    for path in sorted(log_dir.glob("*.log")):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for line in text.splitlines():
            match = LINE.match(line)
            if not match:
                continue
            stamp = match.group("ts")
            utc_dt = datetime.strptime(stamp, "%Y-%m-%dT%H:%M:%S").replace(
                tzinfo=timezone.utc
            )
            local_dt = utc_dt.astimezone().replace(tzinfo=None)
            label = local_dt.strftime("%Y-%m-%dT%H:%M:%S")
            if since_dt and local_dt < since_dt:
                continue
            day = local_dt.strftime("%Y-%m-%d")
            first = label if first is None else min(first, label)
            last = label if last is None else max(last, label)
            level = match.group("level")
            body = match.group("body")
            if level in ("WARN", "ERROR"):
                days[day] += 1
            if level == "ERROR" or "failed" in body or "panic" in body:
                for name, pattern in CATEGORIES:
                    if name.endswith("_failed") or name == "panic":
                        if pattern.search(body):
                            failures.append(f"{label} {body.strip()}")
            for name, pattern in CATEGORIES:
                if pattern.search(body):
                    counts[name] += 1
    return {
        "log_dir": str(log_dir),
        "since": since,
        "first_line": first,
        "last_line": last,
        "counts": dict(sorted(counts.items())),
        "warn_error_by_day": dict(sorted(days.items())),
        "recent_failures": failures[-20:],
    }


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--log-dir", default=str(default_log_dir()))
    parser.add_argument("--since", help="YYYY-MM-DD, inclusive")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = scan(Path(args.log_dir), args.since)
    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0
    print(f"log dir : {report['log_dir']}")
    print(f"range   : {report['first_line']} .. {report['last_line']} (local time)")
    print("signals :")
    for name, value in report["counts"].items():
        print(f"  {name:24} {value}")
    print("warn/error by day:")
    for day, value in report["warn_error_by_day"].items():
        print(f"  {day} {value}")
    if report["recent_failures"]:
        print("recent failures:")
        for line in report["recent_failures"]:
            print(f"  {line}")
    return 0


if __name__ == "__main__":
    sys.exit(main())