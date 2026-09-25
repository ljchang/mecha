# Incognito chat — design

**2026-09-25.** One question: *how does a web chat leave no trace once it is
closed — not the transcript, not a title, not a count, not a file, not a log
line — while still being able to read the owner's data and search the web?*

The owner's words: "session information is invisible and there is no trace of
it after it is closed." Everything below is what that sentence costs in this
codebase. The rulings are §2; the audit that found every trace is §3; the
build order is §9.

---

## 0. The short answer

| Question | Answer | Section |
|---|---|---|
| What is kept after close? | Nothing mecha writes. The conversation lives in `mecha serve`'s memory and a tmpfs folder, and both go when it closes | §1, §4 |
| When does it close? | The **End** button, **30 idle minutes**, or `mecha serve` stopping for any reason. A closed tab cannot be seen, so the timeout covers it | §4.2 |
| What can it do? | Read the owner's mail and calendar, search the web (with a notice), generate and edit images, work with files in its own folder. **Never write anywhere else** | §5 |
| Which model? | **Local only.** A cloud provider keeps the text on someone else's servers | §6.1 |
| Can it be replayed or learned from? | No. Nothing to replay, by design — the nightly loops never see it | §2 R1 |
| What is out of mecha's reach? | Swap, the search engine seeing a query, the mail provider seeing a read, a file the owner saves on purpose | §10 |
| How do we know it works? | A canary string through text, an upload and an image; after close, a scan of every store and log finds nothing | §8 |

---

## 1. The promise, stated exactly

**After an incognito chat closes, no file, record, log line or process on this
machine holds anything derived from it** — its text, its tool inputs or
outputs, its uploads, its images, its title, or the fact that it happened.

Three limits, stated because a promise that overreaches is the
silently-degrading guard in another costume:

- **Other parties see what they see.** A web search reaches the search engine;
  a mail read reaches the mail provider's API. That is the private-browsing
  bargain, and the chat says so before the first search (R4).
- **The kernel can page memory to disk.** tmpfs and process memory are both
  swappable. §10 names the fix; software alone cannot promise it.
- **The owner's own acts are theirs.** A picture downloaded to the phone, text
  copied out, a screenshot.

---

## 2. Rulings (owner, 2026-09-25)

| | Ruling |
|---|---|
| **R1** | Strictly invisible: no transcript, **no content-free counts**, no replay, no learning. Invisibility to `reflect`, `distill`, `runlog`, `harness ruminate` and the rest is the point, not a gap |
| **R2** | No "save this conversation" escape hatch. Live-only affordances (retry, branch) are fine; they die with the session |
| **R3** | **Local + read my data:** mail, calendar and graph reads are allowed; writes of any kind outside the chat's own folder are refused |
| **R4** | **Web search allowed, with a notice** shown before the first search |
| **R5** | **Idle timeout: 30 minutes** |
| **R6** | Images are deleted from disk when the session closes — including the image server's copies and the browser's cache |

---

## 3. The audit: every trace a web chat leaves today

Taken against the tree on 2026-09-25. Each row is closed by a mechanism in §4–§6
or named in §10. Symbols, not line numbers.

### 3.1 Written by the web chat path itself

| Trace | Where | Incognito |
|---|---|---|
| Transcript: meta, the full system prompt (`Record::Config`), every message and tool result, rewrites (rolled-back text survives: `Session::messages_ever`), `Outcome`, `Taint`, `GoalAnchor`, `Title` | `sessions/<id>.jsonl`, created on *open* by `serve/chat.rs::ensure_session_as` — before anything is typed; never pruned | **No `Session` at all** (§4.1) |
| Uploads | `work/web/<key>/inbox/` via `files::upload`. The default key `main` is **reused across restarts**, so old uploads accumulate and later chats can read them | tmpfs folder, random key (§4.3) |
| Model-written files (`fs_write`, `shell`) | `work/web/<key>/` | Same tmpfs folder |
| Oversized tool output | `$TMPDIR/mecha-spill-<uuid>/`, **one directory for the whole serve process**, never deleted, readable by any session through the jail — 88 of them on disk on 2026-09-25 | A spill directory inside the session's tmpfs folder (§4.3). Fix the shared one for everyone (§9, step 0) |
| Title | `title::summarise` sends the owner's first turns to the model; stored as `Record::Title` | Titler off |
| Situation brief (#305, merged the same day) | Assembled per run by reading the board through the graph server (`setup::read_board_for_brief` → `kg_task_list`) and other stores, recorded on `RunStats::brief` in the transcript's outcome | Not assembled: there is no outcome to put it on, and the board read is itself a graph call that says a run happened |
| "Earlier" drawer | `chat::history` scans the transcript directory | Nothing to scan; the live list marks incognito and drops it on close |
| Browser cache | `/api/*` sends **no `Cache-Control` at all** today | `no-store` on every incognito response and file (§4.4) |
| Browser storage | None: `web/src` uses no localStorage, sessionStorage, IndexedDB or service worker | Nothing to do — kept that way by a test |

### 3.2 Derived later from session files

`reflect` (writes `learning/reflections.jsonl` — a **git repository**, so a
deleted line survives in history), `distill` (sends the transcript to the model
and **writes an episode into the knowledge graph**), `learn`, `validate`,
`gossip`, `runlog::Corpus`, `harness_probe::draw_episodes`,
`planning::examples` (copies plan text from recent sessions into later runs),
`outbox_source`. **All read transcripts, so none can see a session that never
wrote one.** Note for elsewhere: `reflect` and `distill` ignore
`SessionKind::Test`, which `runlog::Scan::admits` honours — a gap in the test
mark, not in this design.

### 3.3 Written only when a tool runs

| Tool path | Trace | Incognito |
|---|---|---|
| Outbox-routed tools (mail send/reply, calendar create/update/delete, docs append/replace, sheets, factory) | `outbox/<id>.json` with the full body — **staged in every permission mode, read-only included** | Refused (§5) |
| Graph writes (`kg_upsert`, `kg_task_create`, `kg_verdict`, …) | The graph DB | Refused (§5) |
| **Graph reads** (`kg_search`, `kg_entity`, …) | **`query_log` stores the query text**; `retrieval_touch` counts what was returned (mecha-graph `ledger.rs`) | Refused until mecha-graph has an unrecorded read path (§5.2) |
| Docs create, `calendar_hold` | A new Google file or calendar hold | Refused |
| Mail and calendar reads | `mecha-mail` keeps nothing on reads; the provider's API sees the request | Allowed (R3; §1 limit) |
| Web search, `web_open`, `http_fetch` | The query reaches SearXNG (which forwards upstream), Exa or Tavily | Search allowed with notice (R4) |
| Hooks | `pre_tool`/`post_tool` receive tool input and output; `session_end` runs `distill` | Not run |

### 3.4 Other processes

| Process | Trace | Incognito |
|---|---|---|
| llama-server | The conversation's KV cache, **in RAM only** (no `--slot-save-path`); logs timings, not text | Accepted: RAM, gone on reuse or restart (§6.2) |
| A cloud provider | The text, on the provider's servers | Refused: local only (§6.1) |
| journald (`mecha serve`'s stderr) | `provider/openai.rs::log_dropped_reasoning` logs the **last ~400 characters of the model's reasoning at `warn`** when a local turn comes back empty | Content moves to `debug` for everyone (§9, step 0) |
| ComfyUI | Job history (prompt text, file names) in memory; the preview PNG and uploaded references in its temp folder until it restarts | History deleted per job (#306 already); temp files deleted (§6.3) |
| Voice worker | Logs transcripts at `info` (`scripts/voice/worker.py`) | No voice calls in incognito (v1). Dictation is fine: parakeet logs no text |

---

## 4. The mechanism

### 4.1 A conversation with no `Session`

The precedent exists: `mecha chat --no-session` and `mecha tui --no-session`
skip `Session::create` and every record, attach no mailbox ("stays anonymous:
no identity, no mailbox, no return address"), stamp no session id on the
outbox route, and fire no `session_end` hook. Web chat has no equivalent —
`serve::Args` has no such flag and `ensure_session_as` always creates one.

An incognito web session is a `WebSession` whose `session` is **absent by
type**, not a session marked "don't read me". Every path that would record
goes through the `Option`, so the compiler finds each one — `begin_turn`'s
fail-closed "an unrecorded run is invisible to distill…" guard is exactly the
code that must now accept `None` for this kind and only this kind. A mark read
by the readers would be the wrong shape: §3.2 shows two readers already ignore
the one mark that exists.

### 4.2 Lifecycle

- **Open.** A distinct door (`POST /api/incognito`), never a flag on the
  ordinary open — the ordinary path must not be one parameter away from
  incognito, or the reverse. The key is random (128 bits), never `main`, never
  reused.
- **Live.** In `ChatState.sessions` with its kind, so the page can show it.
- **Close**, on the first of: the **End** button; **30 minutes** since the last
  owner turn or tool completion (R5); `mecha serve` exiting. Closing cancels a
  run in flight, drops the `Conversation`, removes the tmpfs folder, and
  deletes the image server's copies (§6.3).
- **Crash.** tmpfs does not survive a reboot; on the next `serve` start, any
  incognito folder left from a crashed process is removed before the door
  opens.

### 4.3 One folder, in RAM

`$XDG_RUNTIME_DIR/mecha-incognito/<key>/` (tmpfs, per-user, 0700), holding the
jail (`inbox/`, `images/`, model-written files) **and the session's spill
directory**. Nothing incognito touches the SSD, so "deleted" means gone rather
than unlinked. `serve` refuses to open incognito if the runtime directory is
not tmpfs — measured with `statfs`, not assumed.

### 4.4 The page

`Cache-Control: no-store` on every incognito API response and file. The page
already stores nothing locally; a web test pins that `web/src` never calls a
storage API, so a later convenience cannot quietly add one.

---

## 5. What an incognito chat can reach

### 5.1 The rule

Incognito narrows the tool surface through the per-session **`withheld`** list
that task chats already use — it reaches subagents too, so delegation cannot
widen it. Withheld:

- every outbox-routed tool (they stage in every mode, so the permission mode
  cannot stop them);
- every MCP tool that is not `readOnlyHint`;
- the graph's tools, reads included, until §5.2 lands;
- document creation and calendar holds;
- `message_send`.

Allowed: mail and calendar reads, `web_search` / `web_open` / `http_fetch`
(R4), `image_generate` (§6.3), and the builtins, with `fs_*` and `shell`
jailed to the tmpfs folder and still subject to the chat's read-only / ask /
allow toggle.

### 5.2 Graph reads need an unrecorded path

R3 allows graph reads, but mecha-graph records every read's query text
(`query_log`) and demand (`retrieval_touch`). Until it can read without
recording, graph tools stay withheld in incognito. The shape to build there: an
unrecorded mode **fixed per server process** (an environment variable at
spawn), not a per-call flag — so an incognito session gets its own graph server
instance, spawned at open and dropped at close, and a forgotten flag cannot
turn one call into a record. This fits the direction that the graph converges
into mecha core.

### 5.3 The search notice

Before the first search in an incognito chat, the page shows one line: the
search engine sees the query, even though mecha keeps nothing. Shown once per
session, in the page — not in the model's context, where it would cost prefix
bytes and change nothing.

---

## 6. Other processes

### 6.1 Local model only

An incognito session is refused on a cloud provider, and the model chip is
locked. A cloud provider's retention is the provider's; "no trace" cannot be
promised about someone else's servers.

### 6.2 llama-server's cache

The KV cache holds the conversation in RAM until the slot is reused or the
server restarts; nothing is written to disk. llama-server can erase a slot, but
the session does not know which slot served it and erasing idle slots costs
other sessions their cache. Accepted as RAM-only; revisit if slot affinity
becomes visible.

### 6.3 Images

History entries are already deleted on every exit (#306). The temp files — the
preview PNG and uploaded references — have no delete endpoint in ComfyUI, so:

- `[image] server_temp_dir` (global-only, like the rest of `[image]`) names
  ComfyUI's temp directory. After a job, the tool deletes the preview and the
  uploads **by the names the server returned**, each checked to be one plain
  path component inside that directory.
- ComfyUI should run with `--temp-directory` on tmpfs, so even the brief copy
  stays in RAM.
- With `server_temp_dir` unset, `image_generate` is withheld from incognito:
  a promise that cannot be kept is not made.

---

## 7. The page

- **New incognito chat**, beside **+**, with its own icon.
- A banner that does not scroll away: *Incognito — nothing from this chat is
  kept. It ends when you tap End, or after 30 minutes idle.*
- **End** in the header; after it, the page shows that the chat is gone and
  offers a new one. There is no "earlier" entry to reopen.
- The model chip is locked to local; the voice-call button is absent;
  dictation stays.
- An edit's **Edit** button works as in any chat — inside the tmpfs folder.

---

## 8. How we know

A test drives an incognito session through the real `serve` routes, with a
unique **canary string** in the prompt, in an uploaded file's name and bytes,
and in an image prompt against a fake image server. After close, it scans:

- the whole test `MECHA_HOME` (sessions, work, learning, outbox, rules);
- `$TMPDIR` and `$XDG_RUNTIME_DIR`;
- the fake image server's recorded requests for a history delete and temp
  deletions;
- captured stderr, at the default log level.

Any hit fails. The negative is checked for vacuity the same way: the same run
as an *ordinary* chat must find the canary in the transcript, or the scan is
looking in the wrong place.

Unit tests beside each mechanism: the withheld set (an outbox route and a
non-read-only MCP tool are absent; a subagent inherits the absence); the
cloud-provider refusal; the tmpfs check; the crash sweep; `no-store` on every
incognito route.

---

## 9. Build order

0. **Fixes worth having in every chat** (independent, first): per-session spill
   directories removed at run end; `log_dropped_reasoning` logs content only at
   `debug`; `Cache-Control: no-store` on `/api/*`; `reflect` and `distill`
   honour `SessionKind::Test`.
1. **The session with no `Session`**: the incognito door, the `Option` through
   `begin_turn`, the titler and the situation brief, the lifecycle and the
   30-minute timeout, the crash sweep.
2. **The tmpfs folder** with its spill directory, and the `statfs` check.
3. **The withheld set**, the local-only refusal, and the search notice.
4. **Images**: `server_temp_dir` and temp-file deletion; ComfyUI on a tmpfs
   temp directory.
5. **The page**: door, banner, End, locked chip, no voice call.
6. **The canary test** — written with step 1, green only when step 5 lands.
7. **mecha-graph's unrecorded read path** (other repository), then graph reads
   come off the withheld list.

---

## 10. What incognito does not promise

- **Swap.** 15 GB of swap is configured on this box. RAM pages, tmpfs
  included, can be written there under memory pressure. The fix is zram or
  encrypted swap; `mecha doctor` should say which this machine has.
- **What leaves on purpose.** A search reaches the engine; a mail read reaches
  the provider; a downloaded picture is the owner's.
- **A `debug` log level.** `MECHA_LOG=debug` logs content by design, for
  diagnosis; incognito's promise is made at the default level, and the page
  says so if `serve` runs at `debug`.
- **Voice calls**, in v1: the voice worker logs transcripts.
