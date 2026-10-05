"""The guard that keeps conversation text out of this repository
(`check-private.py`), measured both ways (review of #557).

CI has no conversations, so the guard always passes there; this builds a
made-up store under a scratch MECHA_HOME and a scratch git repository, and
checks that a sentence from the store is refused, the same sentence already in
the repository is excused, a persona's name and a real session id are refused,
an unreadable range fails closed, and an empty store passes saying so. Nothing
here reads the machine's real conversations or journal. Run:
`python3 scripts/test_check_private.py`."""

import json
import shutil
import sqlite3
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
GUARD = os.path.join(HERE, "check-private.py")
SAID = "The lighthouse keeper painted every second stair a pale blue colour."
SESSION = "20990101T000000-0a1b2c3d"


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, check=True).stdout


class Guard(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = self.tmp.name
        self.home = os.path.join(root, "home")
        persona = os.path.join(self.home, "personas", "quillon")
        os.makedirs(os.path.join(persona, "sessions"))
        open(os.path.join(persona, "persona.toml"), "w").write(
            'name = "quillon"\ndisplay = "Quillon Marsh"\ncharacter = "tessaly"\n')
        with open(os.path.join(persona, "sessions", f"{SESSION}.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": SESSION}) + "\n")
            f.write(json.dumps({"record": "message", "role": "assistant",
                                "content": [{"type": "text", "text": SAID}]}) + "\n")
        self.repo = os.path.join(root, "repo")
        os.makedirs(self.repo)
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.email", "t@example.com")
        git(self.repo, "config", "user.name", "t")
        open(os.path.join(self.repo, "README"), "w").write("start\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "start")

    def tearDown(self):
        self.tmp.cleanup()

    def run_guard(self, *args, home=None):
        env = dict(os.environ, MECHA_HOME=home or self.home, CHECK_PRIVATE_JOURNAL="0")
        return subprocess.run([sys.executable, GUARD, *args], cwd=self.repo, env=env,
                              capture_output=True, text=True)

    def stage(self, name, text):
        open(os.path.join(self.repo, name), "w").write(text)
        git(self.repo, "add", name)

    def test_a_sentence_from_a_conversation_is_refused_and_not_printed(self):
        self.stage("fixture.py", f"X = {SAID!r}\n")
        r = self.run_guard("--staged")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("fixture.py:1: words said in a conversation", r.stdout)
        self.assertNotIn("lighthouse", r.stdout)

    def test_a_short_quoted_phrase_is_refused(self):
        self.stage("fixture.py", 'X = "every second stair"\n')
        self.assertEqual(self.run_guard("--staged").returncode, 1)

    def test_text_already_in_the_repository_is_excused(self):
        # The harness's own sentences are spoken in conversations too.
        self.stage("a.py", f"X = {SAID!r}\n")
        git(self.repo, "commit", "-q", "--no-verify", "-m", "already here")
        self.stage("b.py", f"Y = {SAID!r}\n")
        self.assertEqual(self.run_guard("--staged").returncode, 0)

    def test_a_persona_name_and_a_real_session_id_are_refused(self):
        self.stage("doc.md", "A note about Quillon.\n")
        self.assertIn("of the owner's personas", self.run_guard("--staged").stdout)
        git(self.repo, "reset", "-q")
        self.stage("doc.md", f"see {SESSION}\n")
        self.assertIn("session id", self.run_guard("--staged").stdout)

    def test_a_sentence_split_across_two_files_is_not_excused(self):
        # The repository "says" a run only within one file: words that meet
        # only across a file boundary excuse nothing (review of #557).
        half = SAID.split()
        self.stage("a.md", "notes\n" + " ".join(half[:4]) + "\n")
        self.stage("b.md", " ".join(half[4:]) + "\nmore\n")
        git(self.repo, "commit", "-q", "--no-verify", "-m", "split")
        self.stage("c.py", f"Y = {SAID!r}\n")
        self.assertEqual(self.run_guard("--staged").returncode, 1)

    def test_a_sentence_wrapped_across_added_lines_is_refused(self):
        # Prose is wrapped: the sentence crosses a line break in what is added.
        half = SAID.split()
        self.stage("doc.md", "/// " + " ".join(half[:5]) + "\n/// " + " ".join(half[5:]) + "\n")
        r = self.run_guard("--staged")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("doc.md:1-2", r.stdout)

    def test_a_commit_message_is_checked(self):
        msg = os.path.join(self.tmp.name, "COMMIT_EDITMSG")
        open(msg, "w").write(f"Fix the fixture\n\nIt said: {SAID}\n# a git comment\n")
        r = self.run_guard("--message", msg)
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("commit message:1-3", r.stdout)
        open(msg, "w").write("Fix the fixture\n\nIt said something made up.\n")
        self.assertEqual(self.run_guard("--message", msg).returncode, 0)

    def test_a_push_reads_every_commit_not_only_the_ends(self):
        # Added in one --no-verify commit and rewritten in the next: the
        # range's endpoint diff is clean, and the push must still refuse.
        base = git(self.repo, "rev-parse", "HEAD").strip()
        self.stage("wip.py", f"X = {SAID!r}\n")
        git(self.repo, "commit", "-q", "--no-verify", "-m", "wip")
        self.stage("wip.py", 'X = "The ferry leaves at noon."\n')
        git(self.repo, "commit", "-q", "--no-verify", "-m", "tidy")
        r = self.run_guard("--range", f"{base}..HEAD")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("wip.py (in ", r.stdout)

    def test_a_pushed_commit_message_is_checked(self):
        base = git(self.repo, "rev-parse", "HEAD").strip()
        self.stage("ok.py", 'X = "The ferry leaves at noon."\n')
        git(self.repo, "commit", "-q", "--no-verify", "-m", f"Fix it\n\nIt said: {SAID}")
        r = self.run_guard("--range", f"{base}..HEAD")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("commit message of", r.stdout)

    def test_a_file_with_a_non_ascii_name_is_read(self):
        # git quotes such paths in a diff header unless told not to.
        self.stage("café.md", f"{SAID}\n")
        r = self.run_guard("--staged")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("café.md:1", r.stdout)

    def test_how_a_persona_is_addressed_and_drawn_is_refused(self):
        for text in ("Talking with Marsh today.\n", "Draw tessaly by the sea.\n"):
            git(self.repo, "reset", "-q")
            self.stage("doc.md", text)
            self.assertIn("of the owner's personas", self.run_guard("--staged").stdout, text)

    def test_an_added_line_starting_with_plus_plus_is_content(self):
        # Inside a hunk, "++ b/x" is a line of text, not a diff header.
        self.stage("notes.md", "++ b/elsewhere\n++ just a line\n" + SAID + "\n")
        r = self.run_guard("--staged")
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("notes.md:1-3", r.stdout)

    def test_required_conversations_refuse_when_missing(self):
        empty = os.path.join(self.tmp.name, "empty")
        os.makedirs(empty)
        env = dict(os.environ, MECHA_HOME=empty, CHECK_PRIVATE_JOURNAL="0", CHECK_PRIVATE_REQUIRE="1")
        r = subprocess.run([sys.executable, GUARD, "--staged"], cwd=self.repo, env=env,
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 2, r.stdout)

    def test_an_unreadable_journal_refuses(self):
        # Pre-#547 call words exist only in the journal: failing to read it
        # is not an empty journal.
        fake = os.path.join(self.tmp.name, "bin")
        os.makedirs(fake)
        jc = os.path.join(fake, "journalctl")
        open(jc, "w").write("#!/bin/sh\necho 'Failed to open journal: permission denied' >&2\nexit 2\n")
        os.chmod(jc, 0o755)
        self.stage("fixture.py", 'X = "The ferry leaves at noon."\n')
        env = dict(os.environ, MECHA_HOME=self.home, CHECK_PRIVATE_JOURNAL="1",
                   PATH=fake + os.pathsep + os.environ["PATH"])
        r = subprocess.run([sys.executable, GUARD, "--staged"], cwd=self.repo, env=env,
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 2, r.stdout)
        self.assertIn("voice journal could not be read", r.stdout)

    def test_a_fact_in_a_persona_memory_is_refused(self):
        # Memory holds paraphrases written from chats: in no transcript.
        fact = "Keeps two hives of bees on the roof garden behind the library."
        db = os.path.join(self.home, "personas", "quillon", "memory.db")
        con = sqlite3.connect(db)
        con.execute("CREATE VIRTUAL TABLE recall_fts USING fts5 (uid UNINDEXED, text)")
        con.execute("INSERT INTO recall_fts VALUES ('f1', ?)", (fact,))
        con.commit()
        con.close()
        self.stage("fixture.py", f"X = {fact!r}\n")
        self.assertEqual(self.run_guard("--staged").returncode, 1)

    def test_an_unreadable_persona_memory_refuses(self):
        db = os.path.join(self.home, "personas", "quillon", "memory.db")
        open(db, "wb").write(b"not a database at all, just bytes " * 40)
        self.stage("fixture.py", 'X = "The ferry leaves at noon."\n')
        r = self.run_guard("--staged")
        self.assertEqual(r.returncode, 2, r.stdout)
        self.assertIn("persona memory", r.stdout)

    def test_a_required_hook_that_cannot_run_refuses(self):
        # The hook, without the script beside it.
        hook = os.path.join(self.repo, "commit-msg")
        shutil.copy(os.path.join(HERE, "..", ".githooks", "commit-msg"), hook)
        msg = os.path.join(self.tmp.name, "MSG")
        open(msg, "w").write("A message.\n")
        for require, want in (("1", 1), ("", 0)):
            env = dict(os.environ, CHECK_PRIVATE_REQUIRE=require)
            r = subprocess.run(["bash", hook, msg], cwd=self.repo, env=env, capture_output=True, text=True)
            self.assertEqual(r.returncode, want, (require, r.stdout))

    def test_a_diff_prefix_config_does_not_block(self):
        # diff.noprefix drops the a/ b/ prefixes the parser reads.
        git(self.repo, "config", "diff.noprefix", "true")
        self.stage("fixture.py", 'X = "The ferry leaves at noon."\n')
        self.assertEqual(self.run_guard("--staged").returncode, 0)
        git(self.repo, "reset", "-q")
        self.stage("fixture.py", f"X = {SAID!r}\n")
        self.assertEqual(self.run_guard("--staged").returncode, 1)

    def test_a_call_into_a_web_chat_is_in_the_corpus(self):
        # Calls speak into ordinary chats; the voice block marks the turn.
        heard = "Can you move the dentist booking to the second week of May."
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions)
        with open(os.path.join(sessions, "20990102T000000-1b2c3d4e.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "x", "kind": "web"}) + "\n")
            f.write(json.dumps({"record": "message", "role": "user", "content": [{"type": "text",
                    "text": "Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\n" + heard}]}) + "\n")
        self.stage("fixture.py", f"X = {heard!r}\n")
        self.assertEqual(self.run_guard("--staged").returncode, 1)

    def test_only_the_spoken_turns_of_a_web_chat_count(self):
        # A chat a call spoke into is mostly the owner typing (often about
        # mecha itself); only the spoken turn and its reply are voice data.
        heard = "Please put the swim lesson on Saturday morning at nine."
        reply = "Done, the swim lesson is on Saturday morning at nine sharp."
        typed = "Run the release checklist and then tag the next version."
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions, exist_ok=True)
        def msg(role, text):
            return json.dumps({"record": "message", "role": role,
                               "content": [{"type": "text", "text": text}]}) + "\n"
        with open(os.path.join(sessions, "20990103T000000-2c3d4e5f.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "y", "kind": "web"}) + "\n")
            f.write(msg("user", typed))
            f.write(msg("assistant", "Tagged the next version after the checklist passed."))
            f.write(msg("user", "Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\n" + heard))
            f.write(msg("assistant", reply))
        for text, want in ((heard, 1), (reply, 1), (typed, 0)):
            git(self.repo, "reset", "-q")
            self.stage("fixture.py", f"X = {text!r}\n")
            self.assertEqual(self.run_guard("--staged").returncode, want, text)

    def test_a_spoken_turn_that_calls_a_tool_keeps_its_reply_and_later_directed_turns_count(self):
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions, exist_ok=True)
        first = "Book the piano tuner for the first Tuesday of next month."
        reply = "The piano tuner is booked for the first Tuesday of next month."
        later = "And tell the neighbours the tuner arrives in the afternoon please."
        later_reply = "I will let the neighbours know the tuner comes in the afternoon."
        steer = "Actually make it the second Tuesday because the first one is busy."
        def rec(role, *blocks):
            return json.dumps({"record": "message", "role": role, "content": list(blocks)}) + "\n"
        t = lambda x: {"type": "text", "text": x}
        with open(os.path.join(sessions, "20990104T000000-3d4e5f6a.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "z", "kind": "web"}) + "\n")
            f.write(rec("user", t("Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\n" + first)))
            f.write(rec("assistant", {"type": "tool_use", "id": "t1", "name": "cal", "input": {}}))
            f.write(rec("user", {"type": "tool_result", "tool_use_id": "t1", "content": "ok"},
                        t(steer)))  # a sentence said mid-turn, folded beside the result
            f.write(rec("assistant", t(reply)))
            f.write(rec("user", t(later)))  # a later spoken turn: no block
            f.write(rec("assistant", t(later_reply)))
            f.write(json.dumps({"record": "spoken_direction", "turn": "x", "sentence": later_reply}) + "\n")
        for text in (steer, reply, later, later_reply):
            git(self.repo, "reset", "-q")
            self.stage("fixture.py", f"X = {text!r}\n")
            self.assertEqual(self.run_guard("--staged").returncode, 1, text)

    def test_a_barge_in_folded_into_a_rewrite_counts_and_one_word_directions_bind_nothing(self):
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions, exist_ok=True)
        typed = "Rebase the branch onto main and rerun the whole test suite."
        heard = "Book the boiler service for the morning of the twelfth."
        barge = "Wait, make that the afternoon of the twelfth instead please."
        t = lambda x: {"type": "text", "text": x}
        msg = lambda role, *b: {"role": role, "content": list(b)}
        typed_reply = "Sure. I rebased onto main and the suite is green again."
        with open(os.path.join(sessions, "20990106T000000-5f6a7b8c.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "v", "kind": "web"}) + "\n")
            for m in (msg("user", t(typed)), msg("assistant", t(typed_reply)),
                      msg("user", t("Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\n" + heard)),
                      msg("assistant", {"type": "tool_use", "id": "t1", "name": "cal", "input": {}}),
                      msg("user", {"type": "tool_result", "tool_use_id": "t1", "content": "ok"})):
                f.write(json.dumps({"record": "message", **m}) + "\n")
            # The barge-in, as the recorder writes it: folded into the tail,
            # and present only in the rewritten list.
            f.write(json.dumps({"record": "rewrite", "messages": [
                msg("user", t(typed)), msg("assistant", t(typed_reply)),
                msg("user", t("Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\n" + heard)),
                msg("assistant", {"type": "tool_use", "id": "t1", "name": "cal", "input": {}}),
                msg("user", {"type": "tool_result", "tool_use_id": "t1", "content": "ok"}, t(barge))]}) + "\n")
            f.write(json.dumps({"record": "spoken_direction", "turn": "x", "sentence": "Sure."}) + "\n")
        for text, want in ((barge, 1), (typed, 0), (typed_reply, 0)):
            git(self.repo, "reset", "-q")
            self.stage("fixture.py", f"X = {text!r}\n")
            self.assertEqual(self.run_guard("--staged").returncode, want, text)

    def test_every_title_counts_and_a_summary_never(self):
        # A title can be written from a spoken turn whichever turn opened the
        # chat (`title::due` renames at turns 1, 3 and 8; review of #559,
        # pass 6); a summary paraphrases typed and spoken turns alike (pass 4).
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions, exist_ok=True)
        spoken_title = "Planning the lighthouse open day with the harbour society"
        typed_title = "Refactoring the compaction cut and rerunning the benchmark suite"
        summary = "The owner asked to move the open day to the last weekend of June."
        voice = "Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\nhello"
        def user(text):
            return json.dumps({"record": "message", "role": "user",
                               "content": [{"type": "text", "text": text}]}) + "\n"
        with open(os.path.join(sessions, "20990105T000000-4e5f6a7b.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "w", "kind": "web", "title": spoken_title}) + "\n")
            f.write(user(voice))
            f.write(json.dumps({"record": "rewrite", "messages": [{"role": "user", "content": [{"type": "text",
                    "text": "[Earlier turns were compacted to fit the context window. What happened in them:]\n" + summary}]}]}) + "\n")
        with open(os.path.join(sessions, "20990105T000001-4e5f6a7c.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "u", "kind": "web", "title": typed_title}) + "\n")
            f.write(user("Refactor the cut, then rerun the benchmark."))
            f.write(user(voice))
        for text, want in ((spoken_title, 1), (typed_title, 1), (summary, 0)):
            git(self.repo, "reset", "-q")
            self.stage("fixture.py", f"X = {text!r}\n")
            self.assertEqual(self.run_guard("--staged").returncode, want, text)

    def test_a_rewrite_yields_only_a_fold_and_only_inside_a_stretch(self):
        # Pass 4 of #559: a compaction's carried state is new user text in a
        # rewrite and is not speech; a typed turn folded after a cancelled
        # tool turn has a barge-in's shape and is not speech either.
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions, exist_ok=True)
        carried = "[Live state, carried past the compaction and current as of now:]\nThe release branch has four commits awaiting the benchmark rerun."
        typed_fold = "Skip the flaky test for now and push the branch to origin."
        t = lambda x: {"type": "text", "text": x}
        msg = lambda role, *b: {"role": role, "content": list(b)}
        head = [msg("user", t("Tag the release once the benchmark has finished running.")),
                msg("assistant", {"type": "tool_use", "id": "t1", "name": "shell", "input": {}}),
                msg("user", {"type": "tool_result", "tool_use_id": "t1", "content": "ok"})]
        with open(os.path.join(sessions, "20990107T000000-6a7b8c9d.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "s", "kind": "web"}) + "\n")
            for m in head:
                f.write(json.dumps({"record": "message", **m}) + "\n")
            f.write(json.dumps({"record": "rewrite", "messages": head[:2] + [
                msg("user", {"type": "tool_result", "tool_use_id": "t1", "content": "ok"}, t(typed_fold))]}) + "\n")
            f.write(json.dumps({"record": "rewrite", "messages": [
                msg("user", t(carried)), msg("assistant", t("Carrying on with the release."))]}) + "\n")
            # A call later spoke into the chat, so it is read for spoken turns.
            f.write(json.dumps({"record": "message", "role": "user", "content": [t(
                "Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\nhi")]}) + "\n")
        for text in (carried.split("\n", 1)[1], typed_fold):
            git(self.repo, "reset", "-q")
            self.stage("fixture.py", f"X = {text!r}\n")
            self.assertEqual(self.run_guard("--staged").returncode, 0, text)

    def test_turns_after_a_mid_run_compaction_are_read_from_the_rewrite(self):
        # `Session::record_run` writes a run that compacted itself as one
        # rewrite: the compacted state plus every turn after the compaction,
        # which appear nowhere else (review of #559, pass 5).
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions, exist_ok=True)
        typed = "Rerun the benchmark on the release branch before the tag goes out."
        heard = "Find a free evening next week for dinner with the book club."
        reply = "Thursday evening is free next week, so the book club dinner fits there."
        t = lambda x: {"type": "text", "text": x}
        msg = lambda role, *b: {"role": role, "content": list(b)}
        voiced = msg("user", t("Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\n" + heard))
        call = msg("assistant", {"type": "tool_use", "id": "t1", "name": "cal", "input": {}})
        result = msg("user", {"type": "tool_result", "tool_use_id": "t1", "content": "ok"})
        with open(os.path.join(sessions, "20990109T000000-8c9d0e1f.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "c", "kind": "web"}) + "\n")
            for m in (msg("user", t(typed)), msg("assistant", t("Benchmark rerun and green.")), voiced, call, result):
                f.write(json.dumps({"record": "message", **m}) + "\n")
            head = msg("user", t(typed), t("\n\n[Earlier turns were compacted to fit the context window. What happened in them:]\nA benchmark rerun."))
            f.write(json.dumps({"record": "rewrite", "messages": [
                head, msg("assistant", t("Benchmark rerun and green.")), voiced, call, result,
                msg("assistant", t(reply))]}) + "\n")
        for text, want in ((reply, 1), (typed, 0)):
            git(self.repo, "reset", "-q")
            self.stage("fixture.py", f"X = {text!r}\n")
            self.assertEqual(self.run_guard("--staged").returncode, want, text)

    def test_the_documented_gap_a_later_undirected_turn_is_out_of_reach(self):
        # The trade spoken_turns makes, pinned so it cannot widen unseen: only
        # a call's first turn carries the voice block, so a later turn whose
        # reply was not directed is not in the corpus, either side. If this
        # starts failing, the gap narrowed — update the docstring with it.
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions, exist_ok=True)
        later = "Then remind me to water the tomatoes on Sunday evening please."
        later_reply = "I will remind you to water the tomatoes on Sunday evening."
        t = lambda x: {"type": "text", "text": x}
        with open(os.path.join(sessions, "20990110T000000-9d0e1f2a.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "g", "kind": "web"}) + "\n")
            for role, text in (("user", "Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\nhi"),
                               ("assistant", "Hello there."), ("user", later), ("assistant", later_reply)):
                f.write(json.dumps({"record": "message", "role": role, "content": [t(text)]}) + "\n")
        for text in (later, later_reply):
            git(self.repo, "reset", "-q")
            self.stage("fixture.py", f"X = {text!r}\n")
            self.assertEqual(self.run_guard("--staged").returncode, 0, text)

    def test_a_listen_tap_binds_nothing_and_a_summary_does_not_ride_with_a_turn(self):
        # A Listen tap records a direction for a typed reply read aloud
        # (`LISTEN_TURN`): neither it nor the typed turn it answers is
        # speech. And when a directed reply pulls in the turn it answers,
        # a compaction summary folded into that turn stays out (pass 6).
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions, exist_ok=True)
        typed = "Explain why the compaction cut must land on an assistant message."
        tapped = "Because an orphaned tool result after the cut is rejected by the provider."
        summary = "Earlier the owner profiled the release build and trimmed the slow tests."
        later = "And book the vet for the cat on the Friday after next please."
        reply = "The vet is booked for the cat on the Friday after next, in the morning."
        t = lambda x: {"type": "text", "text": x}
        with open(os.path.join(sessions, "20990111T000000-0e1f2a3b.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "l", "kind": "web"}) + "\n")
            for role, blocks in (("user", [t(typed)]), ("assistant", [t(tapped)]),
                                 ("user", [t("Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\nhi")]),
                                 ("assistant", [t("Hello there.")]),
                                 ("user", [t(later), t("\n\n[Earlier turns were compacted to fit the context window. What happened in them:]\n" + summary)]),
                                 ("assistant", [t(reply)])):
                f.write(json.dumps({"record": "message", "role": role, "content": blocks}) + "\n")
            f.write(json.dumps({"record": "spoken_direction", "turn": "listen:abc", "sentence": tapped}) + "\n")
            f.write(json.dumps({"record": "spoken_direction", "turn": "c1", "sentence": reply}) + "\n")
        for text, want in ((typed, 0), (tapped, 0), (summary, 0), (later, 1), (reply, 1)):
            git(self.repo, "reset", "-q")
            self.stage("fixture.py", f"X = {text!r}\n")
            self.assertEqual(self.run_guard("--staged").returncode, want, text)

    def test_a_picture_sent_without_words_closes_a_spoken_stretch(self):
        sessions = os.path.join(self.home, "sessions")
        os.makedirs(sessions, exist_ok=True)
        after = "That photo shows the release dashboard with every benchmark passing."
        t = lambda x: {"type": "text", "text": x}
        with open(os.path.join(sessions, "20990108T000000-7b8c9d0e.jsonl"), "w") as f:
            f.write(json.dumps({"record": "meta", "id": "p", "kind": "web"}) + "\n")
            for role, blocks in (("user", [t("Voice mode: everything you write is spoken aloud by a text-to-speech voice.\n\nhi")]),
                                 ("assistant", [t("Hello there.")]),
                                 ("user", [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}}]),
                                 ("assistant", [t(after)])):
                f.write(json.dumps({"record": "message", "role": role, "content": blocks}) + "\n")
        self.stage("fixture.py", f"X = {after!r}\n")
        self.assertEqual(self.run_guard("--staged").returncode, 0)

    def test_the_guard_excuses_its_own_file_in_a_push(self):
        base = git(self.repo, "rev-parse", "HEAD").strip()
        os.makedirs(os.path.join(self.repo, "scripts"))
        self.stage("scripts/check-private.py", f"# e.g. {SAID}\n")
        git(self.repo, "commit", "-q", "--no-verify", "-m", "guard")
        self.assertEqual(self.run_guard("--range", f"{base}..HEAD").returncode, 0)

    def test_a_name_with_an_apostrophe_already_in_the_repo_is_excused(self):
        persona = os.path.join(self.home, "personas", "quillon", "persona.toml")
        open(persona, "w").write('name = "quillon"\ndisplay = "Ann O\'Hara"\n')
        self.stage("a.md", "Written by Ann O'Hara.\n")
        git(self.repo, "commit", "-q", "--no-verify", "-m", "already")
        self.stage("b.md", "Ann O'Hara again.\n")
        self.assertEqual(self.run_guard("--staged").returncode, 0)

    def test_made_up_text_passes(self):
        self.stage("fixture.py", 'X = "The ferry leaves at noon."\n')
        self.assertEqual(self.run_guard("--staged").returncode, 0)

    def test_an_unreadable_range_fails_closed(self):
        r = self.run_guard("--range", "0123456789abcdef0123456789abcdef01234567..HEAD")
        self.assertEqual(r.returncode, 2, r.stdout)
        self.assertIn("refusing", r.stdout)

    def test_no_conversations_passes_and_says_so(self):
        self.stage("fixture.py", f"X = {SAID!r}\n")
        empty = os.path.join(self.tmp.name, "empty")
        os.makedirs(empty)
        r = self.run_guard("--staged", home=empty)
        self.assertEqual(r.returncode, 0)
        self.assertIn("no conversations", r.stdout)


if __name__ == "__main__":
    unittest.main()
