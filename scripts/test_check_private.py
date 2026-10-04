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
        open(os.path.join(persona, "persona.toml"), "w").write('name = "quillon"\n')
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

    def test_a_sentence_split_across_two_lines_is_not_excused(self):
        # The repository "says" a run only on one line: words that meet only
        # across a line break, or a file boundary, excuse nothing.
        half = SAID.split()
        self.stage("a.md", " ".join(half[:4]) + "\n" + " ".join(half[4:]) + "\n")
        git(self.repo, "commit", "-q", "--no-verify", "-m", "split")
        self.stage("b.py", f"Y = {SAID!r}\n")
        self.assertEqual(self.run_guard("--staged").returncode, 1)

    def test_a_commit_message_is_checked(self):
        msg = os.path.join(self.tmp.name, "COMMIT_EDITMSG")
        open(msg, "w").write(f"Fix the fixture\n\nIt said: {SAID}\n# a git comment\n")
        r = self.run_guard("--message", msg)
        self.assertEqual(r.returncode, 1, r.stdout)
        self.assertIn("commit message:3", r.stdout)
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
        self.assertIn("wip.py:1", r.stdout)

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
