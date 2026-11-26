import argparse
import os
import signal
import subprocess
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default=r"D:\tmp\game-frames.csv")
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--process", default="League of Legends.exe")
    parser.add_argument("--session", default="GameBench")
    parser.add_argument(
        "--presentmon",
        default=os.path.expandvars(
            r"%LOCALAPPDATA%\Microsoft\WinGet\Links\presentmon.exe"
        ),
    )
    args = parser.parse_args()

    # --output_file stays empty in this PresentMon build, so stream the same CSV
    # rows through stdout and stop the process with Ctrl+Break to flush them.
    # Stale trace sessions from an interrupted capture make PresentMon start but
    # record nothing, so clear the name before reusing it.
    subprocess.run(
        ["logman", "stop", args.session, "-ets"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    command = [
        args.presentmon,
        "--process_name",
        args.process,
        "--output_stdout",
        "--v1_metrics",
        "--qpc_time_ms",
        "--no_console_stats",
        "--session_name",
        args.session,
        "--stop_existing_session",
    ]
    csv_file = open(args.output, "wb")
    monitor = subprocess.Popen(
        command,
        stdout=csv_file,
        stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
        creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
    )
    time.sleep(args.seconds)
    os.kill(monitor.pid, signal.CTRL_BREAK_EVENT)
    try:
        monitor.wait(timeout=20)
    except subprocess.TimeoutExpired:
        monitor.kill()
        monitor.wait()
    csv_file.close()
    # A hard kill leaves the ETW session behind and later captures fail with
    # error 1450, so stop anything the monitor did not close itself.
    subprocess.run(
        ["logman", "stop", args.session, "-ets"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    size = os.path.getsize(args.output)
    if size < 64:
        raise SystemExit(f"PresentMon captured no frames: {args.output} ({size} bytes)")
    print(f"captured {args.output} target={args.process} seconds={args.seconds} bytes={size}")


if __name__ == "__main__":
    main()