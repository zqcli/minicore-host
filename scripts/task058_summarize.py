#!/usr/bin/env python3
"""Summarize task058 raw results descriptively; never applies pass thresholds."""
import argparse
import json
import math
from pathlib import Path
import statistics


def describe(values):
    values = sorted(value for value in values if value is not None)
    if not values:
        return None
    return {"n": len(values), "min": values[0], "median": statistics.median(values),
            "p95_nearest_rank": values[math.ceil(len(values) * 0.95) - 1], "max": values[-1]}


def summarize(data):
    if "failure" in data:
        raise ValueError("failed runs are not successful performance evidence")
    runs = data["runs"]
    output = {"source_revision": data["environment"]["source_revision"],
              "harness_revision": data["environment"]["harness_revision"],
              "repetitions": len(runs),
              "note": "Descriptive, correlated samples, not statistical significance or capacity.",
              "queued_accept_ms": describe(sample["accept_ms"] for run in runs
                                           for sample in run["paused_queue"]["samples"]),
              "stream": {}, "stock": {}, "cancel": {}, "shutdown": {}}
    for label in sorted({work["label"] for run in runs for work in run["stream"]}):
        works = [work for run in runs for work in run["stream"] if work["label"] == label]
        output["stream"][label] = {
            "accept_ms": describe(sample["accept_ms"] for work in works for sample in work["samples"]),
            "body_ms": describe(observation["body_ms"] for work in works
                                for sample in work["samples"] for observation in sample["observations"]),
            "first_observed_from_send_ms": describe(sample["first_observed_from_send_ms"]
                                                    for work in works for sample in work["samples"]),
            "fixture_wait_ms": describe(sample["fixture_wait_ms"] for work in works for sample in work["samples"]),
            "host_cpu_ms_by_run": [work["resources"]["cpu_ms"] for work in works],
            "wall_ms_by_run": [work["resources"]["wall_ms"] for work in works],
        }
    for label in [stage["label"] for stage in runs[0]["stock"]]:
        stages = [stage for run in runs for stage in run["stock"] if stage["label"] == label]
        output["stock"][label] = {
            "rss_kib_by_run": [stage["resources"]["end"]["rss_kib"] for stage in stages],
            "host_cpu_ms_by_run": [stage["resources"]["cpu_ms"] for stage in stages],
        }
    for key in ["control_accept_ms", "turn_finished_observed_ms", "turn_wait_returned_ms",
                "pid_group_gone_verified_ms", "fixture_wait_ms", "tool_ready_from_release_ms",
                "tool_ready_to_cancel_ms"]:
        output["cancel"][key] = [run["cancel_tool"][key] for run in runs]
    for key in ["accept_ms", "stopped_diagnostic_observed_ms", "process_exit_observed_ms"]:
        output["shutdown"][key] = [run["shutdown"][key] for run in runs]
    output["process"] = [{"run": run["run"], "load_begin": run["load_begin"], "load_end": run["load_end"],
                          "host_cpu_ms": run["host_lifetime_sampled_resources"]["cpu_ms"],
                          "sampled_window_ms": run["host_lifetime_sampled_resources"]["wall_ms"],
                          "peak_sampled_rss_kib": run["host_rss_peak_sampled_kib"],
                          "high_water_rss_kib": run["host_rss_high_water_kib"],
                          "rss_samples_present": run["rss_samples_present"],
                          "rss_samples_missing": run["rss_samples_missing"]} for run in runs]
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results", type=Path)
    args = parser.parse_args()
    print(json.dumps(summarize(json.loads(args.results.read_text())), indent=2))


if __name__ == "__main__":
    main()
