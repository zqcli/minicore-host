#!/usr/bin/env python3
"""Pure parsing checks using explicit DTO fixtures, not performance evidence."""
import copy
import unittest

from task058_perf import (
    MODEL, agent_items, bound_message_turn, finished_turn_data, stream_body,
)


HOST = {"host_id": "h" * 20, "host_incarnation": "i" * 20}
SESSION = {**HOST, "agent_id": "a" * 20, "session_id": "s" * 20}
TURN = {**SESSION, "turn_id": "t" * 20, "load_id": "l" * 20}
MESSAGE_ID = "m" * 20


def message_fixture():
    return {**SESSION, "message_id": MESSAGE_ID, "data": {
        "text": "prompt", "mode": "follow_up", "source": {"kind": "user"},
        "binding": {"state": "bound", "turn_id": TURN["turn_id"]},
    }}


def result_fixture(outcome="completed", persistence="not_applicable"):
    return {**TURN, "data": {
        "info": {"state": "finished", "outcome": outcome, "persistence": persistence,
                 "cleanup_faults": [], "fault": None, "persistence_fault": None},
        "config": {"model": MODEL}, "partials": [],
        "history": [{"kind": "user", "text": "prompt"},
                    {"kind": "assistant", "response_index": 0, "response": {
                        "content": [{"type": "text", "data": "a"},
                                    {"type": "text", "data": "é"}],
                        "finish_reason": "stop", "usage": {}}}],
    }}


class CurrentQueryProjection(unittest.TestCase):
    def test_message_binding_under_public_data(self):
        self.assertEqual(bound_message_turn(message_fixture(), SESSION, MESSAGE_ID, "prompt"),
                         TURN["turn_id"])

    def test_message_receiver_and_message_identity_are_verified(self):
        for key in [*SESSION, "message_id"]:
            with self.subTest(key=key), self.assertRaises(AssertionError):
                value = message_fixture()
                value[key] = "wrong"
                bound_message_turn(value, SESSION, MESSAGE_ID, "prompt")

    def test_message_bytes_and_bound_state_are_verified(self):
        for field, replacement in [("text", "different"), ("binding", {"state": "queued"})]:
            with self.subTest(field=field), self.assertRaises(AssertionError):
                value = message_fixture()
                value["data"][field] = replacement
                bound_message_turn(value, SESSION, MESSAGE_ID, "prompt")

    def test_removed_message_shape_is_not_a_fallback(self):
        value = message_fixture()
        value.update(value.pop("data"))
        with self.assertRaises(KeyError):
            bound_message_turn(value, SESSION, MESSAGE_ID, "prompt")

    def test_public_history_yields_exact_utf8_bytes(self):
        value = result_fixture()
        data = finished_turn_data(value, TURN, "completed", "not_applicable")
        self.assertEqual(stream_body(data, "prompt"), b"a\xc3\xa9")

    def test_cancelled_persisted_state_uses_same_projection(self):
        value = result_fixture("cancelled", "persisted")
        self.assertIs(finished_turn_data(value, TURN, "cancelled", "persisted"), value["data"])

    def test_turn_source_and_load_are_verified(self):
        for key in TURN:
            with self.subTest(key=key), self.assertRaises(AssertionError):
                value = result_fixture()
                value[key] = "wrong"
                finished_turn_data(value, TURN, "completed", "not_applicable")

    def test_finished_state_outcome_persistence_and_faults_are_verified(self):
        for key, bad in [("state", "running"), ("outcome", "cancelled"),
                         ("persistence", "failed"), ("cleanup_faults", [{"message": "bad"}]),
                         ("fault", {"message": "bad"}), ("persistence_fault", {"message": "bad"})]:
            with self.subTest(key=key), self.assertRaises(AssertionError):
                value = result_fixture()
                value["data"]["info"][key] = bad
                finished_turn_data(value, TURN, "completed", "not_applicable")

    def test_partial_cannot_replace_complete_stream_body(self):
        value = result_fixture()
        value["data"]["partials"] = [{"response_index": 0, "partial": {
            "content": [{"type": "text", "data": "aé"}],
            "incomplete_tool_calls": [], "usage": None}}]
        with self.assertRaises(AssertionError):
            finished_turn_data(value, TURN, "completed", "not_applicable")

    def test_legacy_native_result_and_history_are_not_fallbacks(self):
        with self.assertRaises(KeyError):
            finished_turn_data(result_fixture()["data"], TURN, "completed", "not_applicable")
        data = result_fixture()["data"]
        data["history"] = [{"type": "assistant", "data": {"content": [
            {"type": "text", "data": "aé"}]}}]
        with self.assertRaises(KeyError):
            stream_body(data, "prompt")

    def test_stream_response_index_and_text_channel_are_verified(self):
        for bad in [{"response_index": 1}, {"response": {
                "finish_reason": "stop", "content": [{"type": "reasoning", "data": "aé"}]}}]:
            with self.subTest(bad=bad), self.assertRaises(AssertionError):
                data = result_fixture()["data"]
                data["history"][1].update(bad)
                stream_body(data, "prompt")


class CurrentAgentPage(unittest.TestCase):
    def page(self, ids=("a", "b")):
        return {"items": [{"id": value, "host_id": HOST["host_id"]} for value in ids],
                "next_cursor": None}

    def test_concrete_page_items_and_empty_list(self):
        self.assertEqual(len(agent_items(self.page(), HOST)), 2)
        self.assertEqual(agent_items(self.page(()), HOST), [])

    def test_unpaged_result_must_not_have_more_items(self):
        value = self.page()
        value["next_cursor"] = "b"
        with self.assertRaises(AssertionError):
            agent_items(value, HOST)

    def test_agent_ids_must_be_unique_and_lexical(self):
        for ids in [("b", "a"), ("a", "a"), ("",), (None,)]:
            with self.subTest(ids=ids), self.assertRaises(AssertionError):
                agent_items(self.page(ids), HOST)

    def test_concrete_page_fields_and_agent_host_are_required(self):
        value = self.page()
        value["items"][0]["host_id"] = "wrong"
        with self.assertRaises(AssertionError):
            agent_items(value, HOST)
        for key in ("items", "next_cursor"):
            value = self.page()
            del value[key]
            with self.subTest(page_field=key), self.assertRaises(KeyError):
                agent_items(value, HOST)
        for key in ("id", "host_id"):
            value = copy.deepcopy(self.page())
            del value["items"][0][key]
            with self.subTest(item_field=key), self.assertRaises(KeyError):
                agent_items(value, HOST)
        with self.assertRaises(KeyError):
            agent_items({**HOST, "data": self.page()}, HOST)
        with self.assertRaises(AssertionError):
            agent_items(self.page()["items"], HOST)


if __name__ == "__main__":
    unittest.main()
