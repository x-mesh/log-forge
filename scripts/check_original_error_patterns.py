#!/usr/bin/env python3
"""Compare built-in error patterns and scenarios with the original source."""
import argparse
import json
import sys
import tempfile
from dataclasses import asdict
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument(
        "--rust-data",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "data/builtin_error_patterns.json",
    )
    parser.add_argument(
        "--rust-scenarios",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "error_scenarios",
    )
    args = parser.parse_args()
    sys.path.insert(0, str(args.original.resolve()))
    from log_generator.patterns.error_patterns import ErrorPatternLibrary
    from log_generator.patterns.error_manager import ErrorPatternManager

    with tempfile.TemporaryDirectory() as directory:
        original = {
            name: asdict(pattern)
            for name, pattern in ErrorPatternLibrary(directory).patterns.items()
        }
        original_scenarios = {
            name: asdict(scenario)
            for name, scenario in ErrorPatternManager(
                str(Path(directory) / "patterns"), str(Path(directory) / "scenarios")
            ).scenarios.items()
        }
    differences = []
    rust = {}
    for pattern in json.loads(args.rust_data.read_text()):
        name = pattern["id"]
        if name in rust:
            differences.append({"pattern": name, "field": "duplicate_id"})
        rust[name] = pattern
    for name in sorted(original.keys() | rust.keys()):
        if name not in original or name not in rust:
            differences.append({"pattern": name, "field": "id"})
            continue
        original_pattern = original[name]
        rust_pattern = dict(rust[name])
        original_pattern.pop("created_at", None)
        rust_pattern.pop("created_at", None)
        rust_pattern.setdefault("real_world_examples", [])
        for field in sorted(original_pattern.keys() | rust_pattern.keys()):
            if field not in original_pattern or field not in rust_pattern or original_pattern[field] != rust_pattern[field]:
                differences.append({"pattern": name, "field": field})
    scenarios = {}
    for path in sorted(args.rust_scenarios.glob("*.json")):
        scenario = json.loads(path.read_text())
        if scenario["name"] in scenarios:
            raise ValueError(f"duplicate scenario: {scenario['name']}")
        scenarios[scenario["name"]] = scenario
    scenario_differences = []
    for name, scenario in sorted(original_scenarios.items()):
        if name not in scenarios:
            scenario_differences.append({"scenario": name, "field": "name"})
            continue
        for field in sorted(scenario.keys() | scenarios[name].keys()):
            if field not in scenario or field not in scenarios[name] or scenario[field] != scenarios[name][field]:
                scenario_differences.append({"scenario": name, "field": field})
    print(json.dumps({"original": len(original), "rust": len(rust), "differences": differences,
                      "original_scenarios": len(original_scenarios), "rust_scenarios": len(scenarios),
                      "scenario_differences": scenario_differences}, indent=2))
    if differences or scenario_differences:
        sys.exit(1)


if __name__ == "__main__":
    main()
