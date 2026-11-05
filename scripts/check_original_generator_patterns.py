#!/usr/bin/env python3
"""Compare generator message templates with the original Python source."""
import argparse
import importlib
import json
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument("--rust-data", type=Path, default=Path(__file__).resolve().parent.parent / "data")
    args = parser.parse_args()
    sys.path.insert(0, str(args.original.resolve()))
    modules = {
        "NginxErrorLogGenerator": "nginx",
        "ApacheErrorLogGenerator": "apache",
        "MySQLErrorLogGenerator": "mysql",
        "DockerLogGenerator": "docker",
        "KubernetesPodLogGenerator": "kubernetes",
        "FastAPILogGenerator": "fastapi",
    }
    entries = [
        ("NginxErrorLogGenerator", "ERROR_PATTERNS"),
        ("ApacheErrorLogGenerator", "ERROR_PATTERNS"),
        ("MySQLErrorLogGenerator", "ERROR_PATTERNS"),
        ("MySQLErrorLogGenerator", "WARNING_PATTERNS"),
        ("MySQLErrorLogGenerator", "NOTE_PATTERNS"),
        ("DockerLogGenerator", "APP_LOG_PATTERNS"),
        ("DockerLogGenerator", "ERROR_PATTERNS"),
        ("KubernetesPodLogGenerator", "APP_LOG_PATTERNS"),
        ("KubernetesPodLogGenerator", "ERROR_PATTERNS"),
        *(("FastAPILogGenerator", level) for level in ("INFO", "DEBUG", "WARNING", "ERROR")),
    ]
    rust = json.loads((args.rust_data / "generator_patterns.json").read_text())
    differences = []
    expected_keys = {f"{name}.{attribute}" for name, attribute in entries}
    for name, attribute in entries:
        cls = getattr(importlib.import_module(f"log_generator.generators.{modules[name]}"), name)
        original = cls.LOG_PATTERNS[attribute] if name == "FastAPILogGenerator" else getattr(cls, attribute)
        key = f"{name}.{attribute}"
        if key not in rust or rust[key] != original:
            differences.append(key)
    differences.extend(sorted(rust.keys() - expected_keys))
    syslog = importlib.import_module("log_generator.generators.syslog").SyslogGenerator
    if json.loads((args.rust_data / "syslog_patterns.json").read_text()) != syslog.MESSAGE_PATTERNS:
        differences.append("syslog.MESSAGE_PATTERNS")
    goloop = importlib.import_module("log_generator.generators.goloop").GoloopLogGenerator
    goloop_data = json.loads((args.rust_data / "goloop_patterns.json").read_text())
    for key, attribute in (("sub_modules", "SUB_MODULES"), ("log_patterns", "LOG_PATTERNS"), ("source_files", "SOURCE_FILES")):
        if key not in goloop_data or goloop_data[key] != getattr(goloop, attribute):
            differences.append(f"goloop.{attribute}")
    differences.extend(f"goloop.{key}" for key in sorted(goloop_data.keys() - {"sub_modules", "log_patterns", "source_files"}))
    print(json.dumps({"generator_groups": len(entries), "differences": differences}, indent=2))
    if differences:
        sys.exit(1)


if __name__ == "__main__":
    main()
