#!/usr/bin/env python3
"""Pure opt-in checks for task058 RSS postprocessing; no Host performance run."""
import unittest

from task058_perf import aggregate_rss, resource_snapshots


def sample(at, rss, high_water):
    return {"pid": 42, "at_ns": at, "rss_kib": rss, "high_water_rss_kib": high_water}


class RssAggregation(unittest.TestCase):
    def test_boundary_maximum_is_not_omitted(self):
        periodic = [sample(1, 15180, 15180)]
        run = {"cancel_tool": {"resources": {"end": sample(2, 15216, 15216)}}}
        result = aggregate_rss(periodic + [value for _, value in resource_snapshots(run)])
        self.assertEqual(result["host_rss_peak_sampled_kib"], 15216)
        self.assertEqual(result["host_rss_high_water_kib"], 15216)
        self.assertEqual(result["rss_samples_present"], 2)

    def test_duplicate_boundary_is_counted_once(self):
        value = sample(1, 5, 8)
        result = aggregate_rss([value, value.copy()])
        self.assertEqual(result["rss_samples_present"], 1)
        self.assertEqual(result["host_rss_peak_sampled_kib"], 5)
        self.assertEqual(result["host_rss_high_water_kib"], 8)

    def test_missing_fields_remain_independent(self):
        result = aggregate_rss([sample(1, None, 7), sample(2, 3, None)])
        self.assertEqual(result["host_rss_peak_sampled_kib"], 3)
        self.assertEqual(result["host_rss_high_water_kib"], 7)
        self.assertEqual(result["rss_samples_present"], 1)
        self.assertEqual(result["rss_samples_missing"], 1)
        self.assertEqual(result["high_water_samples_missing"], 1)

    def test_all_unknown_does_not_become_zero(self):
        result = aggregate_rss([sample(1, None, None)])
        self.assertIsNone(result["host_rss_peak_sampled_kib"])
        self.assertIsNone(result["host_rss_high_water_kib"])
        self.assertEqual(result["rss_samples_present"], 0)
        self.assertEqual(result["rss_samples_missing"], 1)

    def test_empty_does_not_become_zero(self):
        result = aggregate_rss([])
        self.assertIsNone(result["host_rss_peak_sampled_kib"])
        self.assertIsNone(result["host_rss_high_water_kib"])
        self.assertEqual(result["rss_samples_missing"], 0)


if __name__ == "__main__":
    unittest.main()
