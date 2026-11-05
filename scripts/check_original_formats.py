#!/usr/bin/env python3
import argparse
import importlib
import json
import random
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path


def goloop_reason(record):
    parts = record.split("|")
    if len(parts) < 6:
        return "other"
    if parts[3] == "-":
        return "sequence_dash"
    if not re.fullmatch(r"[A-Z0-9]+", parts[4]):
        return "module_case"
    source = parts[-1].split(" ", 1)[0].split(":", 1)[0]
    if not re.fullmatch(r"[a-z_]+\.go", source):
        return "source_name"
    return "other"


def rejection_reason(kind, record):
    if kind == "django_request" and "\nTraceback (most recent call last):" in record:
        return "django_traceback"
    if kind == "goloop":
        return goloop_reason(record)
    return "other"


def validate_records(binary, kind, records):
    with tempfile.NamedTemporaryFile(mode="w", suffix=".log", encoding="utf-8") as sample:
        sample.write("\n".join(records) + "\n")
        sample.flush()
        result = subprocess.run([str(binary), "validate", "--input", sample.name, "--type", kind], capture_output=True, text=True, timeout=30, check=False)
    match = re.fullmatch(r"validated=(\d+) valid=(\d+) invalid=(\d+)\s*", result.stdout)
    if not match:
        raise RuntimeError(f"{kind} validation failed: {result.stdout} {result.stderr}")
    total, valid, invalid = map(int, match.groups())
    if total != len(records) or valid + invalid != total or result.returncode != (1 if invalid else 0):
        raise RuntimeError(f"{kind} validation failed: {result.stdout} {result.stderr}")
    return invalid


def main():
    parser = argparse.ArgumentParser(description="Check Rust records with the original Python validators")
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/log-forge"))
    parser.add_argument("--samples", type=int, default=50)
    args = parser.parse_args()
    if args.samples <= 0:
        parser.error("samples must be positive")
    sys.path.insert(0, str(args.original.resolve()))
    factory_module = importlib.import_module("log_generator.core.factory")
    factory = factory_module.LogFactory()
    expected = {"nginx_access", "nginx_error", "apache_access", "apache_error", "django_request", "django_sql", "docker", "kubernetes_pod", "kubernetes_event", "mysql_query", "mysql_error", "mysql_slow", "postgresql", "postgresql_slow", "syslog", "fastapi", "goloop"}
    missing = expected.difference(factory.get_available_types())
    if missing:
        raise RuntimeError(f"original factory did not load: {sorted(missing)}")
    binary = args.binary.resolve()
    timestamp = datetime(2024, 1, 1, tzinfo=timezone.utc)
    report = {}
    for kind in factory.get_available_types():
        generator = factory.create_generator(kind, {})
        random.seed(41)
        source_rejected = 0
        source_reasons = {}
        rust_rejected = 0
        rust_reasons = {}
        rust_validator_rejected = 0
        original_records = []
        rust_records = []
        first_failure = None
        for seed in range(args.samples):
            original = generator.generate_log_with_time(timestamp)
            original_records.append(original)
            if not generator.validate_log(original):
                source_rejected += 1
                reason = rejection_reason(kind, original)
                source_reasons[reason] = source_reasons.get(reason, 0) + 1
            command = [str(binary), "start", "--output", "console", "--types", kind, "--total", "1", "--start-time", "2024-01-01", "--seed", str(seed), "--no-monitor"]
            result = subprocess.run(command, capture_output=True, text=True, timeout=15, check=False)
            if result.returncode:
                raise RuntimeError(f"{kind} failed: {result.stderr}")
            record = result.stdout.rstrip("\n")
            rust_records.append(record)
            if not generator.validate_log(record):
                rust_rejected += 1
                reason = rejection_reason(kind, record)
                rust_reasons[reason] = rust_reasons.get(reason, 0) + 1
                with tempfile.NamedTemporaryFile(mode="w", suffix=".log", encoding="utf-8") as sample:
                    sample.write(record + "\n")
                    sample.flush()
                    checked = subprocess.run([str(binary), "validate", "--input", sample.name, "--type", kind], capture_output=True, text=True, timeout=15, check=False)
                    if checked.returncode:
                        rust_validator_rejected += 1
                if first_failure is None:
                    first_failure = record
        original_rust_rejected = validate_records(binary, kind, original_records)
        all_rust_rejected = validate_records(binary, kind, rust_records)
        report[kind] = {"samples": args.samples, "original_rejected": source_rejected, "original_rejection_reasons": source_reasons, "rust_rejected": rust_rejected, "rust_rejection_reasons": rust_reasons, "rust_validator_rejected": rust_validator_rejected, "original_records_rejected_by_rust": original_rust_rejected, "all_rust_records_rejected_by_rust": all_rust_rejected, "first_rejected_rust_record": first_failure}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    if any(value["rust_validator_rejected"] or value["original_records_rejected_by_rust"] or value["all_rust_records_rejected_by_rust"] or value["rust_rejection_reasons"].get("other", 0) for value in report.values()):
        sys.exit(1)


if __name__ == "__main__":
    main()
