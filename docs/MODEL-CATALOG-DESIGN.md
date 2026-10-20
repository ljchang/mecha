# The chat model catalog — designed 2026-10-19

The model chip (`REMOTE-SURFACE-DESIGN.md` §14, D12) switches between the
models the router already serves. On a fresh install that is one: `mecha
setup chat` writes a presets file with a single section, so the chip says
*"no provider follows a router, so there is nothing to switch"* and there is
no way from mecha to get a second model. The four-model router on the GB10
was built by hand — `scripts/start-router.sh`, a hand-kept `[providers.*]`
entry per model, `follow_loaded = true` typed into the config, and one graft
run from a terminal. A second machine (the owner's Mac, 2026-10-19) got the
same set by copying that box's notes over a cross-session message: eleven
files fetched by hand, a graft built from a script, five preset sections
appended to a file that `mecha setup chat` rewrites, and six provider tables
written in one edit. Every step was reviewable; none was mecha's.

This document is the design for making that a choice on the chip: **a short,
reviewed list of chat models compiled into mecha, each installed only when
the owner approves its download.**

## 1. What the owner sees

- `mecha model catalog` lists the chat models this build knows: for each,
  whether it is installed, what it would download (less what the cache
  already holds), what it holds in memory at this machine's tier, and
  whether it fits. A model that does not fit is shown with the reason, never
  hidden — "offered at the 64, 128 GB tiers" is an answer; a missing row is not.
- `mecha model add <id>` prints that plan and asks. Approved, it fetches,
  assembles, installs the preset, restarts the router under the same switch
  `mecha model use` takes, and writes the provider entry. `--use` then
  switches to it; `--yes` is the approval already given.
- `mecha model remove <id>` takes a catalog model back off the router. The
  files stay in the Hugging Face cache, which is the owner's (`hf` reads it
  too); the line says how much they hold and where.
- The chip lists catalog models it does not serve yet under **Available to
  download**, with the same three figures. Tapping one shows the plan and an
  **Install and switch** button — the approval — and the menu then shows the
  install's progress until the model is loaded.

## 2. Decisions

**C1 — The catalog is compiled in, never fetched.** Every entry is a pin a
reviewer committed: repository, revision, path, sha256 and size per file
(`recommend::Source::HuggingFace`, the shape every other model already
uses). A list fetched at run time would be a second supply chain with no
review, and an entry's preset lines are flags handed to llama-server. A new
model is a pull request.

**C2 — One entry is a whole serving decision.** An entry carries what the
GB10's router and config hold for that model, so nothing is left for the
owner to type: the preset section (its name is the alias, and the provider's
`model`), the files, the per-tier geometry (`ctx-size` and `parallel`, or
the production row's own geometry for a variant of it), the sampling and
draft lines, the provider fields (`context_window` = `ctx / parallel`,
`vision`, `temperature` only where the preset sets one — R4 refuses a
mismatch, so an entry whose preset has no `temp` has no `temperature`
either), and the memory arithmetic with its source.

**C3 — Offered by tier, with the figure said.** Each entry's geometry is per
tier, and a tier with no geometry is not offered. No entry asks more of a
tier than the production model already does there (a test holds it to a
tenth over): at 64 GB the 27B models get a 128k window, since the GB10's
256k would hold 34 GiB against production's 28. Its memory is a `recommend::Peak::Arithmetic` from the pinned
files plus the KV cache at that geometry, never measured on a machine it was
not run on (`recommend.rs`'s evidence rule). The router holds one chat model
at a time (`--models-max 1`), so "fits" means fits *in place of* the chat
model, beside the other slots — not beside another chat model.

**C4 — Installing is the owner's act, from a surface the owner holds.** The
CLI, and the chip through `mecha serve`'s owner door. There is no tool: a
model never chooses the model (§14), and it never chooses what is fetched
onto the machine either. The web route runs `mecha model add --yes` as a
child, as the chip's switch already runs `model use` (D4), so the page can
do nothing the terminal cannot.

**C5 — The catalog survives `mecha setup chat`.** Today `presets_text`
writes one section and the file says `setup chat` rewrites it. The installed
catalog models are recorded under the router's directory
(`~/.mecha/sidecars/router/catalog`, one id per line, written through
`write_owned`), and the presets file is always rendered as the base section
followed by one section per recorded id. `setup chat` changing the base
model keeps the catalog; `features enable --reinstall` keeps it.

**C6 — The router restart is the router install's.** Adding a section means
restarting the router, which stops the resident model — a switch, by D13.
`model add` therefore goes through `router_unit::install` with the installed
choice, after the downloads and after waiting for every run holding the
router to finish (the plan says it will). The install itself still declines
rather than waits (the gate's §10.3) for a run that starts in the instant
between, and the line says to run `add` again, the files kept. It takes
the switch across the restart, waits for `/health`, and checks the base
model loads. The model that was resident before is loaded back after unless
`--use` names the new one. Nothing is fetched while the switch is held:
downloads happen first, so hours of fetching never hold every run.

**C7 — A grafted file is hosted, not assembled on each machine.** One
entry, the uncensored Qwen3.6-35B-A3B, ships upstream without its MTP head;
the GB10 serves it with the production model's head grafted on
(`scripts/mtp-graft.py`, #516). Both inputs are pinned, so the output is
deterministic: built on the owner's Mac on 2026-10-19 it was byte-identical
to the GB10's (`8290c5ba…`). All three sources — Qwen's original, HauhauCS's
uncensored weights, unsloth's MTP conversion — are Apache 2.0, which permits
redistributing a modified file with the license and a statement of what
changed. So the graft is hosted at
`mecha-factory/Qwen3.6-35B-A3B-Uncensored-MTP-GGUF` (the owner's decision,
2026-10-19), its card naming both inputs by revision and sha256, the script,
and the credits, with Qwen's LICENSE beside it; the entry pins that file like
any other. The alternative — a hash-locked Python environment on every
machine that installs it, a second 22 GB download for the donor, and a
minute's assembly checked against the same sha256 — bought nothing the pin
does not already guarantee. There is no "serve it without the head"
fallback: the preset's draft lines would fail the child's start.

**C8 — The default entry follows the router once there is a choice.**
`follow_loaded = true` is what lets a default run take whichever model is
resident. `model add` writes it on the owner's table that names this router
(shown, and in the same confirmation) when there was none, because the
second model is the moment the flag starts to mean something. A table
naming the router that the owner has pinned some other way is left alone
and the line says what a switch will not reach.

**C9 — Provider entries come from the pin, not from `/props`.** `setup chat`
reads its provider back from the router because the model it installs is
loaded at that moment. A catalog model is not: asking `/props?model=<alias>`
for an unloaded model loads it (CLAUDE.md, the local model server), which is
a switch nobody asked for. The entry's own fields are written, and the
mismatch checks that already run at a switch (R4's sampling, the preflight
on the first run) read them against the server.

## 3. Not in scope

- A catalog fetched from a server, or entries the owner adds to it. A model
  outside the catalog is still `mecha setup chat`'s *a GGUF you already
  have*, or a hand-written preset on a router mecha did not install.
- Two chat models resident at once. `--models-max 1` stays; sizing a pair
  is by hand (§14).
- Deleting the downloaded files. They are in the shared Hugging Face cache;
  `remove` says how much they hold.
- Engine compatibility checks per entry. Every catalog file is a plain GGUF
  the pinned llama.cpp loads; an entry's draft lines were checked on Metal
  and on the GB10's CUDA before it was added (§4).
- Hand-built routers. A router mecha did not install is the owner's: `model
  add` prints the section and the provider table to paste, and changes
  nothing.

## 4. The first five entries

All from the GB10's router (`mecha-b5`, 2026-10-19), and each loaded,
answered and drafted on the owner's M-series Mac the same day:

| id | files | tiers | ctx × slots | draft acceptance on Metal |
|---|---|---|---|---|
| `qwen3.6-35b-a3b-uncensored` | HauhauCS Q4_K_M with the MTP head grafted on (`mecha-factory`) + HauhauCS f16 projector | production's | production's | 0.86 |
| `qwen3.8-27b` | unsloth UD-Q4_K_XL + BF16 projector (two revisions) | 64, 128 | 131072 × 1 at 64, 262144 × 1 at 128 | 0.51 |
| `qwen3.8-27b-uncensored` | HauhauCS Q4_K_P (head in file) + BF16 projector | 64, 128 | as `qwen3.8-27b` | 0.51 |
| `qwen3.8-27b-abliterated` | huihui-ai UD-Q4_K_XL + BF16 projector | 64, 128 | as `qwen3.8-27b` | 0.64 |
| `gemma-4-26b-a4b` | unsloth UD-Q4_K_M + BF16 projector + MTP draft file, one revision | 32, 64, 128 | 32768 × 1 | 0.73 |

Three of the five are uncensored or abliterated builds. They are in the
catalog because the owner runs them; the catalog says so in each entry's
note, and nothing installs one without its name on the confirmation.
