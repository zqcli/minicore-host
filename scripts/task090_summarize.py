#!/usr/bin/env python3
"""Extend the unchanged Task058 descriptive summary with Task090 boundaries."""
import argparse
import json
from pathlib import Path

from task058_summarize import summarize


def fresh_summary(data):
    result = summarize(data)  # Rejects failed runs, never sets pass thresholds.
    runs = data["runs"]
    result["binary_sha256"] = data["environment"]["binary_sha256"]
    result["actual_model_requests_by_run"] = [run["actual_model_requests"] for run in runs]
    result["shutdown"]["owner_stopped_diagnostic_observed_ms"] = [
        run["shutdown"]["owner_stopped_diagnostic_observed_ms"] for run in runs]
    for label, summary in result["stream"].items():
        works = [work for run in runs for work in run["stream"] if work["label"] == label]
        summary["model_gate_wall_ms_by_run"] = [work["model_gate_resources"]["wall_ms"]
                                                for work in works]
        summary["model_gate_host_cpu_ms_by_run"] = [work["model_gate_resources"]["cpu_ms"]
                                                    for work in works]
    result["paused_queue_resources_by_run"] = [run["paused_queue"]["resources"] for run in runs]
    result["tool_wait_resources_by_run"] = [run["cancel_tool"]["tool_wait_resources"] for run in runs]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    args = parser.parse_args()
    print(json.dumps(fresh_summary(json.loads(args.results.read_text())), indent=2))


if __name__ == "__main__":
    main()
