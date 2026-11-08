#!/usr/bin/env python3
"""mecha's own merged history as a task source (docs/DRIVER-DESIGN.md §3, D3,
D3a, D3b), on the contract of docs/EXPERIMENT-DESIGN.md §21.1
(`source_stub.py` is its reference):

    list                 -> JSON list of {id, prompt, tags}: the admitted tasks
    setup <task>         -> a full clone of the repository at the task's pin, in
                            $MECHA_EXPERIMENT_WORKSPACE, ready for a jailed,
                            offline cargo; and the home-side driven marker
    grade <task>         -> stdin: the run's `--json` result; stdout:
                            {passed, detail, checks}: the PR's own added tests,
                            grafted over the run's tree, plus the touched
                            crates' unit suites for regressions (less the
                            tests the pin already fails, from `baseline`)

and four of its own, for the driver rather than `mecha exp`:

    teardown <task>      -> removes the driven marker (`mecha driven unmark`)
    validate <task> [r]  -> D3a: the hidden tests r times on the pin (each run
                            must fail an assertion) and r times on the merge
                            (each run must pass); one JSON line
    baseline <task> [r]  -> the touched crates' unit tests that already fail
                            on the pin, in the grade's jail, over r runs (2),
                            recorded in tasks.json; a task without one is not
                            offered
    check-prompts        -> D3b: `check-private.py --text` over every prompt,
                            with CHECK_PRIVATE_REQUIRE=1, recording each
                            prompt's sha256 in tasks.json

The bank is `mecha_history/tasks.json` (the seeded draw and one record per
task) and `mecha_history/prompts/` (written by the driver, never copied from
a PR or an issue). `list` offers a task whose admission state is
`admitted`, or `interface-named` with an `## Interface` section in its prompt
(the owner's ruling, 2026-11-07: the hidden tests call an item the pin lacks,
so its signature, never its body, is named), and whose prompt still has the
sha256 its privacy check recorded: a prompt edited after the check is not
offered. Any other state, `rejected` among them, is never offered.

**What setup builds, and why** (DRIVER-DESIGN.md §8, measured 2026-11-07
inside the owner's jail). `[sandbox]`'s bwrap clears the environment and
pins PATH to the system directories, so cargo is not on it and is run by
absolute path; and a cargo started that way does not find rustc, so the
clone's `.cargo/config.toml` names `rustc` and `rustdoc`. Dependencies are
vendored (on the host: `cargo vendor` fetches every platform's crates, so it
needs the network the run never has) and the run's CARGO_HOME is inside the
clone, because cargo writes there even offline and the host's `~/.cargo`
holds `credentials.toml`. `jobs = 2`, because four jobs linking mecha-core's
test binaries exceeded the jail's 8192 MB. And the touched crates' tests are
built once here, in the jail's exact environment, so a run's first cargo
call does not pay a cold build against a 120-second shell timeout. There
is no remote: from a local source a push needs neither credentials nor
network (§7), so the clone is made with no remote at all.
"""

import datetime
import hashlib
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
# MECHA_HISTORY_BANK is for this source's own tests, which bring a made-up bank.
BANK = os.environ.get("MECHA_HISTORY_BANK") or os.path.join(HERE, "mecha_history")
TASKS = os.path.join(BANK, "tasks.json")
SYSTEM_PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
MARKER_FOR = "6h"
FN = re.compile(r"\+\s*(?:pub )?(?:async )?fn (\w+)")
ATTR = re.compile(r"\+\s*(#\[|///|//)")
TEST_ATTR = re.compile(r"\+\s*#\[(tokio::)?test")
RESULT = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)", re.M)
FAILURE = re.compile(r"^---- (\S+) stdout ----\n(.*?)(?=^---- |^failures:$|\Z)", re.M | re.S)
GRAFT_MOD = "hidden_grade"  # a prefix: each grade adds a nonce (`graft_module`)


def graft_module():
    """The module a grade grafts into, named at grade time, after the run:
    a run that writes its own `mod hidden_grade` cannot stand in for it
    (review of #852, pass 7)."""
    return f"{GRAFT_MOD}_{secrets.token_hex(8)}"
OFFERED = {"admitted", "interface-named"}
INTERFACE = re.compile(r"^## Interface\s*$", re.M)
ITEM = re.compile(r"(#\[|///|(pub(\([^)]*\))? )?(async |unsafe |const )*(fn|struct|enum|trait|type|impl|const|static|use|mod|macro_rules!)\b)")


def die(msg, code=2):
    print(f"mecha_history: {msg}", file=sys.stderr)
    sys.exit(code)


def run(cmd, cwd=None, env=None, check=False):
    r = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
    if check and r.returncode != 0:
        die(f"`{' '.join(cmd[:4])}` failed: {(r.stderr or r.stdout).strip()[-400:]}")
    return r


def repo():
    """The source repository: MECHA_HISTORY_REPO, or the checkout this file is in."""
    path = os.environ.get("MECHA_HISTORY_REPO")
    if path:
        return path
    return run(["git", "-C", HERE, "rev-parse", "--show-toplevel"], check=True).stdout.strip()


def git(*args, cwd=None):
    return run(["git", "-C", cwd or repo(), *args], check=True).stdout


def bank():
    with open(TASKS) as fh:
        return json.load(fh)


def find(task_id):
    for t in bank()["tasks"]:
        if t["id"] == task_id:
            return t
    die(f"no task `{task_id}`")


def prompt_path(t):
    return os.path.join(BANK, t["prompt"])


def sha256(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


# --- the toolchain ---------------------------------------------------------


def toolchain_dir(t, commit):
    """The rustup toolchain a commit builds with: its own `rust-toolchain.toml`
    channel, or the task's recorded one for a pin from before the file. A
    recorded `version` is checked, so a `stable` that moved since admission
    refuses instead of grading on another compiler."""
    channel, want = None, None
    r = run(["git", "-C", repo(), "show", f"{commit}:rust-toolchain.toml"])
    if r.returncode == 0:
        m = re.search(r'^channel\s*=\s*"([^"]+)"', r.stdout, re.M)
        channel = m and m.group(1)
    if not channel:
        channel, want = t["toolchain"]["channel"], t["toolchain"].get("version")
    home = os.environ.get("RUSTUP_HOME") or os.path.expanduser("~/.rustup")
    host = run(["rustc", "-vV"], check=True).stdout
    triple = re.search(r"^host: (\S+)", host, re.M).group(1)
    d = os.path.join(home, "toolchains", f"{channel}-{triple}")
    exact = want and os.path.join(home, "toolchains", f"{want}-{triple}")
    if exact and os.path.isfile(os.path.join(exact, "bin", "cargo")):
        # The admitted version by its own name: immune to `stable` moving.
        d = exact
    elif not os.path.isfile(os.path.join(d, "bin", "cargo")):
        die(f"toolchain {channel} is not installed ({d}); `rustup toolchain install {channel}`")
    if want:
        got = run([os.path.join(d, "bin", "rustc"), "-V"], check=True).stdout.split()[1]
        if got != want:
            die(f"toolchain {channel} is {got} now, and this task was admitted on {want}; "
                f"`rustup toolchain install {want}` brings it back")
    return d


def jail_env(tree):
    """The environment a confined command gets (`Sandbox::bwrap_args`): cleared,
    PATH pinned, HOME the workspace, plus the one name `[sandbox] env` passes."""
    return {"PATH": SYSTEM_PATH, "HOME": tree, "CARGO_HOME": os.path.join(tree, ".cargo-home")}


def jail_argv(tc, tree, env):
    """bwrap as `Sandbox::bwrap_args` builds it for the owner's `[sandbox]`
    (no network, the system read-only, a private /tmp, the toolchain tree
    read-only, the workspace the only writable path, the environment
    cleared), up to and not including the command. Not the run's limits: no
    `systemd-run` scope wraps it, so `[sandbox] memory_mb` does not bound
    the grade, and its cargo calls pass `jobs=4`, which CARGO_BUILD_JOBS
    sets ahead of the clone's `[build] jobs = 2` (that setting is the run's,
    under its 8192 MB). The grade runs on the host's memory gate instead."""
    bwrap = shutil.which("bwrap", path="/usr/local/bin:/usr/bin:/bin")
    if not bwrap:
        die("bwrap is not installed, and a run's tree is never built unconfined")
    a = [bwrap, "--die-with-parent", "--new-session", "--unshare-user", "--unshare-pid",
         "--unshare-ipc", "--unshare-uts", "--unshare-cgroup-try", "--unshare-net"]
    for d in ("/usr", "/etc", "/opt"):
        if os.path.isdir(d):
            a += ["--ro-bind-try", d, d]
    for d in ("/bin", "/sbin", "/lib", "/lib32", "/lib64"):
        if os.path.islink(d):
            a += ["--symlink", os.readlink(d), d]
        elif os.path.exists(d):
            a += ["--ro-bind-try", d, d]
    a += ["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp",
          "--ro-bind-try", tc, tc, "--bind", tree, tree, "--chdir", tree, "--clearenv"]
    for k, v in env.items():
        a += ["--setenv", k, v]
    return a + ["--"]


RESTORE = {}  # tree -> (setup's cargo config, {repaired path: text}, vendor sha256), set by `grade`


def cargo(tc, tree, *args, jobs=None, require_backends=False, confined=False):
    """cargo in the jail's environment. `confined` runs it inside bwrap too,
    for a tree a model wrote: compiling one runs its build scripts, proc
    macros and tests, so `grade` does that with no network and no view of
    the host (DRIVER-DESIGN.md §8), where `setup` and `validate` build only
    the repository's own history."""
    env = jail_env(tree)
    if jobs:
        env["CARGO_BUILD_JOBS"] = str(jobs)
    if require_backends:
        env["MECHA_TEST_REQUIRE_BACKENDS"] = "1"
    argv = [os.path.join(tc, "bin", "cargo"), "--offline", *args]
    if os.path.realpath(tree) in RESTORE:
        # The run's build scripts execute during the grade, and could rewrite
        # setup's files between two of its cargo calls: they go back each
        # time, and a vendored crate changed since setup ends the grade.
        config, repairs, digest = RESTORE[os.path.realpath(tree)]
        put_back(tree, config, repairs)
        if vendor_digest(tree) != digest:
            raise TreeRefused("the vendored crates changed during the grade")
    if confined:
        argv = jail_argv(tc, tree, env) + argv
    return run(["nice", "-n", "19", *argv], cwd=tree, env=env)


# --- a tree at a commit ----------------------------------------------------


def prepare(t, tree, commit, cargo_too=True):
    """A full clone at `commit`, with no remote, repaired, and (unless told not
    to) vendored and configured for a jailed offline cargo. Returns the
    toolchain directory."""
    if os.path.exists(tree) and os.listdir(tree):
        die(f"{tree} is not empty; a task starts in a fresh clone")
    # Not `git clone`: a clone copies the source's branches, and `main` holds
    # the merge, so the run could `git show` the answer. Fetching the pin's
    # commit into an empty repository brings its whole history and nothing
    # after it, with no remote to push to (§7) and no ref naming the source.
    os.makedirs(tree, exist_ok=True)
    git("init", "-q", cwd=tree)
    git("fetch", "-q", "--no-tags", repo(), git("rev-parse", f"{commit}^{{commit}}").strip(), cwd=tree)
    git("checkout", "-q", "--detach", "FETCH_HEAD", cwd=tree)
    os.remove(os.path.join(tree, ".git", "FETCH_HEAD"))
    for fix in t.get("repair", []):
        dest = os.path.join(tree, fix["path"])
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "w") as fh:
            fh.write(git("show", f"{fix['from']}:{fix['path']}"))
    with open(os.path.join(tree, ".git", "info", "exclude"), "a") as fh:
        fh.write("\n# mecha_history setup\n/vendor/\n/.cargo/\n/.cargo-home/\n")
        fh.write("".join(f"/{fix['path']}\n" for fix in t.get("repair", [])))
    os.makedirs(os.path.join(tree, ".cargo-home"), exist_ok=True)
    tc = toolchain_dir(t, commit)
    vendored = ""
    if cargo_too:
        # On the host, with the host's own cargo cache: vendoring fetches every
        # platform's crates, which the run (offline) never could.
        r = run([os.path.join(tc, "bin", "cargo"), "vendor", "--locked", "vendor"], cwd=tree)
        if r.returncode != 0:
            die(f"cargo vendor failed: {r.stderr.strip()[-400:]}")
        vendored = r.stdout
    write_cargo_config(tree, vendored + "\n[build]\njobs = 2\n"
                       + f'rustc = "{tc}/bin/rustc"\nrustdoc = "{tc}/bin/rustdoc"\n')
    return tc


def write_cargo_config(tree, text):
    """`.cargo/` holds this one file and nothing else (a legacy `config`
    beside it would win), and `.cargo-home/` starts empty."""
    for d in (".cargo", ".cargo-home"):
        shutil.rmtree(os.path.join(tree, d), ignore_errors=True)
        os.makedirs(os.path.join(tree, d))
    with open(os.path.join(tree, ".cargo", "config.toml"), "w") as fh:
        fh.write(text)


def put_back(tree, config, repairs):
    write_cargo_config(tree, config)
    for path, text in repairs.items():
        dest = os.path.join(tree, path)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "w") as fh:
            fh.write(text)


def vendor_digest(tree):
    """sha256 over `vendor/`'s paths and bytes, or None when nothing is vendored."""
    root = os.path.join(tree, "vendor")
    if not os.path.isdir(root):
        return None
    h = hashlib.sha256()
    for d, dirs, files in os.walk(root):
        dirs.sort()
        for f in sorted(files):
            p = os.path.join(d, f)
            h.update(os.path.relpath(p, root).encode() + b"\0")
            if os.path.islink(p):
                h.update(b"link:" + os.readlink(p).encode())
            else:
                with open(p, "rb") as fh:
                    for chunk in iter(lambda: fh.read(1 << 20), b""):
                        h.update(chunk)
            h.update(b"\0")
    return h.hexdigest()


# --- the hidden tests ------------------------------------------------------


def hidden_tests(merge):
    """[(crate, target, name)] for every test fn the PR added; target is
    `lib`, `bin` or `test:<file>`."""
    out, cur = [], None
    diff = git("diff", "-U0", f"{merge}^1", merge, "--", "*.rs").split("\n")
    for i, line in enumerate(diff):
        m = re.match(r"\+\+\+ b/(.+)", line)
        if m:
            cur = m.group(1)
            continue
        m = FN.match(line)
        if not (m and cur):
            continue
        j, is_test = i - 1, False
        while j >= 0 and ATTR.match(diff[j]):
            is_test = is_test or bool(TEST_ATTR.match(diff[j]))
            j -= 1
        if not is_test:
            continue
        crate = cur.split("/")[0]
        if "/tests/" in cur:
            target = "test:" + os.path.splitext(os.path.basename(cur))[0]
        else:
            target = "bin" if crate == "mecha-cli" else "lib"
        out.append((crate, target, m.group(1)))
    return out


def test_modules(lines):
    """[(name, first line, last line)] (0-based, inclusive) for every
    top-level `mod <..>tests {` block. It ends at the first `}` alone in
    column 0, which is how rustfmt closes a top-level block: counting braces
    is wrong here, because a string or char literal can hold one unmatched
    (a `mod tests` in serve/chat.rs once measured as running past five later
    modules)."""
    out = []
    for i, line in enumerate(lines):
        m = re.match(r"mod (\w*tests)\s*\{\s*$", line)
        if not m:
            continue
        j = next((k for k in range(i + 1, len(lines)) if lines[k].rstrip() == "}"), len(lines) - 1)
        out.append((m.group(1), i, j))
    return out


def is_test_item(item):
    """`#[cfg(test)]` among the item's leading attributes and doc lines, never
    in its body: a production fn with a `#[cfg(test)] { … }` block inside is
    production code (review of #852, pass 6)."""
    for line in item.split("\n"):
        head = line.strip()
        if re.match(r"#\[cfg\(test\)\]$", head):
            return True
        if not (head.startswith("#[") or head.startswith("//") or not head):
            return False
    return False


def whole_items(lines, added, start, end, indent):
    """The items wholly added between `start` and `end` at `indent`: an
    attribute, doc line or item keyword begins one, and it runs to the line
    that closes it at its own indent (`}` or `};`, rustfmt's), or ends on the
    line that opens it, or at a `;` before anything opened."""
    items, i = [], start
    while i <= end and i < len(lines):
        if (i + 1) in added and lines[i].startswith(indent) and ITEM.match(lines[i][len(indent):]):
            j, opened = i, False
            while j < len(lines):
                line = lines[j].rstrip()
                if not opened and "{" in line:
                    opened = True
                    if line.count("{") == line.count("}"):
                        break  # opened and closed on one line
                    j += 1
                    continue
                if not opened and line.endswith(";"):
                    break
                if opened and line in (indent + "}", indent + "};"):
                    break
                j += 1
            if all((k + 1) in added for k in range(i, j + 1)):
                items.append("\n".join(lines[i:j + 1]))
            i = j + 1
            continue
        i += 1
    return items


def added_items(merge):
    """{path: [(module, indent, [item text])]}: the test items the PR added,
    whole (attributes, doc lines, body), from the merged file. `module` is the
    top-level test module (`mod tests`, `mod held_tests`, ...) they sit in, or
    None for a whole test file's items and for top-level `#[cfg(test)]` items
    in an ordinary source file."""
    out = {}
    # `d` drops a file the PR deleted: it has no merged text to read.
    for path in git("diff", "--name-only", "--diff-filter=d", f"{merge}^1", merge, "--", "*.rs").split():
        if path in new_test_files(merge):
            continue
        lines = git("show", f"{merge}:{path}").split("\n")
        added = set()
        diff = git("diff", "-U0", f"{merge}^1", merge, "--", path)
        for m in re.finditer(r"^@@ -\S+ \+(\d+)(?:,(\d+))? @@", diff, re.M):
            start, n = int(m.group(1)), int(m.group(2) or 1)
            added.update(range(start, start + n))
        groups = []
        whole_file = bool(re.search(r"(^|/)(\w+_)?tests\.rs$", path) or "/tests/" in path)
        # A module whose own header the PR added is new as a whole: it is one
        # top-level item, its attributes with it, never also its items
        # (review of #852, pass 3).
        modules = [m for m in test_modules(lines) if (m[1] + 1) not in added]
        for name, start, end in modules:
            items = whole_items(lines, added, start + 1, end - 1, "    ")
            if items:
                groups.append((name, "    ", items))
        inside = {k for _, start, end in modules for k in range(start, end + 1)}
        top = []
        i = 0
        while i < len(lines):
            if i in inside:
                i += 1
                continue
            items = whole_items(lines, added, i, i, "")
            if items:
                item = items[0]
                if whole_file or is_test_item(item):
                    top.append(item)
                i += item.count("\n") + 1
                continue
            i += 1
        if top:
            groups.append((None, "", top))
        if groups:
            out[path] = groups
    return out


def new_test_files(merge):
    return git("diff", "--name-only", "--diff-filter=A", f"{merge}^1", merge, "--", "*/tests/*.rs").split()


class TreeRefused(Exception):
    """The run's tree cannot take the grade (no place for the hidden tests,
    or setup's crates changed under it): the run's doing, so the grade
    answers `passed: false`, never a harness error."""


USE_DECL = re.compile(r"(pub(\([^)]*\))?\s+)?use\s")
USE_TOKEN = re.compile(r"::|[{},*]|\w+")


def use_statements(lines, start, end, indent):
    """The `use` declarations written at `indent` directly inside
    lines[start..end], each as its text (joined over the lines it spans)."""
    out, i = [], start
    while i <= end and i < len(lines):
        rest = lines[i][len(indent):] if lines[i].startswith(indent) else None
        if rest is not None and not rest.startswith(" ") and USE_DECL.match(rest):
            j = i
            while j <= end and not lines[j].split("//")[0].rstrip().endswith(";"):
                j += 1
            out.append("\n".join(lines[i:j + 1]))
            i = j + 1
            continue
        i += 1
    return out


def use_paths(text):
    """One `use` declaration flattened to the paths it imports, in order:
    `use super::{a, b::{c, d as e}, f::*};` is `super::a`, `super::b::c`,
    `super::b::d as e`, `super::f::*`. None when it does not parse — the
    caller then leaves that declaration exactly as the graft always did."""
    body = re.sub(r"//[^\n]*", "", text).strip()
    m = re.match(r"(?:pub(?:\([^)]*\))?\s+)?use\s+(.*);$", body, re.S)
    if not m:
        return None
    tokens = USE_TOKEN.findall(m.group(1))

    def tree(i, prefix):
        segs = []
        while True:
            tok = tokens[i]
            if tok == "{":
                i += 1
                out = []
                while tokens[i] != "}":
                    sub, i = tree(i, prefix + segs)
                    out += sub
                    if tokens[i] == ",":
                        i += 1
                return out, i + 1
            if tok == "*":
                return ["::".join(prefix + segs + ["*"])], i + 1
            if not re.match(r"\w+$", tok):
                raise ValueError(tok)
            segs.append(tok)
            i += 1
            if i < len(tokens) and tokens[i] == "::":
                i += 1
                continue
            path = prefix + segs
            if len(path) > 1 and path[-1] == "self":
                path = path[:-1]  # `a::{self}` imports `a`
            if i < len(tokens) and tokens[i] == "as":
                return ["::".join(path) + " as " + tokens[i + 1]], i + 2
            return ["::".join(path)], i

    try:
        paths, i = tree(0, [])
    except (IndexError, ValueError):
        return None
    return paths if i == len(tokens) else None


def module_imports(lines, module):
    """The paths `module`'s own `use` declarations import, in order, once
    each; a declaration that does not parse is left out."""
    out = []
    for name, start, end in test_modules(lines):
        if name != module:
            continue
        for text in use_statements(lines, start + 1, end - 1, "    "):
            for p in use_paths(text) or []:
                if p not in out:
                    out.append(p)
    return out


def added_imports(merge, path, module):
    """What the PR added to `module`'s imports in `path`: the paths its `use`
    declarations name at the merge and did not at the pin — a whole new `use`
    line and an edit to an existing one alike. `pr-726` added
    `ensure_resident` to `mod tests`'s `use super::{...}` list, the hidden
    tests call it, and a graft that carried only whole items failed every
    independent solution on E0425 (the driven pilot, 2026-11-08)."""
    merged = git("show", f"{merge}:{path}").split("\n")
    r = run(["git", "-C", repo(), "show", f"{merge}^1:{path}"])
    pinned = r.stdout.split("\n") if r.returncode == 0 else []  # a file the PR created
    before = set(module_imports(pinned, module))
    return [p for p in module_imports(merged, module) if p not in before]


def repoint(path):
    """An import written in a test module, said from the graft's own module
    one level inside it, where `super` is the test module."""
    if path == "super" or path.startswith("super::"):
        return "super::" + path
    if path.startswith("self::"):
        return "super::" + path[len("self::"):]
    return path


def plain_use(item):
    """The paths of an item that is a bare `use` declaration — its text
    starts at its first attribute or doc line, so one that starts with `use`
    has neither — or None for anything else."""
    return use_paths(item) if USE_DECL.match(item.lstrip(" ")) else None


def graft_plan(tree, merge, module):
    """{path: new text} for every file the graft writes, or TreeRefused.
    Whole new integration-test files are copied; added items in an existing
    test module go into a module of their own (`hidden_grade`), so a test the
    run wrote under the same name is not a compile error charged to the
    model. Items are placed verbatim, never re-indented: a multi-line string
    literal's lines are its value (review of #852, pass 5).

    Grafted into a test module, the hidden tests sit one level deeper than
    the PR wrote them, and `use super::*` reaches only what that module
    imports. So the imports the PR added to it (`added_imports`) are said
    again from the graft's module, re-pointed one level up (`repoint`); the
    bare `use` items they cover are not also copied, which would import a
    name twice (E0252). An explicit import outranks the glob, so nothing the
    run wrote can make one ambiguous."""
    writes = {}
    for path in new_test_files(merge):
        writes[path] = git("show", f"{merge}:{path}")
    for path, groups in added_items(merge).items():
        p = os.path.join(tree, path)
        if not os.path.isfile(p):
            raise TreeRefused(f"{path}: the run's tree has no such file to graft the hidden tests into")
        with open(p) as fh:
            src = fh.read().split("\n")
        # Bottom of the file first, so an insertion never moves a later one.
        for into, indent, items in sorted(groups, key=lambda g: g[0] is not None):
            inner = indent + "    "
            block = ["", f"{indent}#[cfg(test)]", f"{indent}mod {module} {{",
                     f"{inner}#[allow(unused_imports)]", f"{inner}use super::*;", ""]
            if into is not None:
                imports = added_imports(merge, path, into)
                items = [it for it in items
                         if not (plain_use(it) is not None and set(plain_use(it)) <= set(imports))]
                for p in imports:
                    block.append(f"{indent}#[allow(unused_imports)] use {repoint(p)};")
                if imports:
                    block.append("")
            block += "\n\n".join(items).split("\n") + [f"{indent}}}", ""]
            if into is None:
                src += block
                continue
            at = [i for i, l in enumerate(src) if re.match(rf"mod {into}\s*\{{", l)]
            if not at:
                raise TreeRefused(f"{path}: no `mod {into}` in the run's tree to graft into")
            src[at[-1] + 1:at[-1] + 1] = block
        writes[path] = "\n".join(src)
    return writes


def graft_apply(tree, writes):
    for path, text in writes.items():
        dest = os.path.join(tree, path)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "w") as fh:
            fh.write(text)


def graft(tree, merge, module):
    """Plan and apply; returns the names placed. Grafting twice would be a
    duplicate module; `cmd_grade` holds the once (`claim`)."""
    graft_apply(tree, graft_plan(tree, merge, module))
    return [n for _, _, n in hidden_tests(merge)]


def jailed_status(tc, tree):
    """The paths the run changed, by `git status` inside the grade's jail:
    the run owns `.git/config`, and an fsmonitor or a filter driver there
    would run a program of its choosing under any git on the host (review
    of #852, pass 5)."""
    env = jail_env(tree)
    r = run(jail_argv(tc, tree, env) + ["git", "-c", "core.fsmonitor=false", "status", "--porcelain",
                                         "--untracked-files=all"], cwd=tree, env=env)
    if r.returncode != 0:
        return None
    return [l[3:] for l in r.stdout.splitlines() if l.strip()]


def run_hidden(tc, tree, tests, jobs=None, why=None, confined=False, module=None):
    """{name: pass|fail|skipped|build-error|not-run}, one cargo call per target.

    Under MECHA_TEST_REQUIRE_BACKENDS=1: a hidden test that skips when its
    backend is absent (the monty worker, bwrap, an MCP server) would pass a
    run that did nothing, so its skip is made a failure and named `skipped`
    (DRIVER-DESIGN.md §8, the self-hosting confound). `why`, when given,
    collects the first build error or panic per test."""
    by_target, out = {}, {}
    for crate, target, name in tests:
        by_target.setdefault((crate, target), []).append(name)
    for (crate, target), names in sorted(by_target.items()):
        sel = {"lib": ["--lib"], "bin": ["--bin", "mecha"]}.get(target) or ["--test", target[5:]]
        r = cargo(tc, tree, "test", "-p", crate, *sel, "--", *names, jobs=jobs, require_backends=True,
                  confined=confined)
        # A whole new test file is the merge's own text, written by the graft;
        # a test grafted into the crate counts only under this grade's module.
        out.update(statuses(r.stdout + r.stderr, names, why, None if target.startswith("test:") else module))
    return out


def statuses(text, names, why=None, module=None):
    """Each hidden test's verdict from one cargo call's output. A test the
    run wrote under the same name is the run's, not the grade's: where the
    grafted copy (in `hidden_grade`) ran, only it counts."""
    seen, panics, out = {}, {}, {}
    for path, verdict in RESULT.findall(text):
        seen.setdefault(path.split("::")[-1], []).append((path, verdict))
    for path, body in FAILURE.findall(text):
        panics[path] = body
    built = "test result:" in text or seen
    for name in names:
        hits = seen.get(name, [])
        if module:
            # Only the copy the graft wrote decides; none reporting is not-run.
            hits = [h for h in hits if f"::{module}::" in f"::{h[0]}"]
        if not built:
            out[name] = "build-error"
            if why is not None:
                why.setdefault(name, first_error(text))
        elif not hits:
            out[name] = "not-run"
        elif all(v == "ok" for _, v in hits):
            out[name] = "pass"
        else:
            body = next((panics.get(p, "") for p, v in hits if v == "FAILED"), "")
            out[name] = "skipped" if "MECHA_TEST_REQUIRE_BACKENDS" in body else "fail"
            if why is not None:
                m = re.search(r"panicked at [^\n]*\n[^\n]*", body)
                why.setdefault(name, (m.group(0) if m else body.strip())[:300])
    return out


def first_error(text):
    errs = re.findall(r"^error.*", text, re.M)
    return errs[0][:200] if errs else text.strip()[-200:]


# --- verbs -----------------------------------------------------------------


def offered(t):
    state = t["admission"]["state"]
    if state not in OFFERED:
        return False
    check = t.get("prompt_check") or {}
    if not (check.get("store_present") is True and check.get("sha256") == sha256(prompt_path(t))):
        return False
    # Without the pin's own failures on record, the grade would charge a run with them.
    # Measured on this pin, like a prompt's check on its sha256.
    rec = t.get("baseline") or {}
    base = rec.get("failing")
    if rec.get("pin") != t["pin"] or not isinstance(base, dict) or set(base) != set(t["crates"]):
        return False
    if rec.get("merge") != t["merge"] or not rec.get("must_pass_sha256"):
        return False
    if not os.path.isfile(must_pass_path(t)) or sha256(must_pass_path(t)) != rec["must_pass_sha256"]:
        return False
    # A test that calls an item the pin lacks is unsolvable without its name.
    if state == "interface-named":
        with open(prompt_path(t)) as fh:
            return bool(INTERFACE.search(fh.read()))
    return True


def environment_note(t):
    tc = toolchain_dir(t, t["pin"])
    crates = ", ".join(f"`{c}`" for c in t["crates"])
    return (
        "\n\n## Environment\n\n"
        f"The current directory is a clone of the repository. Cargo is not on PATH: run it as "
        f"`{tc}/bin/cargo`. Dependencies are vendored and the build is offline. The change "
        f"belongs in {crates}, whose tests are already built. A shell command is stopped after "
        f"120 seconds, so run tests for one crate (`-p <crate>`) and, where you can, with a "
        f"name filter, rather than the whole workspace.\n"
    )


def cmd_list():
    out = []
    for t in bank()["tasks"]:
        if not offered(t):
            continue
        with open(prompt_path(t)) as fh:
            body = fh.read().rstrip()
        try:
            note = environment_note(t)
        except SystemExit:
            print(f"mecha_history: {t['id']} is not offered: its toolchain (above)", file=sys.stderr)
            continue
        out.append({"id": t["id"], "prompt": body + note,
                    "tags": [t["author"], t["admission"]["state"], *t["crates"]]})
    json.dump(out, sys.stdout)


def workspace():
    ws = os.environ.get("MECHA_EXPERIMENT_WORKSPACE")
    if not ws:
        die("MECHA_EXPERIMENT_WORKSPACE is not set")
    return os.path.abspath(ws)


def record_path(ws, kind):
    home = os.environ.get("MECHA_HOME") or os.path.expanduser("~/.mecha")
    key = hashlib.sha256(os.path.realpath(ws).encode()).hexdigest()[:32]
    return os.path.join(home, "driven", kind, key)


def save_setup(ws, t):
    """What setup made that the grade relies on, kept out of the run's jail:
    the cargo config, the vendored crates' digest."""
    with open(os.path.join(ws, ".cargo", "config.toml")) as fh:
        config = fh.read()
    p = record_path(ws, "setup")
    os.makedirs(os.path.dirname(p), mode=0o700, exist_ok=True)
    with open(p, "w") as fh:
        json.dump({"task": t["id"], "workspace": os.path.realpath(ws), "cargo_config": config,
                   "vendor_sha256": vendor_digest(ws)}, fh)


def restore_setup(ws, t):
    """Before the grade's first cargo call, everything setup owns is put back
    from its record: `.cargo/` (where `[build] rustc` or a `runner` would
    choose what prints the libtest lines read here), `.cargo-home/` (whose
    `config.toml` cargo also reads), and the repaired fixtures. `.git/info/
    exclude` hides all of them from `git status`, so an edit is never seen
    there (review of #852, pass 4). Returns None, or why the tree cannot be
    graded: the vendored crates are compared, never rebuilt (that needs the
    network)."""
    p = record_path(ws, "setup")
    if not os.path.exists(p):
        die("no setup record for this workspace; grade a tree `setup` made")
    with open(p) as fh:
        rec = json.load(fh)
    if rec.get("task") != t["id"]:
        die(f"this workspace was set up for {rec.get('task')}, not {t['id']}")
    repairs = {fix["path"]: git("show", f"{fix['from']}:{fix['path']}") for fix in t.get("repair", [])}
    RESTORE[os.path.realpath(ws)] = (rec["cargo_config"], repairs, rec.get("vendor_sha256"))
    put_back(ws, rec["cargo_config"], repairs)
    if vendor_digest(ws) != rec.get("vendor_sha256"):
        return "the vendored crates differ from setup's"
    return None


def claim_path(ws):
    """Where a workspace's grade is recorded as taken: in the mecha home, out
    of the run's jail, so a run cannot pre-empt its own grade (review of #852,
    pass 3: a file under the tree's `.git` was the run's to create)."""
    return record_path(ws, "graded")


def claim(ws, task_id):
    """A tree is graded once: a second graft would be a duplicate module and
    every hidden test a build error. Refuses a tree already graded."""
    p = claim_path(ws)
    if os.path.exists(p):
        die("this workspace has been graded already; grade a fresh setup")
    os.makedirs(os.path.dirname(p), mode=0o700, exist_ok=True)
    with open(p, "w") as fh:
        fh.write(json.dumps({"task": task_id, "workspace": os.path.realpath(ws)}) + "\n")


def mecha():
    return os.environ.get("MECHA_BIN", "mecha")


def cmd_setup(task_id):
    t = find(task_id)
    if not offered(t):
        die(f"task `{task_id}` is not offered (admission `{t['admission']['state']}`, or its prompt changed since its check)")
    ws = workspace()
    skip = os.environ.get("MECHA_HISTORY_SKIP_CARGO") == "1"  # this source's own tests
    tc = prepare(t, ws, t["pin"], cargo_too=not skip)
    if os.path.exists(claim_path(ws)):
        os.remove(claim_path(ws))  # a fresh tree at a reused path
    if not skip:
        for crate in t["crates"]:
            r = cargo(tc, ws, "test", "-p", crate, "--no-run", jobs=4)
            if r.returncode != 0:
                die(f"the pin's tests for {crate} do not build: {first_error(r.stderr)}")
    save_setup(ws, t)
    r = run([mecha(), "driven", "mark", t["id"], "--clone", ws, "--for", MARKER_FOR])
    if r.returncode != 0:
        die(f"could not write the driven marker: {(r.stderr or r.stdout).strip()}")
    json.dump({"task": t["id"], "workspace": ws, "cargo": f"{tc}/bin/cargo"}, sys.stdout)


def cmd_teardown(task_id):
    t = find(task_id)
    r = run([mecha(), "driven", "unmark", t["id"]])
    if r.returncode != 0:
        die(f"could not remove the driven marker: {(r.stderr or r.stdout).strip()}")


def cmd_grade(task_id):
    t = find(task_id)
    ws = workspace()
    try:
        json.load(sys.stdin)
    except ValueError:
        pass  # the verdict is the tests', never the run's own report
    tc = toolchain_dir(t, t["pin"])
    tests = hidden_tests(t["merge"])
    # Taken before the tree is touched: a graft that dies partway leaves the
    # tree spent, never open to a second graft (review of #852, pass 4).
    claim(ws, t["id"])
    refused = restore_setup(ws, t)
    if not refused:
        try:
            module = graft_module()
            writes = graft_plan(ws, t["merge"], module)
        except TreeRefused as e:
            refused = str(e)
    if refused:
        json.dump({"passed": False, "detail": refused,
                   "checks": [{"name": "the tree takes the grade", "passed": False, "detail": refused}]}, sys.stdout)
        return
    changed = jailed_status(tc, ws)
    graft_apply(ws, writes)
    names = [n for _, _, n in tests]
    try:
        hidden = run_hidden(tc, ws, tests, jobs=4, confined=True, module=module)
        checks = [{"name": f"hidden: {n}", "passed": v == "pass", "detail": v} for n, v in sorted(hidden.items())]
        pre, keep = t["baseline"]["failing"], must_pass(t)
        for crate in t["crates"]:
            results, why = regressions(tc, ws, crate)
            if results is None:
                ok, detail = False, why
            else:
                ok, detail = counted(results, pre.get(crate, []), keep.get(crate, []), module)
            checks.append({"name": f"regressions: {crate} {' '.join(selection(crate))}", "passed": ok, "detail": detail})
    except TreeRefused as e:
        json.dump({"passed": False, "detail": str(e), "changed": changed,
                   "checks": [{"name": "the tree takes the grade", "passed": False, "detail": str(e)}]}, sys.stdout)
        return
    passed = bool(names) and all(c["passed"] for c in checks)
    detail = (f"{sum(v == 'pass' for v in hidden.values())}/{len(hidden)} hidden tests pass"
              + ("; the run left the tree unchanged" if changed == [] else ""))
    # `changed` is diagnostic (the driver reads it); `passed` never does.
    json.dump({"passed": passed, "detail": detail, "checks": checks, "changed": changed}, sys.stdout)


def selection(crate):
    return ["--bin", "mecha"] if crate == "mecha-cli" else ["--lib"]


def regressions(tc, tree, crate):
    """({test path: ok|FAILED|ignored}, or None when nothing ran, and why)."""
    r = cargo(tc, tree, "test", "-p", crate, *selection(crate), "--no-fail-fast", jobs=4, confined=True)
    results = {}
    for path, verdict in RESULT.findall(r.stdout):
        # A path reported twice keeps its worst verdict.
        results[path] = verdict if results.get(path, "ok") == "ok" else results[path]
    if not results and r.returncode != 0:
        return None, first_error(r.stderr)
    return results, None


def counted(results, on_pin, keep, module=GRAFT_MOD):
    """PASS_TO_PASS: every test that passes on both the pin and the merge
    must be `ok` again; one ignored, deleted or cfg'd out is the run's, as a
    failure is (review of #852, pass 7). A failure the pin already has is
    the pin's; a grafted hidden test is judged by the hidden check, once."""
    hidden = {p for p in results if f"::{module}::" in f"::{p}"}
    failed = sorted(p for p, v in results.items() if v == "FAILED" and p not in hidden)
    known = [p for p in failed if p in on_pin]
    new = [p for p in failed if p not in on_pin]
    gone = sorted(p for p in keep if results.get(p) != "ok" and p not in new)
    why = []
    if new:
        why.append(f"{len(new)} failed: {', '.join(new[:5])}")
    if gone:
        why.append(f"{len(gone)} that pass on the pin and the merge did not pass "
                   f"(ignored, removed or not run): {', '.join(gone[:5])}")
    if why:
        return False, "; ".join(why)
    return True, "passed" + (f" ({len(known)} failing on the pin too, not counted)" if known else "")


def must_pass_path(t):
    return os.path.join(BANK, "baselines", f"{t['id']}.json")


def must_pass(t):
    with open(must_pass_path(t)) as fh:
        return json.load(fh)


def cmd_baseline(task_id, r):
    """The touched crates' unit tests, in the grade's own jail, r runs each on
    the pin and on the merge: the pin's failures (their union), which the
    grade does not charge a run with, and PASS_TO_PASS, the tests `ok` in
    every run on both, which the run must leave passing. The second is
    written to baselines/<task>.json, its sha256 recorded in tasks.json."""
    t = find(task_id)
    failing, both = {}, {}
    with tempfile.TemporaryDirectory(prefix="mecha-history-") as tmp:
        runs = {}
        for side, commit in (("pin", t["pin"]), ("merge", t["merge"])):
            tree = os.path.join(tmp, side)
            tc = prepare(t, tree, commit)
            for crate in t["crates"]:
                for _ in range(r):
                    results, why = regressions(tc, tree, crate)
                    if results is None:
                        die(f"{crate}'s unit tests do not run on the {side}: {why}")
                    runs.setdefault((side, crate), []).append(results)
        for crate in t["crates"]:
            pin_runs, merge_runs = runs[("pin", crate)], runs[("merge", crate)]
            failing[crate] = sorted({p for res in pin_runs for p, v in res.items() if v == "FAILED"})
            ok = lambda rs: set.intersection(*({p for p, v in res.items() if v == "ok"} for res in rs))
            both[crate] = sorted(ok(pin_runs) & ok(merge_runs))
    path = must_pass_path(t)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        json.dump(both, fh, indent=0)
        fh.write("\n")
    data = bank()
    for rec in data["tasks"]:
        if rec["id"] == t["id"]:
            rec["baseline"] = {"r": r, "at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), "by": "baseline",
                               "pin": t["pin"], "merge": t["merge"], "failing": failing,
                               "must_pass": os.path.relpath(path, BANK), "must_pass_sha256": sha256(path),
                               "must_pass_count": {c: len(v) for c, v in both.items()}}
    with open(TASKS, "w") as fh:
        json.dump(data, fh, indent=2, ensure_ascii=False)
        fh.write("\n")
    print(json.dumps({"task": t["id"], "failing": failing, "must_pass": {c: len(v) for c, v in both.items()}}))


def cmd_validate(task_id, r):
    """D3a, on clones this verb makes and removes."""
    t = find(task_id)
    tests = hidden_tests(t["merge"])
    res = {"task": t["id"], "r": r, "tests": [n for _, _, n in tests]}
    if not tests:
        res["verdict"] = "reject: the PR adds no test"
        print(json.dumps(res))
        return
    with tempfile.TemporaryDirectory(prefix="mecha-history-") as tmp:
        pin_tree, merge_tree = os.path.join(tmp, "pin"), os.path.join(tmp, "merge")
        pin_why, merge_why = {}, {}
        tc = prepare(t, pin_tree, t["pin"])
        module = graft_module()
        try:
            graft(pin_tree, t["merge"], module)
        except TreeRefused as e:
            res["verdict"] = f"graft-failed: {e}"
            print(json.dumps(res))
            return
        pin = [run_hidden(tc, pin_tree, tests, jobs=4, why=pin_why, confined=True, module=module)
               for _ in range(r)]
        tc_m = prepare(t, merge_tree, t["merge"])
        merge = [run_hidden(tc_m, merge_tree, tests, jobs=4, why=merge_why, confined=True) for _ in range(r)]
    res["pin"], res["merge"], res["pin_why"], res["merge_why"] = pin, merge, pin_why, merge_why
    if any("skipped" in p.values() for p in pin + merge):
        res["verdict"] = "reject: a hidden test skips here for want of its backend, so it would pass a run that did nothing"
    elif any(set(p.values()) & {"not-run"} for p in pin):
        res["verdict"] = "graft-failed: a hidden test ran nothing on the pin"
    elif any("build-error" in p.values() for p in pin):
        res["verdict"] = "needs-interface: the hidden tests do not build on the pin"
    elif any(set(m.values()) != {"pass"} for m in merge):
        res["verdict"] = "reject: the merge does not pass its own tests every time"
    elif all("fail" in p.values() for p in pin):
        res["verdict"] = "admit"
    elif any("fail" in p.values() for p in pin):
        res["verdict"] = "reject: the grade fails on the pin only sometimes"
    else:
        res["verdict"] = "reject: the grade passes on the pin, so it cannot tell a fix from none"
    print(json.dumps(res))


def cmd_check_prompts():
    """D3b: every prompt through check-private's text mode against a store that
    must be present; a match is rewritten, never admitted with an exception."""
    guard = os.path.join(repo(), "scripts", "check-private.py")
    data, failed = bank(), []
    for t in data["tasks"]:
        p = prompt_path(t)
        if not os.path.exists(p):
            t.pop("prompt_check", None)
            continue
        r = run([sys.executable, guard, "--text", p], cwd=repo(), env=dict(os.environ, CHECK_PRIVATE_REQUIRE="1"))
        if r.returncode != 0:
            t.pop("prompt_check", None)
            failed.append(f"{t['id']}: {r.stdout.strip()}")
        else:
            t["prompt_check"] = {"sha256": sha256(p), "store_present": True}
    with open(TASKS, "w") as fh:
        json.dump(data, fh, indent=2, ensure_ascii=False)
        fh.write("\n")
    if failed:
        die("prompts refused:\n" + "\n".join(failed), code=1)


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    verb, rest = argv[0], argv[1:]
    if verb == "list":
        cmd_list()
    elif verb == "setup" and rest:
        cmd_setup(rest[0])
    elif verb == "grade" and rest:
        cmd_grade(rest[0])
    elif verb == "teardown" and rest:
        cmd_teardown(rest[0])
    elif verb == "validate" and rest:
        cmd_validate(rest[0], int(rest[1]) if len(rest) > 1 else 5)
    elif verb == "baseline" and rest:
        cmd_baseline(rest[0], int(rest[1]) if len(rest) > 1 else 2)
    elif verb == "check-prompts":
        cmd_check_prompts()
    else:
        die(f"unknown verb `{verb}`")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
