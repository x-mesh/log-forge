#!/usr/bin/env python3
import argparse
import json
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path


def docker(*args, timeout=120):
    return subprocess.run(["docker", *args], check=True, capture_output=True, text=True, timeout=timeout).stdout.strip()


def query(base):
    params = urllib.parse.urlencode({"query": 'log_forge_records_total{log_type="redis"}'})
    with urllib.request.urlopen(f"{base}/api/v1/query?{params}", timeout=2) as response:
        payload = json.load(response)
    if payload.get("status") != "success":
        raise RuntimeError(f"Prometheus query failed: {payload}")
    return payload["data"]["result"]


def main():
    parser = argparse.ArgumentParser(description="Check Remote Write with a local Prometheus container")
    parser.add_argument("--binary", type=Path, default=Path("target/release/log-forge"))
    parser.add_argument("--image", default="prom/prometheus:v3.13.3")
    args = parser.parse_args()
    container = None
    generator = None
    try:
        container = docker("run", "--detach", "--rm", "--publish", "127.0.0.1::9090", "--tmpfs", "/prometheus:rw,size=128m,uid=65534,gid=65534", args.image, "--config.file=/etc/prometheus/prometheus.yml", "--storage.tsdb.path=/prometheus", "--web.enable-remote-write-receiver")
        binding = docker("port", container, "9090/tcp")
        port = int(binding.rsplit(":", 1)[1])
        base = f"http://127.0.0.1:{port}"
        deadline = time.monotonic() + 30
        while True:
            try:
                with urllib.request.urlopen(f"{base}/-/ready", timeout=2) as response:
                    if response.status == 200:
                        break
            except (OSError, TimeoutError):
                if time.monotonic() >= deadline:
                    raise RuntimeError("Prometheus did not become ready")
                time.sleep(0.1)
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "config.json"
            config.write_text(json.dumps({"log_generator": {"global": {"total_logs": 16, "virtual_time": False, "generation_interval": 0.25, "batch_size": 1}, "log_types": {"redis": {"enabled": True}}, "output": {"handlers": [{"type": "network", "protocol": "http", "host": "127.0.0.1", "port": port, "endpoint": "/api/v1/write", "format": "prometheus"}]}}}), encoding="utf-8")
            generator = subprocess.Popen([str(args.binary.resolve()), "start", "--config", str(config), "--no-monitor"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            deadline = time.monotonic() + 10
            observed = None
            while time.monotonic() < deadline and generator.poll() is None:
                series = query(base)
                if series and float(series[0]["value"][1]) >= 2:
                    observed = float(series[0]["value"][1])
                    break
                time.sleep(0.1)
            stdout, stderr = generator.communicate(timeout=10)
            if generator.returncode != 0:
                raise RuntimeError(f"log-forge failed: {stderr}")
            if observed is None:
                raise RuntimeError(f"Prometheus did not expose the generated counter: {stdout} {stderr}")
            deadline = time.monotonic() + 5
            while query(base):
                if time.monotonic() >= deadline:
                    raise RuntimeError("Prometheus still returns the series after shutdown")
                time.sleep(0.1)
            print(f"Prometheus received the counter (observed value: {observed:g}) and applied the stale marker.")
    finally:
        if generator is not None and generator.poll() is None:
            generator.terminate()
            generator.wait(timeout=5)
        if container is not None:
            try:
                docker("stop", "--time", "2", container, timeout=15)
            except subprocess.CalledProcessError as error:
                # A container that exits before readiness is removed by --rm.
                if "No such container" not in error.stderr:
                    raise


if __name__ == "__main__":
    main()
