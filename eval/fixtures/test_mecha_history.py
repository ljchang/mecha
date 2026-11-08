#!/usr/bin/env python3
"""The history task source (`mecha_history.py`), measured on a made-up
repository, a made-up bank and a fake rustup toolchain, so nothing here needs
cargo, the network or the owner's store. Run: python3 eval/fixtures/test_mecha_history.py
"""

import json
import os
import shutil
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

    struct Probe;

    impl Probe {
        fn top() -> u32 {
            MAX
        }
    }

    /// The top of the range does not wrap.
    #[test]
    fn saturates_at_the_top() {
        assert_eq!(add(Probe::top(), helper()), MAX);
    }

    #[test]
    fn keeps_its_text() {
        let s = "first
second";
        assert!(s.contains("\nsecond"));
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

/// Production code with a test-only branch in its body.
pub fn home() -> u32 {
    #[cfg(test)]
    {
        0
    }
    #[cfg(not(test))]
    {
        1
    }
}

/// A test at the top of an ordinary source file.
#[cfg(test)]
#[test]
fn top_level_test() {
    assert_eq!(added_code(), 2);
}
"""

PIN_THIRD = """pub fn three() -> u32 {
    3
}
"""

MERGE_THIRD = """pub fn three() -> u32 {
    3
}

/// The module is new, attributes and all.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_module_test() {
        assert_eq!(three(), 3);
    }
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


class CommittedBank(unittest.TestCase):
    """The bank this repository ships, not a made-up one: a task it admits and
    `list` does not offer is a pilot with nothing to run (review of #852,
    pass 2, where no task carried its baseline)."""

    def test_every_task_the_bank_admits_is_offered(self):
        path = os.path.join(HERE, "mecha_history", "tasks.json")
        with open(path) as fh:
            tasks = json.load(fh)["tasks"]
        admitted = [t for t in tasks if t["admission"]["state"] in mecha_history.OFFERED]
        self.assertTrue(admitted)
        old = mecha_history.BANK
        mecha_history.BANK = os.path.dirname(path)
        try:
            unoffered = [t["id"] for t in admitted if not mecha_history.offered(t)]
        finally:
            mecha_history.BANK = old
        self.assertEqual(unoffered, [])


class Regressions(unittest.TestCase):
    def test_a_test_item_is_one_whose_leading_attributes_say_so(self):
        self.assertTrue(mecha_history.is_test_item("/// doc\n#[cfg(test)]\n#[test]\nfn t() {}"))
        self.assertFalse(mecha_history.is_test_item("pub fn f() {\n    #[cfg(test)]\n    { 0 }\n}"))
        self.assertFalse(mecha_history.is_test_item("#[cfg(not(test))]\nfn f() {}"))

    def test_a_test_the_pin_already_fails_is_not_the_runs(self):
        self.assertEqual(mecha_history.counted({"a::slow": "FAILED", "a::ok": "ok"}, ["a::slow"], ["a::ok"]),
                         (True, "passed (1 failing on the pin too, not counted)"))

    def test_a_new_failure_is_the_runs(self):
        ok, detail = mecha_history.counted({"a::slow": "FAILED", "b::fresh": "FAILED"}, ["a::slow"], [])
        self.assertFalse(ok)
        self.assertEqual(detail, "1 failed: b::fresh")

    def test_a_test_that_must_pass_and_is_ignored_or_gone_is_the_runs(self):
        # PASS_TO_PASS, not "nothing FAILED" (review of #852, pass 7).
        for results in ({"a::kept": "ignored"}, {}):
            ok, detail = mecha_history.counted(results, [], ["a::kept"])
            self.assertFalse(ok, results)
            self.assertIn("did not pass (ignored, removed or not run): a::kept", detail)
        self.assertEqual(mecha_history.counted({"a::kept": "ok", "a::new": "ok"}, [], ["a::kept"]), (True, "passed"))

    def test_a_hidden_test_is_judged_once_by_the_hidden_check(self):
        m = "hidden_grade_0f"
        self.assertEqual(mecha_history.counted({f"x::{m}::t": "FAILED"}, [], [], m), (True, "passed"))
        self.assertEqual(mecha_history.counted({f"{m}::t": "FAILED"}, [], [], m), (True, "passed"))
        # A module the run named `hidden_grade` is the run's, not the grade's.
        self.assertFalse(mecha_history.counted({"x::hidden_grade::t": "FAILED"}, [], [], m)[0])


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
        write(os.path.join(self.repo, "demo", "src", "third.rs"), PIN_THIRD)
        write(os.path.join(self.repo, "demo", "src", "gone.rs"), "pub fn gone() {}\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "pin")
        self.pin = git(self.repo, "rev-parse", "HEAD").strip()
        write(os.path.join(self.repo, "demo", "src", "lib.rs"), MERGE_LIB)
        write(os.path.join(self.repo, "demo", "src", "other.rs"), MERGE_OTHER)
        write(os.path.join(self.repo, "demo", "src", "third.rs"), MERGE_THIRD)
        os.remove(os.path.join(self.repo, "demo", "src", "gone.rs"))
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
            path = os.path.join(self.bank, "baselines", f"{t['id']}.json")
            os.makedirs(os.path.dirname(path), exist_ok=True)
            write(path, '{"demo": ["tests::adds"]}\n')
            t["baseline"] = {"pin": self.pin, "merge": self.merge, "failing": {"demo": []},
                             "must_pass": f"baselines/{t['id']}.json",
                             "must_pass_sha256": mecha_history.sha256(path)}
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

    def test_an_interface_named_task_is_offered_when_its_prompt_names_the_interface(self):
        path = os.path.join(self.bank, "prompts", "pr-2.md")
        write(path, "# Make it saturate\n\nThe sum wraps.\n\n## Interface\n\n```rust\npub fn saturating(a: u8, b: u8) -> u8;\n```\n")
        self.other["prompt_check"]["sha256"] = mecha_history.sha256(path)
        self.write_bank()
        self.assertEqual([t["id"] for t in json.loads(self.source("list").stdout)], ["pr-1", "pr-2"])

    def test_a_rejected_or_unknown_state_is_never_offered(self):
        for state in ("rejected", "proposed"):
            self.other["admission"] = {"state": state}
            path = os.path.join(self.bank, "prompts", "pr-2.md")
            write(path, "# Make it saturate\n\n## Interface\n\n`fn saturating`\n")
            self.other["prompt_check"]["sha256"] = mecha_history.sha256(path)
            self.write_bank()
            self.assertEqual([t["id"] for t in json.loads(self.source("list").stdout)], ["pr-1"], state)

    def test_a_task_without_the_pins_own_failures_on_record_is_not_offered(self):
        for base in (None, {"pin": self.pin, "failing": {}}, {"pin": self.pin, "failing": {"other": []}},
                     {"failing": {"demo": []}}, {"pin": self.merge, "failing": {"demo": []}}):
            if base is None:
                self.task.pop("baseline")
            else:
                self.task["baseline"] = base
            self.write_bank()
            self.assertEqual(json.loads(self.source("list").stdout), [], base)

    def test_a_prompt_edited_after_its_check_is_not_offered(self):
        with open(os.path.join(self.bank, "prompts", "pr-1.md"), "a") as fh:
            fh.write("One more line, unchecked.\n")
        self.assertEqual(json.loads(self.source("list").stdout), [])
        self.task["prompt_check"]["store_present"] = False
        self.write_bank()
        self.assertEqual(json.loads(self.source("list").stdout), [])

    def test_a_stable_toolchain_that_moved_since_admission_drops_its_task_not_the_list(self):
        self.task["toolchain"] = {"channel": "stable", "version": "1.90.0"}
        self.write_bank()
        r = self.source("list")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout), [])
        self.assertIn("admitted on 1.90.0", r.stderr)
        self.assertIn("pr-1 is not offered", r.stderr)
        # The admitted version installed by its own name is used, whatever
        # `stable` has become (review of #852, pass 7).
        exact = os.path.join(self.rustup, "toolchains", "1.90.0-test-triple", "bin")
        os.makedirs(exact)
        executable(os.path.join(exact, "cargo"), "#!/bin/sh\nexit 0\n")
        executable(os.path.join(exact, "rustc"), "#!/bin/sh\necho 'rustc 1.90.0 (made up)'\n")
        tasks = json.loads(self.source("list").stdout)
        self.assertEqual([t["id"] for t in tasks], ["pr-1"])
        self.assertIn(exact, tasks[0]["prompt"])

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
                [("demo", "lib", "a_new_module_test"), ("demo", "lib", "keeps_its_text"),
                 ("demo", "lib", "saturates_at_the_top"), ("demo", "lib", "top_level_test"),
                 ("demo", "test:fresh", "a_whole_new_file")],
            )
            self.assertEqual(self.source("setup", "pr-1").returncode, 0)
            # A test the run wrote under the hidden test's own name.
            lib = os.path.join(self.ws, "demo", "src", "lib.rs")
            text = read(lib).replace(
                "    #[test]\n    fn adds()", "    #[test]\n    fn saturates_at_the_top() {}\n\n    #[test]\n    fn adds()")
            write(lib, text)
            mecha_history.graft(self.ws, self.merge, "hidden_grade_t")
        finally:
            del os.environ["MECHA_HISTORY_REPO"]
        grafted = read(lib)
        start = grafted.index("mod tests {")
        module = grafted.index("    mod hidden_grade_t {", start)
        self.assertLess(module, grafted.index("fn helper()"), "grafted after the module's own items")
        block = grafted[module:grafted.index("fn helper()", module)]
        self.assertIn("use super::*;", block)
        # The PR's import, said once from the graft's module (`added_imports`),
        # never also copied as an item: twice is E0252.
        self.assertEqual(block.count("use std::u32::MAX;"), 1)
        self.assertIn("\n    #[allow(unused_imports)] use std::u32::MAX;\n", block)
        # Placed verbatim, never re-indented.
        self.assertIn("\n    /// The top of the range does not wrap.\n", block)
        self.assertIn("\n    fn saturates_at_the_top() {\n", block)
        # A helper with no attribute of its own is still the PR's test code.
        self.assertIn("\n    impl Probe {\n", block)
        self.assertIn("\n    struct Probe;\n", block)
        # A multi-line string literal's lines are its value (review of #852, pass 5).
        self.assertIn('\n        let s = "first\nsecond";\n', block)
        self.assertNotIn("fn adds()", block, "an item the pin already had is not grafted")
        self.assertEqual(grafted.count("fn saturates_at_the_top()"), 2)
        self.assertEqual(read(os.path.join(self.ws, "demo", "tests", "fresh.rs")), NEW_TEST_FILE)
        # A top-level `#[cfg(test)]` test in an ordinary file goes into a
        # test-only module at the end; the PR's non-test code never does.
        other = read(os.path.join(self.ws, "demo", "src", "other.rs"))
        tail = other[other.index("#[cfg(test)]\nmod hidden_grade_t {"):]
        self.assertIn("\nfn top_level_test() {", tail)
        self.assertIn("\n/// A test at the top of an ordinary source file.", tail)
        self.assertNotIn("added_code() -> u32", other)
        # `#[cfg(test)]` in a body is not a test item (review of #852, pass 6).
        self.assertEqual(other.count("pub fn home()"), 0, "the run's tree never had it, and it is not grafted")
        # A test module the PR added whole is grafted once, as one item.
        third = read(os.path.join(self.ws, "demo", "src", "third.rs"))
        self.assertEqual(third.count("fn a_new_module_test()"), 1, third)
        self.assertEqual(third.count("mod hidden_grade_t {"), 1, third)
        tail = third[third.index("mod hidden_grade_t {"):]
        self.assertIn("\n/// The module is new, attributes and all.\n#[cfg(test)]\nmod tests {", tail)

    def with_home(self):
        home = os.path.join(self.tmp.name, ".mecha")  # where `source` (HOME=tmp) puts it
        old = os.environ.get("MECHA_HOME")
        os.environ["MECHA_HOME"] = home
        self.addCleanup(lambda: os.environ.__setitem__("MECHA_HOME", old) if old is not None
                        else os.environ.pop("MECHA_HOME", None))
        os.environ["MECHA_HISTORY_REPO"] = self.repo
        self.addCleanup(lambda: os.environ.pop("MECHA_HISTORY_REPO", None))

    def test_the_grade_puts_back_everything_setup_owns_before_any_cargo(self):
        # The run's tree is its one writable path, and `.git/info/exclude`
        # hides setup's files from `git status` (review of #852, pass 4).
        self.with_home()
        self.assertEqual(self.source("setup", "pr-1").returncode, 0)
        config = read(os.path.join(self.ws, ".cargo", "config.toml"))
        write(os.path.join(self.ws, ".cargo", "config.toml"), '[build]\nrustc = "/tmp/forge"\n')
        write(os.path.join(self.ws, ".cargo", "config"), '[target.x]\nrunner = "/tmp/forge"\n')
        write(os.path.join(self.ws, ".cargo-home", "config.toml"), '[build]\nrustc = "/tmp/forge"\n')
        write(os.path.join(self.ws, "demo", "fixture.json"), '{"made": "by the run"}\n')
        self.assertIsNone(mecha_history.restore_setup(self.ws, self.task))
        self.assertEqual(read(os.path.join(self.ws, ".cargo", "config.toml")), config)
        self.assertEqual(os.listdir(os.path.join(self.ws, ".cargo")), ["config.toml"])
        self.assertEqual(os.listdir(os.path.join(self.ws, ".cargo-home")), [])
        self.assertEqual(read(os.path.join(self.ws, "demo", "fixture.json")), '{"made": "up"}\n')
        # A build script rewriting `.cargo/` mid-grade is undone at the next call.
        write(os.path.join(self.ws, ".cargo", "config"), '[target.x]\nrunner = "/tmp/forge"\n')
        fake = os.path.join(self.tmp.name, "tc")
        os.makedirs(os.path.join(fake, "bin"))
        executable(os.path.join(fake, "bin", "cargo"), "#!/bin/sh\nexit 0\n")
        mecha_history.cargo(fake, self.ws, "test")
        self.assertEqual(os.listdir(os.path.join(self.ws, ".cargo")), ["config.toml"])
        # A vendored crate changed by a build script mid-grade ends the grade.
        os.makedirs(os.path.join(self.ws, "vendor", "dep"))
        write(os.path.join(self.ws, "vendor", "dep", "mid.rs"), "x\n")
        with self.assertRaises(mecha_history.TreeRefused):
            mecha_history.cargo(fake, self.ws, "test")
        shutil.rmtree(os.path.join(self.ws, "vendor"))
        # The vendored crates are compared, never trusted.
        os.makedirs(os.path.join(self.ws, "vendor", "dep", "src"))
        write(os.path.join(self.ws, "vendor", "dep", "src", "lib.rs"), "pub fn forged() {}\n")
        self.assertEqual(mecha_history.restore_setup(self.ws, self.task), "the vendored crates differ from setup's")

    def test_a_graft_that_dies_partway_leaves_the_tree_spent(self):
        self.with_home()
        self.assertEqual(self.source("setup", "pr-1").returncode, 0)
        os.remove(os.path.join(self.ws, "demo", "src", "other.rs"))  # the run deleted it
        first = self.source("grade", "pr-1", stdin="{}")
        # The run's doing, so a verdict on the contract, never a harness error.
        self.assertEqual(first.returncode, 0, first.stderr)
        verdict = json.loads(first.stdout)
        self.assertFalse(verdict["passed"])
        self.assertIn("other.rs", verdict["detail"])
        self.assertEqual(read(os.path.join(self.ws, "demo", "src", "lib.rs")).count("hidden_grade"), 0,
                         "nothing is grafted when the plan is refused")
        second = self.source("grade", "pr-1", stdin="{}")
        self.assertEqual(second.returncode, 2)
        self.assertIn("graded already", second.stderr)

    def test_a_grade_is_taken_once_and_its_record_is_out_of_the_runs_tree(self):
        home = os.path.join(self.tmp.name, ".mecha")
        old = os.environ.get("MECHA_HOME")
        os.environ["MECHA_HOME"] = home
        try:
            self.assertEqual(self.source("setup", "pr-1").returncode, 0)
            path = mecha_history.claim_path(self.ws)
            self.assertFalse(os.path.realpath(path).startswith(os.path.realpath(self.ws) + os.sep))
            mecha_history.claim(self.ws, "pr-1")
            with self.assertRaises(SystemExit):
                mecha_history.claim(self.ws, "pr-1")
            # Nothing the run can write in its tree stands in for the record.
            write(os.path.join(self.ws, ".git", "hidden-grade"), "x\n")
            self.assertTrue(os.path.exists(path))
            # A fresh setup at the same path is a fresh tree, not yet graded.
            self.assertEqual(self.source("teardown", "pr-1").returncode, 0)
            shutil.rmtree(self.ws)
            self.assertEqual(self.source("setup", "pr-1").returncode, 0)
            self.assertFalse(os.path.exists(path))
        finally:
            if old is None:
                del os.environ["MECHA_HOME"]
            else:
                os.environ["MECHA_HOME"] = old

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
        self.assertEqual(mecha_history.statuses(out, ["needs_a_worker", "asserts"]),
                         {"needs_a_worker": "skipped", "asserts": "fail"})

    def test_the_grafted_copy_decides_over_a_same_named_test_the_run_wrote(self):
        m = "hidden_grade_5e"
        out = ("test tests::saturates_at_the_top ... FAILED\n"
               f"test tests::{m}::saturates_at_the_top ... ok\n\nfailures:\n\n"
               "---- tests::saturates_at_the_top stdout ----\npanicked at src/lib.rs:1:1:\nthe run's own\n\n"
               "failures:\n\ntest result: FAILED. 1 passed; 1 failed\n")
        self.assertEqual(mecha_history.statuses(out, ["saturates_at_the_top"], module=m), {"saturates_at_the_top": "pass"})
        # A `hidden_grade` the run wrote itself, with the grafted copy cfg'd
        # out, is not the grade's copy (review of #852, pass 7).
        forged = ("test anywhere::hidden_grade::saturates_at_the_top ... ok\n"
                  "test tests::saturates_at_the_top ... ok\n\ntest result: ok. 2 passed\n")
        self.assertEqual(mecha_history.statuses(forged, ["saturates_at_the_top"], module=m),
                         {"saturates_at_the_top": "not-run"})
        self.assertEqual(mecha_history.statuses("error[E0425]: nope\n", ["x"]), {"x": "build-error"})

    def test_the_grade_builds_in_a_jail_with_no_network_and_no_home(self):
        # The argv is the policy; CI's runner has no bwrap to name, so the
        # lookup is stubbed and the assertions still run there.
        from unittest import mock
        with mock.patch.object(mecha_history.shutil, "which", return_value="/usr/bin/bwrap"):
            argv = mecha_history.jail_argv("/opt/tc", "/work/clone", {"PATH": "/usr/bin", "HOME": "/work/clone"})
        with mock.patch.object(mecha_history.shutil, "which", return_value=None), \
                self.assertRaises(SystemExit):
            mecha_history.jail_argv("/opt/tc", "/work/clone", {})
        self.assertIn("--unshare-net", argv)
        self.assertIn("--clearenv", argv)
        i = argv.index("--bind")
        self.assertEqual(argv[i + 1:i + 3], ["/work/clone", "/work/clone"])
        self.assertEqual(argv.count("--bind"), 1, "the clone is the only writable path")
        self.assertIn(["--ro-bind-try", "/opt/tc", "/opt/tc"], [argv[k:k + 3] for k in range(len(argv))])
        self.assertEqual(argv[-1], "--")
        # The real toolchain sits under the home: it alone of the home is bound.
        home = os.path.expanduser("~")
        tc = os.path.join(home, ".rustup", "toolchains", "stable-test-triple")
        with mock.patch.object(mecha_history.shutil, "which", return_value="/usr/bin/bwrap"):
            argv = mecha_history.jail_argv(tc, "/work/clone", {"HOME": "/work/clone"})
        under_home = {a for a in argv if a == home or a.startswith(home + os.sep)}
        self.assertEqual(under_home, {tc}, "nothing of the home but the toolchain")

    def test_the_runs_git_is_asked_only_inside_the_jail(self):
        # The run owns `.git/config` (an fsmonitor, a filter driver): any git
        # the grade asks of its tree runs confined (review of #852, pass 5).
        from unittest import mock
        seen = []
        with mock.patch.object(mecha_history.shutil, "which", return_value="/usr/bin/bwrap"), \
                mock.patch.object(mecha_history, "run",
                                  side_effect=lambda argv, **kw: seen.append(argv) or
                                  subprocess.CompletedProcess(argv, 0, " M a.rs\n?? b.rs\n", "")):
            self.assertEqual(mecha_history.jailed_status("/opt/tc", "/work/clone"), ["a.rs", "b.rs"])
        argv = seen[0]
        self.assertEqual(argv[0], "/usr/bin/bwrap")
        self.assertIn("--unshare-net", argv)
        self.assertEqual(argv[argv.index("--") + 1], "git")

    def test_libtest_output_is_read_per_test(self):
        out = ("test tests::adds ... ok\n"
               "test tests::hidden_grade::saturates_at_the_top ... FAILED\n"
               "test tests::saturates_at_the_top ... ok\n")
        seen = {}
        for path, verdict in mecha_history.RESULT.findall(out):
            seen.setdefault(path.split("::")[-1], []).append(verdict)
        self.assertEqual(seen["saturates_at_the_top"], ["FAILED", "ok"])


PIN_LISTED = """fn kept() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::{
        kept,
    };

    #[test]
    fn keeps() {
        assert_eq!(kept(), 1);
    }
}
"""

MERGE_LISTED = """fn kept() -> u32 {
    1
}

fn resident() -> u32 {
    2
}

#[cfg(test)]
mod tests {
    use super::{
        kept, resident,
    };

    #[test]
    fn keeps() {
        assert_eq!(kept(), 1);
    }

    #[test]
    fn the_resident_one_is_two() {
        assert_eq!(resident(), 2);
    }
}
"""


class Imports(unittest.TestCase):
    """`pr-726`'s shape (the driven pilot, 2026-11-08): the PR adds a private
    function, edits `mod tests`'s explicit `use super::{...}` list to name
    it, and a hidden test calls it. The graft sits one level inside `tests`,
    where `use super::*` reaches only what `tests` imports, so it must carry
    the PR's own import edit — or every independent solution fails on
    E0425, the model's code never having been the problem."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = os.path.join(self.tmp.name, "repo")
        os.makedirs(os.path.join(self.repo, "src"))
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.email", "t@example.com")
        git(self.repo, "config", "user.name", "t")
        write(os.path.join(self.repo, "src", "setup.rs"), PIN_LISTED)
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "pin")
        write(os.path.join(self.repo, "src", "setup.rs"), MERGE_LISTED)
        git(self.repo, "commit", "-q", "-am", "the change")
        self.merge = git(self.repo, "rev-parse", "HEAD").strip()
        # The run's tree: an independent solution, the function written and
        # the test module's import list left as the pin had it.
        self.tree = os.path.join(self.tmp.name, "tree")
        os.makedirs(os.path.join(self.tree, "src"))
        write(os.path.join(self.tree, "src", "setup.rs"),
              PIN_LISTED.replace("#[cfg(test)]", "fn resident() -> u32 {\n    2\n}\n\n#[cfg(test)]", 1))
        os.environ["MECHA_HISTORY_REPO"] = self.repo
        self.addCleanup(lambda: os.environ.pop("MECHA_HISTORY_REPO", None))

    def tearDown(self):
        self.tmp.cleanup()

    def history(self, name, pin, merge, path="src/setup.rs"):
        """A repository of its own with `pin` then `merge` at `path`, made the
        source; its merge commit."""
        repo = os.path.join(self.tmp.name, name)
        os.makedirs(os.path.dirname(os.path.join(repo, path)))
        git(repo, "init", "-q")
        git(repo, "config", "user.email", "t@example.com")
        git(repo, "config", "user.name", "t")
        write(os.path.join(repo, path), pin)
        git(repo, "add", "-A")
        git(repo, "commit", "-q", "-m", "pin")
        write(os.path.join(repo, path), merge)
        git(repo, "commit", "-q", "-am", "the change")
        os.environ["MECHA_HISTORY_REPO"] = repo
        return git(repo, "rev-parse", "HEAD").strip()

    def graft_block(self, merge, path="src/setup.rs", close="\n    }\n"):
        text = mecha_history.graft_plan(self.tree, merge, "hidden_grade_t")[path]
        start = text.index("mod hidden_grade_t {")
        return text[start:text.index(close, start)]  # the graft's module only, not the run's own

    def test_a_whole_test_files_top_level_import_edit_is_carried_too(self):
        # `hud/tests.rs` and twenty more are test code with no `mod tests`:
        # their items take the top-level path, which carried no imports
        # (review of #857, pass 3).
        pin = "use super::{kept};\n\n#[test]\nfn keeps() {\n    assert_eq!(kept(), 1);\n}\n"
        merge = ("use super::{kept, resident};\n\n#[test]\nfn keeps() {\n    assert_eq!(kept(), 1);\n}\n\n"
                 "#[test]\nfn the_resident_one_is_two() {\n    assert_eq!(resident(), 2);\n}\n")
        path = "src/x/tests.rs"
        merge_sha = self.history("whole-file", pin, merge, path)
        os.makedirs(os.path.dirname(os.path.join(self.tree, path)), exist_ok=True)
        write(os.path.join(self.tree, path), pin)
        self.assertEqual(mecha_history.added_imports(merge_sha, path, None), [("super::resident", ())])
        block = self.graft_block(merge_sha, path, close="\n}\n")
        self.assertNotIn("use super::{", block, "never copied as written")
        self.assertEqual(block.count("use super::super::resident;"), 1)
        # No grafted test names `kept`, and this file's `super` is another
        # file the graft cannot read, so it is not re-said — losing an unused
        # import costs nothing, and an unresolved one fails the file (pass 6).
        self.assertNotIn("super::super::kept", block)
        self.assertIn("fn the_resident_one_is_two()", block)

    def test_a_use_under_an_attribute_spread_over_lines_is_left_as_it_was(self):
        # An attribute this cannot read whole must not be dropped: the import
        # is left to the item, never also re-said ungated (review of #857,
        # pass 3).
        added = "    #[cfg(\n        unix\n    )]\n    use super::resident;\n\n"
        merge = self.history("multi-line-attr", PIN_LISTED,
                             MERGE_LISTED.replace("    use super::{\n        kept, resident,\n    };",
                                                  "    use super::{\n        kept,\n    };", 1)
                             .replace("    #[test]\n    fn keeps()", added + "    #[test]\n    fn keeps()", 1))
        self.assertEqual(mecha_history.added_imports(merge, "src/setup.rs", "tests"), [])
        block = self.graft_block(merge)
        self.assertNotIn("super::super::resident", block, "never re-said without its gate")
        self.assertIn("    #[cfg(\n        unix\n    )]\n    use super::resident;", block)

    def test_an_edit_to_a_test_modules_import_list_is_carried_into_the_graft(self):
        self.assertEqual(mecha_history.added_imports(self.merge, "src/setup.rs", "tests"),
                         [("super::resident", ())])
        text = mecha_history.graft_plan(self.tree, self.merge, "hidden_grade_t")["src/setup.rs"]
        block = text[text.index("mod hidden_grade_t {"):]
        # Said from one level deeper: `super` there is `tests`, not the file.
        self.assertIn("use super::super::resident;", block)
        self.assertNotIn("super::super::kept", block, "the pin already imported it")
        self.assertIn("fn the_resident_one_is_two()", block)

    def test_a_one_line_import_list_edited_is_never_copied_verbatim(self):
        # rustfmt keeps a short list on one line, so the whole `use` line is
        # added and reads as an item. Copied as it was, `super` would be
        # `tests` inside the graft: E0432 on `kept`, E0252 on `resident`
        # (review of #857, pass 1).
        one = lambda text: text.replace("use super::{\n        kept,\n    };", "use super::{kept};").replace(
            "use super::{\n        kept, resident,\n    };", "use super::{kept, resident};")
        merge = self.history("one-line", one(PIN_LISTED), one(MERGE_LISTED))
        write(os.path.join(self.tree, "src", "setup.rs"), one(read(os.path.join(self.tree, "src", "setup.rs"))))
        block = self.graft_block(merge)
        self.assertNotIn("use super::{", block, "a relative list is never copied into the graft as written")
        self.assertEqual(block.count("use super::super::resident;"), 1, "said once")
        self.assertIn("use super::super::kept;", block)
        self.assertIn("fn the_resident_one_is_two()", block)

    def test_a_use_under_a_doc_line_or_attribute_is_re_said_with_its_attributes(self):
        # `whole_items` starts an item at its doc line, so a doc-commented
        # `use` was both re-said and copied verbatim (review of #857, pass 2).
        # A `#[cfg]` gate must ride with the import it gates.
        added = ("    /// The resident one.\n    use super::resident;\n"
                 "    #[cfg(unix)]\n    use super::kept as unix_kept;\n\n")
        merge = self.history("attributed", PIN_LISTED,
                             MERGE_LISTED.replace("    #[test]\n    fn keeps()", added + "    #[test]\n    fn keeps()", 1))
        block = self.graft_block(merge)
        self.assertNotIn("\n    use super::resident;", block, "never copied as written")
        self.assertEqual(block.count("use super::super::resident;"), 1, "said once")
        self.assertIn("    #[cfg(unix)]\n    #[allow(unused_imports)] use super::super::kept as unix_kept;", block)
        self.assertNotIn("The resident one.", block, "a doc line is documentation, not part of the import")

    def test_an_import_only_an_edited_test_uses_is_not_grafted(self):
        # The PR adds `helper` for a test it *edited*, which is not grafted; an
        # independent solution has no reason to write `helper`, and an
        # unresolved `use` fails the whole file (review of #857, pass 5).
        # The new test goes ahead of the edited one, so the diff cannot pair
        # its closing brace with the edited test's and leave it half added.
        merge = self.history(
            "edited-test", PIN_LISTED,
            PIN_LISTED.replace("fn kept() -> u32 {\n    1\n}\n",
                               "fn kept() -> u32 {\n    1\n}\n\nfn helper() -> u32 {\n    1\n}\n\n"
                               "fn resident() -> u32 {\n    2\n}\n", 1)
            .replace("        kept,\n", "        helper, kept, resident,\n", 1)
            .replace("    #[test]\n    fn keeps() {\n        assert_eq!(kept(), 1);\n",
                     "    #[test]\n    fn the_resident_one_is_two() {\n        assert_eq!(resident(), 2);\n    }\n\n"
                     "    #[test]\n    fn keeps() {\n        assert_eq!(kept(), helper());\n", 1))
        self.assertEqual([p for p, _ in mecha_history.added_imports(merge, "src/setup.rs", "tests")],
                         ["super::helper", "super::resident"])
        block = self.graft_block(merge)
        self.assertIn("use super::super::resident;", block, "a grafted test names it")
        self.assertNotIn("helper", block, "only the edited, ungrafted test needed it")

    def test_a_one_line_list_cannot_carry_an_edited_tests_import_past_the_guard(self):
        # A one-line list is a wholly added line, so its paths came in as an
        # item and skipped `needed` (review of #857, pass 6).
        pin = PIN_LISTED.replace("    use super::{\n        kept,\n    };", "    use super::{kept};")
        merge = self.history(
            "one-line-edited", pin,
            pin.replace("fn kept() -> u32 {\n    1\n}\n",
                        "fn kept() -> u32 {\n    1\n}\n\nfn helper() -> u32 {\n    1\n}\n\n"
                        "fn resident() -> u32 {\n    2\n}\n", 1)
            .replace("    use super::{kept};", "    use super::{helper, kept, resident};", 1)
            .replace("    #[test]\n    fn keeps() {\n        assert_eq!(kept(), 1);\n",
                     "    #[test]\n    fn the_resident_one_is_two() {\n        assert_eq!(resident(), 2);\n    }\n\n"
                     "    #[test]\n    fn keeps() {\n        assert_eq!(kept(), helper());\n", 1))
        write(os.path.join(self.tree, "src", "setup.rs"),
              pin.replace("#[cfg(test)]", "fn resident() -> u32 {\n    2\n}\n\n#[cfg(test)]", 1))
        block = self.graft_block(merge)
        self.assertNotIn("helper", block, "the run never wrote it, and no grafted test names it")
        self.assertIn("use super::super::resident;", block)
        self.assertIn("use super::super::kept;", block, "the run's file declares it, so it resolves")

    def test_a_name_the_run_declared_only_inside_its_test_module_does_not_resolve(self):
        # `super::super` from the graft is the file root; a `helper` inside
        # `mod tests` passed a whole-file search and was E0432 (review of
        # #857, pass 7).
        merge = self.history(
            "nested-helper", PIN_LISTED,
            PIN_LISTED.replace("fn kept() -> u32 {\n    1\n}\n",
                               "fn kept() -> u32 {\n    1\n}\n\nfn helper() -> u32 {\n    1\n}\n\n"
                               "fn resident() -> u32 {\n    2\n}\n", 1)
            .replace("        kept,\n", "        helper, kept, resident,\n", 1)
            .replace("    #[test]\n    fn keeps() {\n        assert_eq!(kept(), 1);\n",
                     "    #[test]\n    fn the_resident_one_is_two() {\n        assert_eq!(resident(), 2);\n    }\n\n"
                     "    #[test]\n    fn keeps() {\n        assert_eq!(kept(), helper());\n", 1))
        write(os.path.join(self.tree, "src", "setup.rs"),
              PIN_LISTED.replace("#[cfg(test)]", "fn resident() -> u32 {\n    2\n}\n\n#[cfg(test)]", 1)
              .replace("    #[test]\n    fn keeps()", "    fn helper() -> u32 {\n        1\n    }\n\n    #[test]\n    fn keeps()", 1))
        block = self.graft_block(merge)
        self.assertNotIn("super::super::helper", block, "not at the root, so not reachable")
        self.assertIn("use super::super::resident;", block)

    def test_a_glob_is_decided_by_whether_its_module_resolves(self):
        # Pass 9: a glob left the guard before the resolve check, so a support
        # module the PR created and glob-imported was E0432 for every arm.
        run_file = "fn kept() {}\n\nmod support {\n    pub fn s() {}\n}\n"
        self.assertTrue(mecha_history.needed("super::*", "", run_file), "the module itself")
        self.assertTrue(mecha_history.needed("super::support::*", "", run_file), "the run's root has it")
        self.assertFalse(mecha_history.needed("super::helpers::*", "", run_file), "the PR's, never the run's")
        self.assertTrue(mecha_history.needed("crate::persona::tests_support::*", "", run_file),
                        "another file's head, kept on purpose")
        self.assertTrue(mecha_history.needed("super::helpers::*", "", ""), "a file this cannot read")

    def test_only_a_root_declaration_counts(self):
        self.assertTrue(mecha_history.needed("super::helper", "", "pub(crate) async fn helper() {}\n"))
        self.assertTrue(mecha_history.needed("super::helper", "", "use crate::x::{\n    helper,\n};\n"))
        self.assertFalse(mecha_history.needed("super::helper", "", "mod tests {\n    fn helper() {}\n}\n"))

    def test_a_crate_import_the_runs_file_declares_is_kept_though_unnamed(self):
        # A crate trait imported for its methods is named nowhere in a test
        # body; the run's file declaring it is what makes the import resolve.
        run_file = "trait Greet {\n    fn greet(&self);\n}\n"
        self.assertTrue(mecha_history.needed("super::Greet", "x.greet()", run_file))
        self.assertFalse(mecha_history.needed("super::helper", "x.greet()", run_file))
        self.assertTrue(mecha_history.needed("super::kept as k", "", "use super::{kept};\n"))

    def test_a_library_import_is_kept_and_a_crate_one_only_when_named(self):
        self.assertTrue(mecha_history.needed("std::io::Write", "fn t() { w.write_all(b\"x\") }"),
                        "a trait for its methods is named nowhere in the body")
        self.assertTrue(mecha_history.needed("super::*", ""))
        self.assertTrue(mecha_history.needed("super::a as b", "b()"))
        self.assertFalse(mecha_history.needed("crate::x::helper", "helpers()"), "a word, not a prefix")

    def test_a_use_declaration_flattens_to_the_paths_it_imports(self):
        self.assertEqual(
            mecha_history.use_paths("use super::{a, b::{c, d as e}, f::*, g::{self}};"),
            ["super::a", "super::b::c", "super::b::d as e", "super::f::*", "super::g"])
        self.assertEqual(mecha_history.use_paths("    pub(crate) use crate::x::Y; // why"), ["crate::x::Y"])
        self.assertIsNone(mecha_history.use_paths("use super::{a, b;"), "unbalanced: left alone")
        self.assertEqual(mecha_history.repoint("super::a"), "super::super::a")
        self.assertEqual(mecha_history.repoint("self::a"), "super::a")
        self.assertEqual(mecha_history.repoint("crate::a"), "crate::a")


if __name__ == "__main__":
    unittest.main()
