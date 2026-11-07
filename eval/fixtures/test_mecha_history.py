#!/usr/bin/env python3
"""The history task source (`mecha_history.py`), measured on a made-up
repository, a made-up bank and a fake rustup toolchain, so nothing here needs
cargo, the network or the owner's store. Run: python3 eval/fixtures/test_mecha_history.py
"""

import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = os.path.join(HERE, "mecha_history.py")
sys.path.insert(0, HERE)
import mecha_history  # noqa: E402

PIN_LIB = """pub fn add(a: u32, b: u32) -> u32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper() -> u32 {
        2
    }

    #[test]
    fn adds() {
        assert_eq!(add(1, helper()), 3);
    }
}
"""

MERGE_LIB = """pub fn add(a: u32, b: u32) -> u32 {
    a.saturating_add(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::u32::MAX;

    fn helper() -> u32 {
        2
    }

    #[test]
    fn adds() {
        assert_eq!(add(1, helper()), 3);
    }

    /// The top of the range does not wrap.
    #[test]
    fn saturates_at_the_top() {
        assert_eq!(add(MAX, helper()), MAX);
    }
}
"""

PIN_OTHER = """pub fn kept() -> u32 {
    1
}
"""

MERGE_OTHER = """pub fn kept() -> u32 {
    1
}

pub fn added_code() -> u32 {
    kept() + 1
}

/// A test at the top of an ordinary source file.
#[cfg(test)]
#[test]
fn top_level_test() {
    assert_eq!(added_code(), 2);
}
"""

NEW_TEST_FILE = """#[test]
fn a_whole_new_file() {
    assert!(true);
}
"""


def git(cwd, *args):
    return subprocess.run(["git", "-C", cwd, *args], capture_output=True, text=True, check=True).stdout


def read(path):
    with open(path) as fh:
        return fh.read()


def write(path, text):
    with open(path, "w") as fh:
        fh.write(text)


def executable(path, text):
    with open(path, "w") as fh:
        fh.write(text)
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)


class Source(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = self.tmp.name
        # The source repository: a pin, then a merge that changes the code and
        # adds tests both inside the module and as a new file.
        self.repo = os.path.join(root, "repo")
        os.makedirs(os.path.join(self.repo, "demo", "src"))
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.email", "t@example.com")
        git(self.repo, "config", "user.name", "t")
        write(os.path.join(self.repo, "demo", "src", "lib.rs"), PIN_LIB)
        write(os.path.join(self.repo, "demo", "src", "other.rs"), PIN_OTHER)
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "pin")
        self.pin = git(self.repo, "rev-parse", "HEAD").strip()
        write(os.path.join(self.repo, "demo", "src", "lib.rs"), MERGE_LIB)
        write(os.path.join(self.repo, "demo", "src", "other.rs"), MERGE_OTHER)
        os.makedirs(os.path.join(self.repo, "demo", "tests"))
        write(os.path.join(self.repo, "demo", "tests", "fresh.rs"), NEW_TEST_FILE)
        write(os.path.join(self.repo, "demo", "fixture.json"), '{"made": "up"}\n')
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "the change")
        self.merge = git(self.repo, "rev-parse", "HEAD").strip()
        # A fake rustup: one toolchain, and a `rustc` on PATH naming the host.
        self.bin = os.path.join(root, "bin")
        os.makedirs(self.bin)
        executable(os.path.join(self.bin, "rustc"), "#!/bin/sh\necho 'rustc 9.9.9'\necho 'host: test-triple'\n")
        self.rustup = os.path.join(root, "rustup")
        tc = os.path.join(self.rustup, "toolchains", "stable-test-triple", "bin")
        os.makedirs(tc)
        executable(os.path.join(tc, "cargo"), "#!/bin/sh\nexit 0\n")
        executable(os.path.join(tc, "rustc"), "#!/bin/sh\necho 'rustc 1.97.1 (made up)'\n")
        # A fake `mecha` that records what it was asked.
        self.calls = os.path.join(root, "mecha-calls")
        executable(os.path.join(self.bin, "mecha"), f"#!/bin/sh\necho \"$@\" >> {self.calls}\n")
        # The bank.
        self.bank = os.path.join(root, "bank")
        os.makedirs(os.path.join(self.bank, "prompts"))
        self.task = {
            "id": "pr-1", "pr": 1, "author": "history", "merge": self.merge, "pin": self.pin,
            "crates": ["demo"], "toolchain": {"channel": "stable", "version": "1.97.1"},
            "prompt": "prompts/pr-1.md",
            "repair": [{"path": "demo/fixture.json", "from": self.merge}],
            "admission": {"state": "admitted"},
        }
        self.other = dict(self.task, id="pr-2", prompt="prompts/pr-2.md", admission={"state": "interface-named"})
        for t in (self.task, self.other):
            write(os.path.join(self.bank, t["prompt"]), "# Make it saturate\n\nThe sum wraps.\n")
            t["prompt_check"] = {"sha256": mecha_history.sha256(os.path.join(self.bank, t["prompt"])), "store_present": True}
        self.write_bank()
        self.ws = os.path.join(root, "ws")

    def tearDown(self):
        self.tmp.cleanup()

    def write_bank(self):
        with open(os.path.join(self.bank, "tasks.json"), "w") as fh:
            json.dump({"tasks": [self.task, self.other]}, fh)

    def source(self, *args, extra=None, stdin=""):
        env = {
            "PATH": self.bin + os.pathsep + os.environ["PATH"],
            "HOME": self.tmp.name,
            "MECHA_HISTORY_REPO": self.repo,
            "MECHA_HISTORY_BANK": self.bank,
            "RUSTUP_HOME": self.rustup,
            "MECHA_BIN": os.path.join(self.bin, "mecha"),
            "MECHA_EXPERIMENT_WORKSPACE": self.ws,
            "MECHA_HISTORY_SKIP_CARGO": "1",
            **(extra or {}),
        }
        return subprocess.run([sys.executable, SOURCE, *args], env=env, input=stdin,
                              capture_output=True, text=True)

    # --- list ---

    def test_list_offers_only_an_admitted_task_with_its_checked_prompt(self):
        r = self.source("list")
        self.assertEqual(r.returncode, 0, r.stderr)
        tasks = json.loads(r.stdout)
        self.assertEqual([t["id"] for t in tasks], ["pr-1"])
        prompt = tasks[0]["prompt"]
        self.assertTrue(prompt.startswith("# Make it saturate"))
        self.assertIn(os.path.join(self.rustup, "toolchains", "stable-test-triple", "bin", "cargo"), prompt)
        self.assertIn("`demo`", prompt)

    def test_an_interface_named_task_is_offered_only_when_included_by_name(self):
        r = self.source("list", extra={"MECHA_HISTORY_INCLUDE": "interface-named"})
        self.assertEqual([t["id"] for t in json.loads(r.stdout)], ["pr-1", "pr-2"])

    def test_a_prompt_edited_after_its_check_is_not_offered(self):
        with open(os.path.join(self.bank, "prompts", "pr-1.md"), "a") as fh:
            fh.write("One more line, unchecked.\n")
        self.assertEqual(json.loads(self.source("list").stdout), [])
        self.task["prompt_check"]["store_present"] = False
        self.write_bank()
        self.assertEqual(json.loads(self.source("list").stdout), [])

    def test_a_stable_toolchain_that_moved_since_admission_refuses(self):
        self.task["toolchain"]["version"] = "1.90.0"
        self.write_bank()
        r = self.source("list")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("admitted on 1.90.0", r.stderr)

    # --- setup / teardown ---

    def test_setup_makes_a_clone_at_the_pin_that_cannot_see_the_merge(self):
        r = self.source("setup", "pr-1")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(git(self.ws, "rev-parse", "HEAD").strip(), self.pin)
        # The answer is not in the object store, no ref names anything, and
        # there is no remote to push to.
        missing = subprocess.run(["git", "-C", self.ws, "cat-file", "-e", self.merge], capture_output=True)
        self.assertNotEqual(missing.returncode, 0, "the merge commit is reachable from the run's clone")
        self.assertEqual(git(self.ws, "remote").strip(), "")
        self.assertEqual(git(self.ws, "for-each-ref").strip(), "")
        self.assertFalse(os.path.exists(os.path.join(self.ws, ".git", "FETCH_HEAD")))
        # Repaired, configured, and the setup's own files are not the run's diff.
        self.assertEqual(read(os.path.join(self.ws, "demo", "fixture.json")), '{"made": "up"}\n')
        config = read(os.path.join(self.ws, ".cargo", "config.toml"))
        self.assertIn("jobs = 2", config)
        self.assertIn(f'rustc = "{self.rustup}/toolchains/stable-test-triple/bin/rustc"', config)
        self.assertTrue(os.path.isdir(os.path.join(self.ws, ".cargo-home")))
        self.assertEqual(git(self.ws, "status", "--porcelain").strip(), "")
        # And the home-side marker names the clone.
        self.assertEqual(read(self.calls).split(), ["driven", "mark", "pr-1", "--clone", self.ws, "--for", "6h"])
        self.source("teardown", "pr-1")
        self.assertIn("driven unmark pr-1", read(self.calls))

    def test_setup_refuses_a_task_that_is_not_offered_and_a_used_workspace(self):
        r = self.source("setup", "pr-2")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("not offered", r.stderr)
        self.assertFalse(os.path.exists(self.calls))
        os.makedirs(self.ws)
        write(os.path.join(self.ws, "left-behind"), "x")
        r = self.source("setup", "pr-1")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("not empty", r.stderr)

    # --- the graft ---

    def test_the_hidden_tests_are_named_and_grafted_into_a_module_of_their_own(self):
        os.environ["MECHA_HISTORY_REPO"] = self.repo
        try:
            self.assertEqual(
                sorted(mecha_history.hidden_tests(self.merge)),
                [("demo", "lib", "saturates_at_the_top"), ("demo", "lib", "top_level_test"),
                 ("demo", "test:fresh", "a_whole_new_file")],
            )
            self.assertEqual(self.source("setup", "pr-1").returncode, 0)
            # A test the run wrote under the hidden test's own name.
            lib = os.path.join(self.ws, "demo", "src", "lib.rs")
            text = read(lib).replace(
                "    #[test]\n    fn adds()", "    #[test]\n    fn saturates_at_the_top() {}\n\n    #[test]\n    fn adds()")
            write(lib, text)
            mecha_history.graft(self.ws, self.merge)
        finally:
            del os.environ["MECHA_HISTORY_REPO"]
        grafted = read(lib)
        start = grafted.index("mod tests {")
        module = grafted.index("    mod hidden_grade {", start)
        self.assertLess(module, grafted.index("fn helper()"), "grafted after the module's own items")
        block = grafted[module:grafted.index("\n    }\n", module)]
        self.assertIn("use super::*;", block)
        self.assertIn("        use std::u32::MAX;", block)
        self.assertIn("        /// The top of the range does not wrap.", block)
        self.assertIn("        fn saturates_at_the_top() {", block)
        self.assertNotIn("fn adds()", block, "an item the pin already had is not grafted")
        self.assertEqual(grafted.count("fn saturates_at_the_top()"), 2)
        self.assertEqual(read(os.path.join(self.ws, "demo", "tests", "fresh.rs")), NEW_TEST_FILE)
        # A top-level `#[cfg(test)]` test in an ordinary file goes into a
        # test-only module at the end; the PR's non-test code never does.
        other = read(os.path.join(self.ws, "demo", "src", "other.rs"))
        tail = other[other.index("#[cfg(test)]\nmod hidden_grade {"):]
        self.assertIn("    fn top_level_test() {", tail)
        self.assertIn("    /// A test at the top of an ordinary source file.", tail)
        self.assertNotIn("added_code() -> u32", other)

    def test_a_test_module_ends_where_rustfmt_closes_it_not_where_braces_balance(self):
        lines = [
            "mod tests {",
            "    #[test]",
            "    fn opens_a_brace() {",
            '        let s = "{";',
            "    }",
            "}",
            "",
            "#[cfg(test)]",
            "mod held_tests {",
            "    #[test]",
            "    fn added_here() {",
            "        assert!(true);",
            "    }",
            "}",
            "",
            "#[cfg(test)]",
            "#[test]",
            "fn added_at_the_top() {}",
        ]
        self.assertEqual(mecha_history.test_modules(lines), [("tests", 0, 5), ("held_tests", 8, 13)])
        added = set(range(1, len(lines) + 1))
        self.assertEqual(
            [i.split("\n")[1].strip() for i in mecha_history.whole_items(lines, added, 9, 12, "    ")],
            ["fn added_here() {"])
        self.assertEqual(mecha_history.whole_items(lines, added, 15, 17, ""),
                         ["#[cfg(test)]\n#[test]\nfn added_at_the_top() {}"])

    def test_a_skip_for_want_of_a_backend_is_told_apart_from_a_failure(self):
        out = ("test tests::hidden_grade::needs_a_worker ... FAILED\n"
               "test tests::hidden_grade::asserts ... FAILED\n\nfailures:\n\n"
               "---- tests::hidden_grade::needs_a_worker stdout ----\n"
               "thread 'x' panicked at src/a.rs:1:1:\nthe worker is unavailable, and MECHA_TEST_REQUIRE_BACKENDS is set\n\n"
               "---- tests::hidden_grade::asserts stdout ----\n"
               "thread 'y' panicked at src/a.rs:2:2:\nassertion `left == right` failed\n\nfailures:\n")
        bodies = {p.split("::")[-1]: b for p, b in mecha_history.FAILURE.findall(out)}
        self.assertIn("MECHA_TEST_REQUIRE_BACKENDS", bodies["needs_a_worker"])
        self.assertNotIn("MECHA_TEST_REQUIRE_BACKENDS", bodies["asserts"])
        self.assertIn("assertion", bodies["asserts"])

    def test_libtest_output_is_read_per_test(self):
        out = ("test tests::adds ... ok\n"
               "test tests::hidden_grade::saturates_at_the_top ... FAILED\n"
               "test tests::saturates_at_the_top ... ok\n")
        seen = {}
        for path, verdict in mecha_history.RESULT.findall(out):
            seen.setdefault(path.split("::")[-1], []).append(verdict)
        self.assertEqual(seen["saturates_at_the_top"], ["FAILED", "ok"])


if __name__ == "__main__":
    unittest.main()
