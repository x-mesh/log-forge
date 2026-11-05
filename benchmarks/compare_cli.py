#!/usr/bin/env python3
import argparse
import json
import platform
import statistics
import subprocess
import sys
import time
from pathlib import Path


def run_worker(command, cwd, count):
    import resource

    started = time.perf_counter()
    result = subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=300, check=False)
    elapsed = time.perf_counter() - started
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {result.stderr.decode(errors='replace')}")
    records = result.stdout.count(b'HTTP/1.1"')
    if records != count:
        raise RuntimeError(f"expected {count} records, got {records}")
    peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    peak_bytes = peak if sys.platform == "darwin" else peak * 1024
    return {"seconds": elapsed, "peak_rss_bytes": peak_bytes}


def run(command, cwd, count):
    result = subprocess.run(
        [sys.executable, str(Path(__file__).resolve()), "--worker"],
        input=json.dumps({"command": command, "cwd": str(cwd), "count": count}),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=310,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(f"measurement failed: {result.stderr}")
    return json.loads(result.stdout)


def main():
    if sys.argv[1:] == ["--worker"]:
        options = json.load(sys.stdin)
        print(json.dumps(run_worker(**options)))
        return
    if sys.platform not in ("darwin", "linux"):
        raise RuntimeError("peak memory measurement requires macOS or Linux")
    parser = argparse.ArgumentParser(description="Compare Nginx CLI generation with the Python source")
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--binary", type=Path, default=Path("target/release/log-forge"))
    parser.add_argument("--count", type=int, default=10000)
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    if args.count <= 0 or args.runs <= 0:
        parser.error("count and runs must be positive")
    original = args.original.resolve()
    binary = args.binary.resolve()
    common = ["start", "--output", "console", "--types", "nginx_access", "--total", str(args.count), "--virtual-time", "--no-monitor", "--start-time", "2024-01-01"]
    cases = [("python", [args.python, str(original / "local_cli.py"), *common], original), ("rust", [str(binary), *common], binary.parent)]
    samples = {name: [] for name, _, _ in cases}
    for index in range(args.runs):
        for name, command, cwd in cases[index % 2:] + cases[:index % 2]:
            samples[name].append(run(command, cwd, args.count))
    seconds = {name: [sample["seconds"] for sample in values] for name, values in samples.items()}
    peak_rss = {name: [sample["peak_rss_bytes"] for sample in values] for name, values in samples.items()}
    medians = {name: statistics.median(times) for name, times in seconds.items()}
    print(json.dumps({"platform": platform.platform(), "python": args.python, "records": args.count, "runs": args.runs, "wall_seconds": seconds, "peak_rss_bytes": peak_rss, "median_peak_rss_bytes": {name: statistics.median(values) for name, values in peak_rss.items()}, "median_seconds": medians, "median_records_per_second": {name: args.count / elapsed for name, elapsed in medians.items()}, "median_speedup": medians["python"] / medians["rust"]}, indent=2))


if __name__ == "__main__":
    main()
