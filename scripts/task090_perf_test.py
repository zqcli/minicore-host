#!/usr/bin/env python3
"""Task090 diagnostic parser fixtures, not Host performance measurements."""
import copy
import unittest

from task090_perf import stopped_boundaries


HOST = {"host_id": "h" * 20, "host_incarnation": "i" * 20}
OPERATION = "o" * 20


def records():
    common = {"event": "host.stopped", "schema_version": 1,
              "scope": "HOST", "level": "INFO", **HOST}
    return [({**common, "component": "host.owner", "operation_id": OPERATION,
              "attributes": {"phase": "stopped", "mode": "persistent"}}, 100),
            ({**common, "component": "host.process"}, 200)]


class CurrentStoppedDiagnostics(unittest.TestCase):
    def parse(self, values):
        return stopped_boundaries(values, HOST, OPERATION)

    def test_owner_and_process_are_separate_boundaries(self):
        self.assertEqual(self.parse(records()), {"host.owner": 100, "host.process": 200})

    def test_both_records_are_required_not_either_as_a_fallback(self):
        for values in [[], records()[:1], records()[1:]]:
            with self.subTest(values=values), self.assertRaises(AssertionError):
                self.parse(values)

    def test_duplicates_are_not_multiple_completion_witnesses(self):
        for values in [records() * 2, records()[:1] * 2, records()[1:] * 2]:
            with self.subTest(values=values), self.assertRaises(AssertionError):
                self.parse(values)

    def test_current_schema_and_host_identity_are_required_for_each(self):
        for index in range(2):
            for key, bad in [("host_id", "wrong"), ("host_incarnation", "wrong"),
                             ("component", "unknown"), ("schema_version", 2),
                             ("scope", "SESSION"), ("level", "DEBUG")]:
                value = records()
                value[index][0][key] = bad
                with self.subTest(index=index, key=key), self.assertRaises(AssertionError):
                    self.parse(value)

    def test_actual_shutdown_operation_and_stopped_phase_are_required(self):
        for key, bad in [("operation_id", "wrong"), ("attributes", {"phase": "stopping"})]:
            value = records()
            value[0][0][key] = bad
            with self.subTest(key=key), self.assertRaises(AssertionError):
                self.parse(value)

    def test_missing_source_and_component_fields_are_not_silently_accepted(self):
        for index in range(2):
            for key in [*HOST, "component", "schema_version", "scope", "level"]:
                value = copy.deepcopy(records())
                del value[index][0][key]
                with self.subTest(index=index, key=key), self.assertRaises(KeyError):
                    self.parse(value)

    def test_no_cross_producer_diagnostic_order_is_assumed(self):
        values = records()
        values = [(values[1][0], 50), values[0]]
        self.assertEqual(self.parse(values), {"host.process": 50, "host.owner": 100})

    def test_other_events_are_not_completion_witnesses(self):
        other = {"event": "host.shutdown_accepted", "component": "host.owner"}
        self.assertEqual(self.parse([(other, 1), *records()]), self.parse(records()))


if __name__ == "__main__":
    unittest.main()
