import argparse
import asyncio
import bisect
import csv
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

from sidecar_bench import percentile, run as run_sidecar_bench


def read_presentmon(path):
    frames = []
    with open(path, "r", encoding="utf-8-sig", newline="") as file:
        for row in csv.DictReader(file):
            try:
                frame_time = float(row["MsBetweenPresents"])
                qpc_time = float(row["CPUStartQPCTime"])
            except (KeyError, TypeError, ValueError):
                continue
            if frame_time > 0:
                frames.append({"qpcTimeMs": qpc_time, "frameTimeMs": frame_time})
    return frames


def summarize_frames(frames):
    values = [frame["frameTimeMs"] for frame in frames]
    if not values:
        return {"frames": 0}
    p99 = percentile(values, 0.99)
    return {
        "frames": len(values),
        "fpsAvg": round(1000.0 / statistics.mean(values), 3),
        "frameTimeP50Ms": round(percentile(values, 0.50), 3),
        "frameTimeP95Ms": round(percentile(values, 0.95), 3),
        "frameTimeP99Ms": round(p99, 3),
        "frameTimeMaxMs": round(max(values), 3),
        "onePercentLowFps": round(1000.0 / p99, 3) if p99 > 0 else 0,
        "stallsOver50Ms": sum(value > 50 for value in values),
        "stallsOver100Ms": sum(value > 100 for value in values),
    }


def aggregate_rounds(rounds):
    if not rounds:
        return {"rounds": 0}
    frames = [item["frames"] for item in rounds]

    def median(key):
        return round(statistics.median(item[key] for item in frames), 3)

    stalls_per_minute = []
    for item in rounds:
        duration = float(item.get("sidecar", {}).get("durationSeconds") or 0)
        if duration > 0:
            stalls_per_minute.append(
                item["frames"].get("stallsOver50Ms", 0) * 60.0 / duration
            )
    return {
        "rounds": len(rounds),
        "frameTimeP50Ms": median("frameTimeP50Ms"),
        "frameTimeP95Ms": median("frameTimeP95Ms"),
        "frameTimeP99Ms": median("frameTimeP99Ms"),
        "frameTimeP99WorstMs": round(
            max(item["frameTimeP99Ms"] for item in frames), 3
        ),
        "stallsOver50MsPerMinute": round(statistics.median(stalls_per_minute), 3)
        if stalls_per_minute
        else 0,
        "sidecarCpuAvgPercent": round(
            statistics.median(item["sidecar"]["cpuAvgPercent"] for item in rounds), 3
        ),
        "sidecarLatencyP99Ms": round(
            statistics.median(item["sidecar"]["latencyP99Ms"] for item in rounds), 3
        ),
        "workingSetAvgMB": round(
            statistics.median(item["sidecar"]["workingSetAvgMB"] for item in rounds), 2
        ),
    }


def compare_regression(
    current,
    baseline,
    max_p99_regression_percent=5.0,
    max_stall_increase_per_minute=2.0,
):
    reasons = []
    base_p99 = float(baseline.get("frameTimeP99Ms") or 0)
    current_p99 = float(current.get("frameTimeP99Ms") or 0)
    if base_p99 <= 0 or current_p99 <= 0:
        reasons.append("baseline or current P99 is missing")
        p99_regression = None
    else:
        p99_regression = round((current_p99 / base_p99 - 1.0) * 100.0, 3)
        if p99_regression > max_p99_regression_percent:
            reasons.append(
                "P99 regression %.3f%% exceeds %.3f%%"
                % (p99_regression, max_p99_regression_percent)
            )
    base_stalls = float(baseline.get("stallsOver50MsPerMinute") or 0)
    current_stalls = float(current.get("stallsOver50MsPerMinute") or 0)
    stall_increase = round(current_stalls - base_stalls, 3)
    if stall_increase > max_stall_increase_per_minute:
        reasons.append(
            "stall increase %.3f/min exceeds %.3f/min"
            % (stall_increase, max_stall_increase_per_minute)
        )
    return {
        "passed": not reasons,
        "p99RegressionPercent": p99_regression,
        "stallIncreasePerMinute": stall_increase,
        "thresholds": {
            "maxP99RegressionPercent": max_p99_regression_percent,
            "maxStallIncreasePerMinute": max_stall_increase_per_minute,
        },
        "reasons": reasons,
    }


def _nearest(sorted_values, value):
    index = bisect.bisect_left(sorted_values, value)
    candidates = []
    if index < len(sorted_values):
        candidates.append(sorted_values[index])
    if index > 0:
        candidates.append(sorted_values[index - 1])
    return min(candidates, key=lambda candidate: abs(candidate - value)) if candidates else None


def correlate_stalls(frames, samples, request_samples, threshold_ms=50.0):
    sample_times = [sample["qpcTimeMs"] for sample in samples]
    cpu_values = []
    request_latencies = []
    for frame in frames:
        if frame["frameTimeMs"] <= threshold_ms:
            continue
        nearest = _nearest(sample_times, frame["qpcTimeMs"])
        if nearest is not None:
            cpu_values.append(samples[sample_times.index(nearest)]["cpuPercent"])
        nearby = [
            request["latencyMs"]
            for request in request_samples
            if abs(request["qpcTimeMs"] - frame["qpcTimeMs"]) <= 250
        ]
        if nearby:
            request_latencies.append(max(nearby))
    return {
        "stallFrames": sum(frame["frameTimeMs"] > threshold_ms for frame in frames),
        "sidecarCpuP95Percent": round(percentile(cpu_values, 0.95), 3) if cpu_values else 0,
        "sidecarCpuMaxPercent": round(max(cpu_values), 3) if cpu_values else 0,
        "nearbyRequestP95Ms": round(percentile(request_latencies, 0.95), 3)
        if request_latencies
        else 0,
        "nearbyRequestMaxMs": round(max(request_latencies), 3) if request_latencies else 0,
    }


async def capture(args, round_index=1):
    if args.warmup > 0:
        print(
            "round %d: warmup %.1f seconds" % (round_index, args.warmup),
            file=sys.stderr,
        )
        await asyncio.sleep(args.warmup)
    output_dir = Path(args.output_dir) / ("round-%d" % round_index)
    output_dir.mkdir(parents=True, exist_ok=True)
    csv_path = output_dir / "presentmon.csv"
    json_path = output_dir / "summary.json"
    duration = max(1, int(args.duration))
    command = [
        args.presentmon,
        "--process_name",
        args.process_name,
        "--output_file",
        str(csv_path),
        "--qpc_time_ms",
        "--no_console_stats",
        "--timed",
        str(duration),
        "--terminate_after_timed",
        "--session_name",
        "Seraphine-%d-%d" % (time.time_ns(), id(command)),
        "--stop_existing_session",
    ]
    creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    presentmon = await asyncio.create_subprocess_exec(
        *command,
        stdin=asyncio.subprocess.DEVNULL,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        creationflags=creationflags,
    )
    present_task = asyncio.create_task(presentmon.communicate())
    bench_task = asyncio.create_task(
        run_sidecar_bench(duration, args.rate, args.sidecar, include_samples=True)
    )
    try:
        done, _ = await asyncio.wait(
            {present_task, bench_task}, return_when=asyncio.FIRST_COMPLETED
        )
        if present_task in done and bench_task not in done:
            bench_task.cancel()
            await asyncio.gather(bench_task, return_exceptions=True)
            stdout, stderr = present_task.result()
            raise RuntimeError(
                "PresentMon stopped before capture completed: %s"
                % (
                    stderr.decode("utf-8", errors="replace")
                    or stdout.decode("utf-8", errors="replace")
                )
            )
        sidecar = await bench_task
        stdout, stderr = await present_task
        if presentmon.returncode != 0:
            raise RuntimeError(
                "PresentMon failed (%s): %s"
                % (
                    presentmon.returncode,
                    stderr.decode("utf-8", errors="replace")
                    or stdout.decode("utf-8", errors="replace")
                )
            )
    finally:
        if presentmon.returncode is None:
            try:
                presentmon.terminate()
            except ProcessLookupError:
                pass
            await presentmon.wait()
        if not bench_task.done():
            bench_task.cancel()
            await asyncio.gather(bench_task, return_exceptions=True)
        if not present_task.done():
            present_task.cancel()
            await asyncio.gather(present_task, return_exceptions=True)
    frames = read_presentmon(csv_path)
    if not frames:
        raise RuntimeError("PresentMon produced no usable frame rows")
    result = {
        "round": round_index,
        "presentMonCsv": str(csv_path),
        "processName": args.process_name,
        "frames": summarize_frames(frames),
        "sidecar": sidecar,
        "correlation": correlate_stalls(
            frames,
            sidecar.get("samples", []),
            sidecar.get("requestSamples", []),
        ),
    }
    json_path.write_text(json.dumps(result, indent=2), encoding="utf-8")
    return result


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--presentmon", required=True)
    parser.add_argument("--process-name", default="League of Legends.exe")
    parser.add_argument("--duration", type=float, default=300)
    parser.add_argument("--warmup", type=float, default=60)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--rate", type=float, default=20)
    parser.add_argument("--baseline", help="previous aggregate or single-round JSON")
    parser.add_argument("--max-p99-regression-percent", type=float, default=5.0)
    parser.add_argument("--max-stall-increase-per-minute", type=float, default=2.0)
    parser.add_argument(
        "--sidecar",
        default=str(
            Path(__file__).resolve().parents[2]
            / "rust"
            / "target"
            / "release"
            / "seraphine-sidecar.exe"
        ),
    )
    parser.add_argument("--output-dir", default="benchmark-output")
    args = parser.parse_args(argv)
    if args.rounds < 1:
        parser.error("--rounds must be at least 1")
    rounds = [
        asyncio.run(capture(args, index)) for index in range(1, args.rounds + 1)
    ]
    report = {"rounds": rounds, "aggregate": aggregate_rounds(rounds)}
    if args.baseline:
        baseline_report = json.loads(Path(args.baseline).read_text(encoding="utf-8"))
        baseline = baseline_report.get("aggregate", baseline_report)
        report["regression"] = compare_regression(
            report["aggregate"],
            baseline,
            args.max_p99_regression_percent,
            args.max_stall_increase_per_minute,
        )
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / "summary.json").write_text(
        json.dumps(report, indent=2), encoding="utf-8"
    )
    print(json.dumps(report, indent=2))
    if report.get("regression", {}).get("passed") is False:
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
