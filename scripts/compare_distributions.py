#!/usr/bin/env python3
"""Compare web access log distributions with the original generators."""
import argparse
import collections
import datetime
import importlib
import json
import random
import re
import subprocess
import sys
import tempfile
from pathlib import Path


PATTERN = re.compile(r'"(?P<method>[A-Z]+) \S+ HTTP/1\.1" (?P<status>\d{3}) ')


def distribution(records):
    counts = {"method": collections.Counter(), "status": collections.Counter()}
    total = 0
    for record in records:
        match = PATTERN.search(record)
        if not match:
            raise ValueError(f"invalid access record: {record!r}")
        for field in counts:
            counts[field][match[field]] += 1
        total += 1
    if not total:
        raise ValueError("no records")
    return {field: {key: value / total for key, value in sorted(values.items())}
            for field, values in counts.items()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/log-forge"))
    parser.add_argument("--count", type=int, default=5000)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()
    if args.count <= 0:
        parser.error("count must be positive")
    sys.path.insert(0, str(args.original.resolve()))
    timestamp = datetime.datetime(2024, 1, 1, tzinfo=datetime.timezone.utc)
    report = {}
    with tempfile.TemporaryDirectory() as directory:
        config = Path(directory) / "empty.yaml"
        config.write_text("log_generator: {}\n", encoding="utf-8")
        for kind, module, name in [("nginx_access", "nginx", "NginxAccessLogGenerator"),
                                   ("apache_access", "apache", "ApacheAccessLogGenerator")]:
            random.seed(args.seed)
            generator = getattr(importlib.import_module(f"log_generator.generators.{module}"), name)()
            generator.faker.seed_instance(args.seed)
            original = distribution(generator.generate_log_with_time(timestamp) for _ in range(args.count))
            command = [str(args.binary.resolve()), "start", "--config", str(config), "--types", kind,
                       "--output", "console", "--total", str(args.count), "--start-time", "2024-01-01",
                       "--seed", str(args.seed), "--no-monitor"]
            output = subprocess.run(command, capture_output=True, text=True, check=True, timeout=300)
            records = output.stdout.splitlines()
            if len(records) != args.count:
                raise ValueError(f"{kind}: expected {args.count} records, got {len(records)}")
            rust = distribution(records)
            delta = {field: {key: rust[field].get(key, 0) - original[field].get(key, 0)
                             for key in sorted(original[field].keys() | rust[field].keys())}
                     for field in original}
            report[kind] = {"original": original, "rust": rust, "delta": delta}
    print(json.dumps({"samples": args.count, "seed": args.seed, "types": report}, indent=2))


if __name__ == "__main__":
    main()
