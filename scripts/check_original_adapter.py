#!/usr/bin/env python3
"""Check original Python generators through the process adapter."""
import argparse
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/log-forge"))
    args = parser.parse_args()
    original = args.original.resolve()
    sys.path.insert(0, str(original))
    adapter = Path(__file__).resolve().parents[1] / "adapters/python_generator.py"
    cases = [
        ("nginx", "NginxAccessLogGenerator", {"format_type": "common", "status_codes": {"503": 1}}),
        ("docker", "DockerLogGenerator", {"format_type": "text"}),
        ("syslog", "SyslogGenerator", {"rfc_format": "5424"}),
    ]
    report = {}
    with tempfile.TemporaryDirectory() as directory:
        config_path = Path(directory) / "config.json"
        for module, name, options in cases:
            module_name = f"log_generator.generators.{module}"
            cls = getattr(importlib.import_module(module_name), name)
            constructor_options = {key: value for key, value in options.items() if key in ("format_type", "rfc_format")}
            generator = cls(**constructor_options)
            config = {"log_generator": {"global": {"output_format": "console", "total_logs": 10,
                                                       "start_time": "2024-01-01"},
                      "log_types": {"legacy": {"plugin_command": sys.executable,
                                                  "plugin_args": [str(adapter), f"{module_name}:{name}"],
                                                  "plugin_config": options, "plugin_timeout": 5}}}}
            config_path.write_text(json.dumps(config), encoding="utf-8")
            env = os.environ.copy()
            env["PYTHONPATH"] = str(original)
            output = subprocess.run([str(args.binary.resolve()), "start", "--config", str(config_path),
                                     "--no-monitor"], capture_output=True, text=True, env=env, timeout=30)
            if output.returncode:
                raise RuntimeError(f"{module}: {output.stderr}")
            records = output.stdout.splitlines()
            if len(records) != 10 or not all(generator.validate_log(record) for record in records):
                raise ValueError(f"{module}: invalid output or record count")
            if module == "nginx" and any(record.count('"') != 2 for record in records):
                raise ValueError("nginx common format was not applied")
            if module == "nginx" and any('" 503 ' not in record for record in records):
                raise ValueError("nginx status code was not applied")
            if module == "syslog" and any(">1 " not in record for record in records):
                raise ValueError("syslog RFC 5424 format was not applied")
            if module == "docker" and any(record.startswith("{") for record in records):
                raise ValueError("docker text format was not applied")
            report[module] = {"records": len(records), "warning": output.stderr.strip()}
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
