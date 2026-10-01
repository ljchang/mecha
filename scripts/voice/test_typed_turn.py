"""A line typed into a live call becomes a user turn and nothing else
(the owner's ask, 2026-10-01): trimmed, capped, and never a field or a
setting. Run in the worker's venv:
`~/models/voice-worker-venv/bin/python scripts/voice/test_typed_turn.py`."""

import sys
import unittest

sys.path.insert(0, __file__.rsplit("/", 1)[0])

from worker import MAX_TYPED_CHARS, UNLOGGED, spoken_words, typed_turn  # noqa: E402


class TypedTurn(unittest.TestCase):
    def test_a_line_is_the_turn_trimmed(self):
        self.assertEqual(typed_turn({"text": "  how was the dig?  "}), "how was the dig?")

    def test_nothing_worth_a_turn_is_none(self):
        for data in (None, {}, {"text": ""}, {"text": "   "}, {"text": 7}, "text", {"voice": "ada"}):
            self.assertIsNone(typed_turn(data), repr(data))

    def test_a_long_paste_is_capped(self):
        self.assertEqual(len(typed_turn({"text": "x" * (MAX_TYPED_CHARS + 50)})), MAX_TYPED_CHARS)

    def test_its_words_are_withheld_from_the_log_in_an_incognito_call(self):
        with UNLOGGED.held(True):
            self.assertNotIn("secret", spoken_words("the secret plan", 80))


if __name__ == "__main__":
    unittest.main()
