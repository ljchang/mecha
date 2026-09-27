---
title: Switching models
sidebar_position: 2.5
description: One local model loaded at a time, chosen by loading it — every surface follows the switch, a switch waits for the runs using the old model, and each run records the model that answered it.
---

# Switching models

With llama-server in **router mode**, one port offers several chat models and
loads whichever one a request names. mecha builds its model switching on
that, with one rule: **the model the router has loaded *is* the choice.**
`mecha model use gemma26` loads Gemma, and from then on every run that doesn't
name a model uses Gemma — in the terminal, in web chat, in voice, in Slack, in
the nightly passes. No restart is needed, and there is no "current model"
setting to edit.

There is no second record of the pick on purpose. A stored setting is a second
answer that can disagree with what the server actually loaded. Asking the
router is the same rule as asking a server what it serves (`/props`) rather
than trusting the config.

## Setting it up

1. **Run llama-server as a router.** `scripts/start-router.sh` starts it with
   no `-m` and a generated `--models-preset`. Each model gets a section with
   its own flags (context, projector, sampling, reasoning budget), so
   switching never mistunes a model. `--models-max 1` keeps one chat model in
   memory at a time. The embedding server stays a separate process, so an
   embedding request can never evict the chat model.
2. **Give each model its own provider entry** on the same `base_url`. The
   entry's `model` must be the preset's section name: in router mode that
   string is what selects the model.
3. **Mark the default entry `follow_loaded = true`.** A run that takes the
   default provider then resolves to whichever sibling entry names the loaded
   model.

```toml
default_provider = "local"

[providers.local]
kind = "local"
base_url = "http://127.0.0.1:8080"
model = "qwen3.6-35b-a3b"
context_window = 262144            # -c 1048576 / -np 4
follow_loaded = true

[providers.gemma26]
kind = "local"
base_url = "http://127.0.0.1:8080"
model = "gemma-4-26b-a4b"
context_window = 32768             # -c 32768 / -np 1
```

A followed run takes **everything** from the sibling entry: its
`context_window`, sampling, prices, `fallbacks` and retries, not only its
model. So each entry needs the settings of the model it names — the four
numbers in [Serving a local model](/docs/features/models/serving#four-numbers-that-have-to-agree)
apply per entry. The keys are in the
[configuration reference](/docs/reference/configuration).

## Switching

```bash
mecha model                   # what each router serves, and what is loaded (●)
mecha model use gemma26       # by provider entry
mecha model use gemma-4-26b-a4b   # or by the router's model name
```

`use` loads the model and waits until it is ready. A load runs at disk speed:
33–39 s from cold, about 9 s when the file is already in the page cache. If
the new model fails to come up, the one it replaced is loaded back.

It refuses a model whose preset temperature disagrees with its provider entry.
An entry that sets `temperature` sends it on every request, so the mismatch
would silently retune the model on every call; the refusal names the entry to fix.

## Who follows

**One model serves every surface, and a switch from any of them is a switch
for all of them.**

- **Long-lived surfaces check before every turn.** `mecha serve` (web chat
  and its voice calls), `mecha voice-serve`, the Slack connector and the
  trigger daemon ask the router which model is loaded at the start of each
  turn or each fire. They rebuild their agent only when the loaded model
  changed.
- **A one-shot command reads the router once, when it starts.** `mecha run`,
  `batch`, the nightly `learn` / `validate` / `harness ruminate` passes: each
  is one model from start to finish.
- **A named provider is a pin, and never moves.** `--provider`, `--model`, a
  trigger's `provider`, an experiment arm. In router mode a pin is also a
  *load*: naming a model that is not loaded loads it, and later default runs
  follow it. `mecha model use` is the deliberate way to do the same thing.
- **`mecha eval` never follows.** A scorecard grades the model it names, and
  two scorecards taken a week apart must not be different models under one
  condition.
- **Not yet: `mecha chat` and the TUI.** They read the router once at start,
  like a one-shot command, but they don't re-check per turn and a switch
  doesn't wait for them. Their next turn loads their model back. `/model` in
  the TUI is a pin, and on a router a pin is a load: it switches the model for
  the whole machine, without waiting for the runs still on the old one. Use
  `mecha model use` instead. In `mecha chat`, `/model` only shows the active
  model.

A router in an ambiguous state — mid-swap, or with a model loaded that no
entry names, or that two entries name — does not move a long-lived surface.
It keeps the model it had, because falling back to the default would load
production over your pick. A one-shot command has no earlier model to keep:
it warns and runs on the default. `mecha model list` flags any entry that
names a model its router doesn't serve.

## A switch waits for runs in progress

The router itself only protects single *requests*: it never evicts a model
mid-reply. But a run is many requests, and between two of them the model sits
idle. A switch at that moment would succeed, and the run's next request would
load the old model straight back. So mecha makes **a switch wait until no run
is using the model**, and a run is answered by one model from start to finish.

Each run on a router takes a *hold*, a small file in `~/.mecha/holds/`, and
drops it when it ends. `mecha model use` waits until no hold remains, and
tells you what it is waiting for:

```
waiting for 2 run(s) to finish: web chat, trigger morning-brief (--now stops them, Ctrl-C cancels the switch)
```

- **A run that starts during the wait waits for the switch**, then starts on
  the new model. Web chat says so on the page (*Switching the model to X —
  this turn starts once it is loaded*), and a command says so on stderr.
- **There is no time limit on the wait**, by design: a long delegated task
  finishes on the model it started with. `--wait-secs` bounds only the
  *load* after the wait.
- **`--now` stops the runs instead.** Each one is asked to stop at its next
  safe point, as Ctrl-C would stop it, and gets 15 seconds before the switch
  goes ahead. A reply in progress ends early.
- **Ctrl-C cancels the switch**, and the loaded model stays.
- **`mecha model cancel-switch` withdraws a stuck switch**: one whose
  `mecha model use` is gone, or whose file can't be read. Every run on that
  router would otherwise wait for it.

Only one switch can be pending on a router at a time. A second
`mecha model use` is refused and names the first. `cancel-switch` withdraws
every pending switch, on every router.

**One way to wedge it:** a run whose agent starts another run through
`shell` — a chat turn that calls `mecha run …`, say. With a switch pending,
the child run waits for the switch, the switch waits for the parent, and the
parent waits for its child. Nothing breaks the cycle except `--now` or
`cancel-switch`. The waiting message names the run that's holding things up,
which is where to look.

## What gets recorded

Every run records the model that answered it. In router mode that record is
exact rather than hopeful, because the `model` field is what selected the
model. A conversation that continues across a switch records a fresh config
line before its next turn, so each turn names its own model.

That record is what makes it safe for background work to follow your pick.
The nightly passes run on whatever is loaded, and readers of the
[run-quality corpus](/docs/features/learning/run-quality) slice by the model
that answered rather than trusting the scheduler to keep models apart. Rule
retirement counts only evidence measured on the model currently in use, so
one model's results never convict a rule on another's behalf.

## Who may switch

Only you: `mecha model use` at the terminal. **There is no tool for it.** No
model is handed a way to choose the model, because an injection that could
move the machine onto a different model would be choosing who reads every
later turn.

"No tool" isn't the same as "unreachable", though: a run with unconfined
`shell` could type `mecha model use`, and that command meets your approval
rules like any other. To close that route, forbid the prefix:

```toml
[[rule]]
tool = "shell"
pattern = ["mecha", "model", "use"]
decision = "forbid"
match = ["mecha model use gemma26"]
justification = "Only the owner switches the model."
```

Switching from the web chat's model chip is designed and not built yet. The
chip shows which model answered, and in an
[incognito chat](/docs/features/interfaces/incognito) it notes that only the
local model is allowed.

## See also

- [`mecha model`](/docs/reference/cli#model) in the CLI reference.
- [Serving a local model](/docs/features/models/serving) — slots, and the
  numbers each entry has to agree with.
- [Providers](/docs/features/models/providers) — pins, fallbacks, and why a
  fallback answers under its own model name.
