# The GitHub board — a HUD draft

Steps 2 to 5 of `docs/GITHUB-DASHBOARD-DESIGN.md` §7: views 1 and 2
(failing now, and new issues and outside pull requests), with the headline
counts, and view 3 (use: clones, views, release downloads, stars and package
downloads, each with a thirty-day strip). The New table's `GitHub` column is the typed reference (§4): on the
tailnet board it opens the issue, but only for a repository that is in the
collector's inventory and in `repos.toml`; a published copy shows it as text.
Its `about` and `summary` columns are step 4's summaries, which the
collector's quarantined pass writes and the loader only reads.

**Summaries.** A local model's one or two sentences about each body, and a
kind from a closed four (bug, question, request, other), made once per body
revision by `mecha github collect` — never at refresh. They are model prose
about a stranger's words. Where there is none the column says why: the
model's answer was refused, the issue has no description, or the collector
has not reached it yet (it summarises a few dozen a run, newest first).

It reads the collector's store (`mecha github collect`, daily per G10), which
the owner registers once in `~/.mecha/hud/sources.toml` — third-party,
because issue titles are strangers' text:

```toml
[source.github]
kind = "sqlite"
path = "~/.mecha/github/github.db"
content = "third-party"
```

Then:

```bash
mecha hud validate scripts/hud/github
mecha hud install scripts/hud/github     # installs as `github`
mecha hud refresh github                 # fill it now; mecha-hud.timer keeps it
```

and open `#hud/github` in the web app. A board installed from here is the
owner's from then on (`~/.mecha/hud/boards/github/`); a later revision of this
draft reaches it only through another install (or `mecha hud accept` of a
proposal), never by itself.

**Use.** A clone is not an install, and every figure is named for what it
counts. Traffic totals run from the day the collector first read the
repository (`clones_since`). Days no read covered are `missing_days`, not
zeros. A repository the login cannot push to shows a dash, because GitHub
shows traffic only to accounts that can. Package names come from each
repository's own `pyproject.toml`, `setup.cfg`, `package.json` or conda
recipe, and the Packages table shows which name is counted and where it came
from. To correct one, use the web app's Settings → GitHub (the picker, which
writes `~/.mecha/github/packages.toml`), or write the file by hand:

```toml
[[package]]
repo = "example-lab/tool-a"
registry = "pypi"          # pypi, npm or conda
name = "tool-a-py"

[[package]]
repo = "example-lab/tool-b"
registry = "npm"
none = true                # publishes nothing there
```

A name detected in a private repository is never sent to a registry; a name
written here is.

**What "red" means.** The latest completed run of a workflow (by id, so a
rename stays one workflow) on the default branch failed, timed out or never
started, counting real CI only — `push`, `schedule`, a manual dispatch.
Dependabot's update runs (event `dynamic`) name a new workflow per update and
are not the branch's CI, so they are left out; counting them made one
repository look like forty failing workflows on the first real run. The
allowlist is also a blind spot: CI that runs only on `merge_group`, `release`
or `repository_dispatch` is never red here, and a workflow deleted while
failing stays red until its runs age out of the 90-day history.
