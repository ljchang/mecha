# The GitHub board — a HUD draft

Step 2 of `docs/GITHUB-DASHBOARD-DESIGN.md` §7: views 1 and 2 (failing now,
and new issues and outside pull requests), with the headline counts. No
summaries (step 4) and no links (step 3) yet.

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

**What "red" means.** The latest completed run of a workflow (by id, so a
rename stays one workflow) on the default branch failed, timed out or never
started, counting real CI only — `push`, `schedule`, a manual dispatch.
Dependabot's update runs (event `dynamic`) name a new workflow per update and
are not the branch's CI, so they are left out; counting them made one
repository look like forty failing workflows on the first real run.
