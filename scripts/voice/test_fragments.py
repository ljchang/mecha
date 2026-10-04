"""Pieces with nothing to say on their own wait for the next (`fragments.py`).
Pure, so it runs on a bare python3 as CI does:
`python3 scripts/voice/test_fragments.py`."""

import importlib.util
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from fragments import MAX_HOLD, Joiner, holds  # noqa: E402


def run(pieces):
    j = Joiner()
    out = [s for p in pieces if (s := j.push(p)) is not None]
    if (rest := j.flush()) is not None:
        out.append(rest)
    return out


class WhatWaits(unittest.TestCase):
    def test_nothing_speakable_waits(self):
        for piece in (".", "…", "?!", "(laugh)", "(clears throat)", "(sigh) .", "  "):
            self.assertTrue(holds(piece) or not piece.strip(), piece)

    def test_a_trailing_ellipsis_waits(self):
        for piece in ("Hmm..", "let me see...", "maybe…", '"Wait..."', "so.."):
            self.assertTrue(holds(piece), piece)

    def test_a_whole_sentence_is_spoken(self):
        for piece in ("Mm.", "Yes!", "The kettle is on.", "(laugh) That is so funny.",
                      "Is the train late?", "I met him (and his sister) at the lake."):
            self.assertFalse(holds(piece), piece)

    def test_a_long_run_is_spoken_anyway(self):
        self.assertFalse(holds("word.. " * (MAX_HOLD // 7 + 1)))


class TheJoin(unittest.TestCase):
    def test_a_trailing_off_run_is_one_phrase_not_forty(self):
        # The shape of the 2026-10-03 reply that came out as 84 pieces.
        pieces = ["Hmm..", ".", "let me see..", ".", "give me a second..", ".",
                  "[removed].", "Okay..", ".", "so..", ".", "and then..", ".", "Good."]
        out = run(pieces)
        self.assertEqual(out, [
            "Hmm... let me see... give me a second... The kettle is on.",
            "Okay... so... and then... Good.",
        ])
        self.assertFalse(any(s.strip(" .…") == "" for s in out), "a lone dot was sent")

    def test_a_tag_rides_with_the_sentence_after_it(self):
        self.assertEqual(run(["(laugh)", "That is so funny."]), ["(laugh) That is so funny."])

    def test_a_tag_at_the_very_end_is_flushed_with_what_was_held(self):
        self.assertEqual(run(["You did it.", "(laugh)"]), ["You did it.", "(laugh)"])

    def test_ordinary_sentences_pass_through_unchanged(self):
        pieces = ["The bus was on time.", "Did the parcel come?", "Leave it by the door."]
        self.assertEqual(run(pieces), pieces)

    def test_clear_drops_what_was_held(self):
        j = Joiner()
        self.assertIsNone(j.push("Hmm.."))
        j.clear()
        self.assertEqual(j.push("Hello."), "Hello.")


@unittest.skipUnless(
    all(importlib.util.find_spec(m) for m in ("pipecat", "loguru")),
    "the worker's splitter needs its venv",
)
class TheWorkersSplitter(unittest.TestCase):
    """pipecat's own sentence splitter with the joiner in front, fed the way a
    streaming reply arrives: in chunks that split words."""

    def test_streamed_chunks_come_out_joined(self):
        import asyncio

        import worker

        agg = worker.JoiningAggregator()

        async def feed():
            out = []
            for chunk in ["Hmm.. ", "le", "t me see.. ", "one sec.. ", "I am here. ",
                          "(laugh) ", "So funny. ", "Bye"]:
                async for piece in agg.aggregate(chunk):
                    out.append(piece.text)
            rest = await agg.flush()
            out.append(rest.text if rest else None)
            return out

        self.assertEqual(asyncio.run(feed()),
                         ["Hmm.. let me see.. one sec.. I am here.", "(laugh) So funny.", "Bye"])

    def test_an_interruption_drops_what_was_held(self):
        import asyncio

        import worker

        agg = worker.JoiningAggregator()

        async def feed():
            async for _ in agg.aggregate("Hmm.. let me see.. "):
                pass
            await agg.handle_interruption()
            out = [p.text async for p in agg.aggregate("Hello there. Next")]
            return out

        self.assertEqual(asyncio.run(feed()), ["Hello there."])


if __name__ == "__main__":
    unittest.main()
