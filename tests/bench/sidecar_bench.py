import argparse
import asyncio
import json
import statistics
import time
from pathlib import Path

import psutil

from app.backend.sidecar_client import SidecarClient


def percentile(values, ratio):
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * ratio))]


async def run(duration, rate, sidecar_path, include_samples=False):
    client = SidecarClient(sidecar_path=sidecar_path)
    await client.start()
    process = psutil.Process(client.process.pid)
    samples = []
    request_samples = []
    latencies = []
    errors = 0
    diagnostics = None
    sidecar_returncode = None
    sidecar_stderr = []
    started = time.monotonic()
    next_request = started
    last_cpu = process.cpu_times()
    last_sample = started

    try:
        while time.monotonic() - started < duration:
            now = time.monotonic()
            if rate > 0 and now >= next_request:
                request_started = time.perf_counter_ns()
                try:
                    await client.health()
                    latency_ms = (time.perf_counter_ns() - request_started) / 1_000_000
                    latencies.append(latency_ms)
                    request_samples.append(
                        {
                            "qpcTimeMs": time.perf_counter_ns() / 1_000_000,
                            "latencyMs": latency_ms,
                        }
                    )
                except asyncio.CancelledError:
                    raise
                except Exception:
                    errors += 1
                next_request = next_request + 1.0 / rate if rate > 0 else float("inf")

            now = time.monotonic()
            if now - last_sample >= 0.5:
                try:
                    cpu = process.cpu_times()
                    memory = process.memory_info()
                except psutil.NoSuchProcess:
                    errors += 1
                    sidecar_stderr = list(client.stderr_lines)
                    break
                cpu_ms = (cpu.user - last_cpu.user) + (cpu.system - last_cpu.system)
                samples.append(
                    {
                        "cpuPercent": cpu_ms * 1000 / ((now - last_sample) * 1000) * 100,
                        "workingSetMB": memory.rss / (1024 * 1024),
                        "privateMB": getattr(memory, "private", memory.rss) / (1024 * 1024),
                        "qpcTimeMs": time.perf_counter_ns() / 1_000_000,
                    }
                )
                last_cpu = cpu
                last_sample = now

            await asyncio.sleep(min(0.01, max(0.0, next_request - time.monotonic())))
    finally:
        if not client.closed:
            try:
                diagnostics = await client.diagnostics()
            except BaseException:
                diagnostics = None
        else:
            sidecar_stderr = list(client.stderr_lines)
        sidecar_returncode = client.process.returncode if client.process is not None else None
        await client.close()

    cpu_values = [sample["cpuPercent"] for sample in samples]
    working_values = [sample["workingSetMB"] for sample in samples]
    private_values = [sample["privateMB"] for sample in samples]
    result = {
        "durationSeconds": duration,
        "requests": len(latencies),
        "errors": errors,
        "sidecarReturncode": sidecar_returncode,
        "latencyP50Ms": round(percentile(latencies, 0.50), 3),
        "latencyP95Ms": round(percentile(latencies, 0.95), 3),
        "latencyP99Ms": round(percentile(latencies, 0.99), 3),
        "cpuAvgPercent": round(statistics.mean(cpu_values), 3) if cpu_values else 0,
        "cpuMaxPercent": round(max(cpu_values), 3) if cpu_values else 0,
        "workingSetAvgMB": round(statistics.mean(working_values), 2) if working_values else 0,
        "workingSetMaxMB": round(max(working_values), 2) if working_values else 0,
        "privateAvgMB": round(statistics.mean(private_values), 2) if private_values else 0,
        "privateMaxMB": round(max(private_values), 2) if private_values else 0,
        "diagnostics": diagnostics,
    }
    if include_samples:
        result["samples"] = samples
        result["requestSamples"] = request_samples
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--duration", type=float, default=60)
    parser.add_argument("--rate", type=float, default=10)
    parser.add_argument(
        "--sidecar",
        default=str(Path(__file__).resolve().parents[2] / "rust" / "target" / "release" / "seraphine-sidecar.exe"),
    )
    args = parser.parse_args()
    print(json.dumps(asyncio.run(run(args.duration, args.rate, args.sidecar)), indent=2))


if __name__ == "__main__":
    main()
