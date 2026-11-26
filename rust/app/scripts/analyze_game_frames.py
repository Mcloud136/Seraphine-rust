import csv
import io
import statistics
import sys


def percentile(values, ratio):
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * ratio))]


def read_text(path):
    raw = open(path, "rb").read()
    if raw.startswith(b"\xff\xfe"):
        return raw.decode("utf-16-le").lstrip("\ufeff")
    if raw.startswith(b"\xfe\xff"):
        return raw.decode("utf-16-be").lstrip("\ufeff")
    return raw.decode("utf-8-sig", errors="replace")


def main(path, target):
    values = []
    for row in csv.DictReader(io.StringIO(read_text(path))):
        if row.get("Application") != target:
            continue
        try:
            value = float(row["msBetweenPresents"])
        except (KeyError, TypeError, ValueError):
            continue
        if value > 0:
            values.append(value)
    if not values:
        print({"frames": 0})
        return
    p99 = percentile(values, 0.99)
    print(
        {
            "frames": len(values),
            "fpsAvg": round(1000.0 / statistics.mean(values), 2),
            "p50Ms": round(percentile(values, 0.50), 2),
            "p95Ms": round(percentile(values, 0.95), 2),
            "p99Ms": round(p99, 2),
            "maxMs": round(max(values), 2),
            "stallsOver50": sum(value > 50 for value in values),
            "stallsOver100": sum(value > 100 for value in values),
        }
    )


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else "League of Legends.exe")