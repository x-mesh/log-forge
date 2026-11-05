#!/usr/bin/env python3
import argparse
import datetime
import importlib
import json
import random
import re
import subprocess
import sys
import tempfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/log-forge"))
    parser.add_argument("--count", type=int, default=100)
    args = parser.parse_args()
    if args.count <= 0:
        parser.error("count must be positive")
    sys.path.insert(0, str(args.original.resolve()))
    factory = importlib.import_module("log_generator.core.factory").LogFactory()
    timestamp = datetime.datetime(2024, 1, 1, tzinfo=datetime.timezone.utc)
    cases = {
        "nginx_access": ({"status_codes": {503: 1}, "user_agents": ["configured-agent"]}, r'" 503 \d+ "[^"]*" "configured-agent"$'),
        "apache_access": ({"status_codes": {503: 1}, "user_agents": ["configured-agent"]}, r'" 503 \d+ "[^"]*" "configured-agent"$'),
        "nginx_error": ({"error_levels": {"warn": 1}}, r" \[warn\] "),
        "apache_error": ({"error_levels": {"warn": 1}}, r" \[warn\] "),
        "mysql_error": ({"log_levels": {"Error": 1}}, r" \[Error\] "),
        "mysql_query": ({"query_types": {"INSERT": 1}, "table_names": ["audit_events"]}, r"^\d{6} \d{2}:\d{2}:\d{2}\s+\d+\s+(?:Query\s+INSERT INTO audit_events\b|(?!Query\b)\w+\s+.+)"),
        "syslog": ({"facilities": {"local3": 1}, "severities": {"err": 1}, "hostname": "configured-host"}, r"^<155>.* configured-host "),
        "fastapi": ({"log_levels": {"WARNING": 1}}, r" \[WARNING\] "),
        "django_request": ({"log_levels": {"WARNING": 1}}, r" WARNING django\.request: "),
        "django_sql": ({"models": ["Ledger"]}, r'"app_ledger"'),
        "postgresql": ({"log_levels": {"ERROR": 1}}, r" \[ERROR\] "),
        "kubernetes_pod": ({"log_levels": {"ERROR": 1}}, r" ERROR "),
        "kubernetes_event": ({"event_types": {"Warning": 1}}, r" Warning "),
        "goloop": ({"log_levels": {"E": 1}, "modules": ["BM"]}, r"^E\|[^|]+\|[^|]+\|[^|]+\|BM\|"),
    }
    report = {}
    with tempfile.TemporaryDirectory() as directory:
        config = Path(directory) / "config.json"
        for kind, (fields, pattern) in cases.items():
            random.seed(42)
            generator = factory.create_generator(kind, {"custom_fields": fields})
            original = [generator.generate_log_with_time(timestamp) for _ in range(args.count)]
            config.write_text(json.dumps({"log_generator": {"global": {"output_format": "console", "total_logs": args.count}, "log_types": {kind: {"custom_fields": fields}}}}), encoding="utf-8")
            command = [str(args.binary.resolve()), "start", "--config", str(config), "--standalone-config", "--types", kind, "--start-time", "2024-01-01", "--seed", "42", "--no-monitor"]
            result = subprocess.run(command, capture_output=True, text=True, timeout=30, check=True)
            rust = result.stdout.splitlines()
            if len(rust) != args.count:
                raise RuntimeError(f"{kind}: expected {args.count} Rust records, got {len(rust)}")
            failures = {name: sum(re.search(pattern, record) is None for record in records) for name, records in (("original", original), ("rust", rust))}
            if kind == "mysql_query" and any(not any(" Query " in record for record in records) for records in (original, rust)):
                raise RuntimeError("mysql_query: no SQL query generated")
            report[kind] = failures
        docker = factory.create_generator("docker", {})
        original_docker = [json.loads(docker.generate_log_with_time(timestamp))["log"] for _ in range(args.count)]
        command = [str(args.binary.resolve()), "start", "--output", "console", "--types", "docker", "--total", str(args.count), "--start-time", "2024-01-01", "--seed", "42", "--no-monitor"]
        result = subprocess.run(command, capture_output=True, text=True, timeout=30, check=True)
        rust_docker = [json.loads(line)["log"] for line in result.stdout.splitlines()]
        if len(rust_docker) != args.count:
            raise RuntimeError(f"docker: expected {args.count} Rust records, got {len(rust_docker)}")
        docker_endings = {name: {"newline": sum(value.endswith("\n") for value in records), "literal_backslash_n": sum(value.endswith("\\n") for value in records)} for name, records in (("original", original_docker), ("rust", rust_docker))}
    print(json.dumps({"configured": report, "docker_log_endings": docker_endings}, indent=2))
    if any(any(failures.values()) for failures in report.values()):
        sys.exit(1)


if __name__ == "__main__":
    main()
