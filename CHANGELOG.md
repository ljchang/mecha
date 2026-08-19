# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.7] - unreleased

Two releases in one day, and they rhyme. mecha got a writable seat at the
user's documents — with the grant chosen so a document nobody handed over is
not reachable by any prompt — and it learned to file mail, with the reading of
that mail quarantined so a subject line cannot reach a run holding the
calendar. Both landed a verb in the same new capability quadrant, from
opposite directions.

### Upgrading

**Both mail accounts must re-authenticate.** The Google and Microsoft scopes
widened so mail can be filed rather than only read, and a widened scope does
not cover a grant issued under the old one. Run `mecha-mail auth <account>`
for each; `mecha doctor` names any account that cannot triage, with the
remedy. Nothing else in this release requires action.

### Added

- **A run now records how it went, not only what it cost.**
  `Record::Outcome(RunStats)` lands one row per finished run in the session
  transcript, written by every front-end. `RunOutcome` carries fifteen fields
  and the transcript kept two, so an interactive run was measurably less
  observable than an unattended one — the signal was computed and discarded at
  the end of every run a human was watching. Every field is a deterministic
  count and none is derived from the *content* of a tool result, which is what
  makes the corpus usable as an input to automated grading: a counter carries
  no instructions.

- **`mecha sessions health`** reads that corpus back across the whole session
  store — stop causes, tool reliability, how often a run finished over a
  failure — split by model, because a corpus spanning two has no single rate
  worth quoting. A rate with no denominator prints as an em dash rather than
  as zero: "nothing went wrong" and "nothing happened" are different answers.

- **`mecha diagnose`** reads the corpus, folds in `doctor`'s findings, and
  proposes one typed change with a falsifiable prediction. It does not measure
  and does not apply — it prints the `eval --ab-config` line that would falsify
  it. Automated failure attribution is right about which step failed roughly
  one time in seven, so the design target is that being wrong costs one
  measurement. The brief is built from counters and has no field for a
  transcript excerpt, and a proposal reproducing eight consecutive words from
  anything the diagnostician read is refused.

- **`mecha eval --ab-config KEY=VALUE`** runs the case set twice, differing
  only in the override, and judges the difference: paired by case,
  deterministic holdout, and a work guardrail that rejects a gain bought by
  attempting less. Overrides are a closed set of run options, so both arms are
  built by one code path.

- **`mecha doctor` reads the population, not just the incident** — a model
  finishing a fifth of its runs over a failed call, failing a quarter of its
  tool calls, or having a quarter of its runs cut short by a ceiling.
  Cancellations are excluded: a person pressing Ctrl-C is the system working.
  It also names a trigger failing a large share of its calls, and a trigger
  whose most recent run succeeded having done nothing at all.

- **Mail triage.** `mecha-mail` gained `mail_triage` — archive, read, unread,
  spam, trash — as a closed action enum over a whole thread. The write surface
  was previously send, reply and calendar CRUD, so a mailbox could be read and
  answered but never emptied. Gmail does all five as label arithmetic in one
  call; Graph has no thread resource at all, so each verb resolves the
  conversation to message ids and acts on each, reports how many it touched,
  and fails only if every message failed.

- **`mecha mail`** — `classify`, `list`, `show`, `dismiss` — over a new store
  at `~/.mecha/mail-triage/`, one typed verdict per thread. `classify` reads
  recent mail through the MCP surface and hands each thread to a classifier
  with no tools, no history and no shared cache prefix, which returns a
  bucket (`respond`/`notify`/`ignore`), an urgency, a proposal, tags from a
  closed vocabulary, a deadline, and the request kind if it recognises one.
  Verified on fifty real threads: twenty-eight archivable, twenty-two needing
  attention, four request kinds recognised.

- **A snippet-first pass with escalation.** A preview settles the newsletters;
  the full body is read only when the verdict is `respond` or names a request
  kind — the cases where the answer changes what happens. Measured at 25% of
  threads escalating. Deliberately not triggered by snippet length: a provider
  caps its preview, so nearly every real email looks truncated and escalating
  on that would escalate everything.

- **A nightly sweep**, `scripts/mecha-mail-classify.{service,timer}`, 05:30
  UTC and Dartmouth-only. A timer rather than a `mecha trigger`, because a
  trigger's action is a prompt on purpose and this is a deterministic command.

- **Google Docs, Sheets and Slides**, as a fourth binary on `mecha-mail`.
  `mecha-docs auth` consents once; `mecha-docs pick` opens Google's real file
  chooser to adopt a document that already existed; `mecha-docs list` reads
  the reachable set back from Drive rather than from a local index, since a
  listing under this scope returns exactly the in-scope files and a second
  copy could only disagree. `mecha-docs serve` is the MCP face.

- **Eleven tools**: `docs_list`, `docs_read`, `sheets_read`, `slides_read`,
  `docs_create`, `docs_append`, `docs_replace`, `sheets_create`,
  `sheets_write`, `slides_create`, `docs_trash`. `docs_replace` is the
  surgical edit — quote the text to change rather than index into it — and
  reports zero matches as a failure, because a model told "ok" there goes on
  to describe an edit that never happened.

- **`--paste`, for a machine with no reachable browser.** It prints the
  authorization URL and takes back the `127.0.0.1` address the browser lands
  on, which is displayed in full even though nothing is listening. No tunnel
  and no forwarded port. There is deliberately no device-code flow: Google's
  device flow refuses the client type the file chooser requires, and two
  clients cannot substitute, because a per-file grant belongs to a
  *(user, client)* pair and the two would hold disjoint sets of files.

- **`mecha-mail corpus`** — download a span of mail for analysis, envelope and
  snippet only, one JSON object per message. **An operator command,
  deliberately not an MCP tool**: the model has no business bulk-reading a year
  of mail, and a corpus verb on the tool surface would be one prompt away from
  being asked to. Deliberately unclassified, too — running a corpus through the
  classifier projects the existing tags onto it and confirms them by
  construction, so the taxonomy has to come from the mail rather than from the
  labels. Graph needed a new path (`search` caps at 100 with no cursor, and
  `$search` forbids `$orderby`, so it walks `@odata.nextLink` with `$orderby`
  alone, as the calendar has all along).

### Security

- **The mail classifier sees the prose; nothing else does.** Reading mail arms
  `untrusted_input`, so a loop that reads fifty threads into one conversation
  arms the trifecta for all fifty and stages fifty tainted drafts — correct,
  and useless, because a warning that fires on everything has stopped being a
  warning. The front door's shape applies one directory over:
  `Record::for_privileged_run` returns the typed verdict and the sender's
  address, and there is deliberately no argument that returns the subject, the
  sender's chosen display name, the classifier's own reasoning, or its
  one-line summary. That last one is the judgement call — short, and exactly
  what a summary wants — but it is model prose derived from attacker prose,
  which is the laundering path the front door withholds `reading` to close.

- **`mail_triage` carries `destructiveHint` alone.** It mutates the user's own
  mailbox and reaches nobody, so it is not `external_send` and must never
  appear in `[outbox] tools` — staging it would make triage review a queue in
  order to fill another queue. It is not `readOnlyHint` either, or a
  `permission_mode = "read-only"` trigger could empty an inbox unattended.
  `assert_tool_surface` takes a third slice asserting both negatives. This is
  the quadrant `docs_trash` also lands in.

- **Tagging is not a provider operation.** Gmail labels and Graph categories
  are different objects and a tag meaning different things per account fails
  at the one job tags have. A mecha tag lives on the triage record, costs no
  OAuth scope, and works identically on both providers; a test asserts the
  mail surface offers no `label`/`tag`/`categor*` verb.

- **Recognising a request kind is not routing it.** `REQUEST_TYPES` is what
  the classifier can name; `ROUTABLE_TYPES` is the subset a manifest exists
  for, and only that subset can be promoted to the front door. A recognised
  kind with no manifest keeps its name as evidence and loses only the
  promotion there would be nothing behind. Enforced in code, not asked of the
  model.

- **The scope is the boundary.** `drive.file` is the one non-sensitive scope
  in the family: no verification, no annual security assessment, and — on a
  published project — no seven-day token expiry. It also cannot reach a
  document that was not created by mecha or explicitly handed to it, which is
  a stronger guarantee than any of the wider scopes could be made to give.

- **Writing a document counts as sending.** Writing into a document a third
  party can read is exfiltration; it looks like a local edit and it is a
  publish. Every write carries `openWorldHint` and belongs in
  `[outbox] tools`, so it stages for review rather than executing. Reads are
  `readOnlyHint` and not `openWorldHint`, with `untrusted_input` forced by
  config the way mail and the graph already are — a document comment is an
  injection vector invisible in the rendered page.

- **`docs_trash` is in neither quadrant**, carrying `destructiveHint` alone:
  it reaches no third party, so staging it would make review circular, and it
  is not read-only, so an unattended read-only run must not reach it. The same
  quadrant `mail_triage` occupies. There is no permanent-delete verb and no
  sharing or permissions verb; the scope would permit both, so the boundary is
  the tool surface and a test asserts the absence.

- **Mail OAuth scopes widened, and a refresh stopped over-asking.**
  `gmail.modify` replaced `gmail.readonly` (stopping short of
  `https://mail.google.com/`, whose only addition is irreversible deletion)
  and `Mail.ReadWrite` replaced `Mail.Read`. Both invalidate existing grants.
  Separately: Entra's refresh sent the whole scope list on every renewal, so
  widening it asked for a superset of what the stored grant consented to —
  refused as `invalid_grant`, indistinguishable from a revocation, and
  classified as permanent. Every working Outlook account would have gone dark
  about an hour after install. A refresh now sends no scope at all, per
  RFC 6749 §6, and a test asserts the absence.

- **A grant now says what it can do, and when it dies.** `StoredCredentials`
  records `granted_scopes` verbatim and `granted_at` (stamped at consent,
  never touched by a refresh). `mecha doctor` reports an account whose grant
  cannot support the triage verbs, and warns two days before a declared
  `grant_lifetime_days` expires — the Google Testing-mode seven-day clock that
  a marker written on failure could only ever report after the outage began.
  Absent reads as *not covered* and as *unknown*, never as safe.

- **The grant lives under its own root** (`~/.mecha/docs/<account>/`), sharing
  the credential type with mail but not its namespace: `mecha doctor` reads
  every `oauth.json` under `~/.mecha/mail/` as a mail grant, so a documents
  credential there would be reported as a broken mail account — a finding
  naming the wrong subsystem.

### Changed

- **The learned-rules budget is per domain, and a run carries only the domains
  it names.** `MAX_ACTIVE_RULES_PER_DOMAIN` 15 → 25 with `RULES_CHAR_BUDGET`
  1600 → 2600 beside it, and the learner frames are handed the same constant
  rather than repeating it as prose — a frame saying "never exceed 15" while
  the gate admits 25 fails silently, because an over-consolidating learner
  looks like a well-behaved one. `rules_prompt_block_for(RUN_DOMAINS)` gives a
  run `behavior` + `writing` and nothing else; new domains are opt-in and
  `unrouted_domains` warns at startup about any holding rules no run carries.
  Everything reconstructing "what a run sees" moved with it, or the validation
  ledger would be keyed to a rule set no run ever had.

- **The mail taxonomy is measured rather than guessed.** A year of real mail
  said the request kinds were wrong in both directions. `student-advising`
  joins `REQUEST_TYPES` and `advising` joins `TAGS` — the largest category by a
  wide margin, and absent because it is the most routine thing that arrives. `book` leaves: two
  threads in ten months, neither a request to write a book. A name on that list
  is a claim that the kind arrives.

- **Mail no longer routes to the front door.** `Proposed::Frontdoor`,
  `ROUTABLE_TYPES` and `is_routable` are removed. The front door's
  `[verification]` block exists to prove a stranger controls an email address,
  and an email arrived from one — so the machinery being imported solved a
  problem mail does not have, while removing threads from the queue whose whole
  value is being the one place. Mail keeps its own request kinds; the surviving
  half of the idea becomes mail's own `needs-info`. Recognising a kind was
  already separate from routing it, which is why nothing else moved.

- **`mecha mail correct`** — say the classifier got a thread wrong, field by
  field. A misread bucket, a missed deadline and a wrong request kind are
  different errors with different fixes, so the correction names which. The
  verdict is fixed in place immediately and the before/after pair is kept on
  the record, because a learner shown only the right answer cannot see what to
  stop doing. A correction that agrees with the classifier records nothing.

- **`mecha mail score`** — score the live triage store against what actually
  happened. Behaviour (did a reply go out) and testimony (what you corrected)
  are reported apart, because a reply is one-sided evidence and a correction is
  not. Threads younger than 48 hours are excluded: most replies that ever
  happen land on the first day, so a same-day thread has no outcome yet and
  counting it would punish a rule for how recently the mail arrived.

- **`mecha mail eval`** — grade the classifier against a corpus whose outcome
  is known, with no human grading anything. Reports a false-`ignore` rate on
  the answered stratum and a volume on the unanswered one, never a blended
  accuracy; `--out` keeps every graded verdict so a run can be re-read without
  being re-run.

### Fixed

- **A pile of identical failures no longer rides in every request.**
  `compact::collapse_repeated_failures` folds repeated failures onto their
  newest member. Errors are exempt from supersession on purpose — a failed
  call says nothing about the target — and that rule is right for one failure
  and inverts for eight: a model is measurably likelier to fail a step when
  the context holds its own earlier errors, and the effect does not go away
  with model size. Nothing in the harness touched these before: eviction skips
  errors by construction and thinning only truncates long results, so a
  sixty-character failure message was untouchable by both.

- **A run that answers over a failed call now says so.**
  `RunOutcome::ended_on_failed_call`, with an `expect` check beside it. Nothing
  in the answer text or the stop reason distinguishes a run that recovered
  from one reporting success over a failure, and grading the claim needs a
  judge, which measures near chance at exactly this.

- **A retired proposal no longer makes a triage record unreadable.**
  `Proposed`'s derived `Deserialize` failed the whole record on an unknown
  string, so removing a variant from a type written to an append-only store
  would have silently truncated it — five records in a live store carried
  `proposed: frontdoor` on the day it was removed. The hand-rolled impl
  degrades an unknown proposal to `none`, which is what "a human decides"
  already meant.

- **`mecha-mail corpus` no longer silently truncates Gmail.** Each month was
  capped at 500 messages and a failed month was `unwrap_or_default()`ed into an
  empty one, so a partial corpus and a quiet month were the same value. On a
  real year, most months hit the cap and the loss grew toward the present as
  volume grew — which does not add noise to an analysis, it adds a slope, and a
  slope is indistinguishable from a finding. The ceiling is now
  high enough to be theoretical, reported on stderr when reached, and errors
  propagate.


- **A failed classification is retried instead of buried forever.** The sweep
  skipped anything the store had heard of, and the store holds failures as well
  as verdicts — so a thread whose classification failed was skipped by every
  later sweep on the strength of a record saying it never happened. An outage
  on 2026-08-19 left 17 threads in that state, one of them a manuscript review
  invitation.

- **A classify run that did nothing now fails.** It returned success however
  badly it went, so a run that classified 0 of 16 logged SUCCESS and every
  exit-code check — `OnFailure=`, `systemctl --failed`, doctor's failed-unit
  scan — read a dead nightly as healthy. Partial failure stays a success
  deliberately.

- **A retired learned rule survives a reworded re-derivation.** Retirement was
  carried by exact text, so the same rule in different spelling or punctuation
  came back live. It now matches after normalising case, punctuation, spacing
  and `-ise`/`-ize`, checked only against retired rules so two distinct rules
  cannot be merged by accident.

## [0.1.6] - 2026-08-16

The release where the knowledge graph became a sibling product: pkg went
public as [mecha-graph](https://github.com/ljchang/mecha-graph) (three crates
on crates.io), and mecha's side of the seam was renamed, un-prefixed, and
folded into the install ritual.

### Added

- **`prefix_tools` on `[[mcp]]`.** Unset keeps the collision-proof
  `<name>__<tool>` default; `false` registers a server's tools under their
  raw names, for a server whose tools already carry their own namespace —
  the graph's `kg_*` family, where `graph__kg_search` was stutter the model
  typed in every call. Unprefixed is a promise of distinct names, and the
  promise is enforced: a collision with anything already registered fails
  startup loudly (a prefixed name always contains the `__` marker, so the
  two cases cannot be confused, and prefixed shadowing keeps its existing
  semantics).

### Changed

- **The eval grades the surface that actually runs.** `pkg-cases.jsonl` is
  now `graph-cases.jsonl`, its fixture `graph_server.py`, and the trace
  checks expect the bare `kg_*` names production serves — an eval still
  measuring `pkg__kg_*` would grade a configuration that no longer exists
  anywhere. Scorecards across the rename are not comparable, which is the
  honest cost of any tool-surface change. The `distill`, `vet`, and
  `corroborate` server defaults follow the live alias to `graph`.

- **The graph installs with mecha.** The README offers `mecha-graph-mcp`
  and `mecha-graph` beside `mecha-mail`, the sample `[[mcp]]` block wires
  the installed binary (`command = "mecha-graph-mcp"`, `prefix_tools =
  false`), and the update skill folds the graph into the same
  cargo-install ritual as every other binary.

### Fixed

- **`--mcp-file` fixture paths survive the workspace.** Relative `args`
  were joined against the file's directory but left relative, and since
  MCP servers began starting in the run's workspace that join resolved
  there instead — every fixture handshake failed regardless of names. The
  base directory is canonicalized, so the joined path means the same file
  from any working directory.

## [0.1.5] - 2026-08-16

### Added

- **`recall` — the record is searchable after the summary.** A tool over the
  session's recorded transcript: the union of every message ever recorded,
  including what a compaction rewrite replaced, so a run missing a dropped
  detail looks it up instead of re-running tools or re-living the stretch.
  Taint-neutral by construction (everything it returns entered this
  conversation once, and its arrival is what armed the interlock; the
  transcript path is fixed at registration, never model input). Registered
  by chat, the TUI, and resumed runs — deliberately not Slack (one shared
  registry across per-thread conversations would cross-wire transcripts) and
  not fresh one-shots or triggers (a per-run record is empty until run end).

- **The cache lens: prompt-cache reuse is watched, not assumed.** A pure
  per-run observer fed each request as actually sent plus the usage the
  provider reported. It names the reason when reuse legitimately breaks
  (tool/system surface changed, transcript rewritten by compaction), stays
  silent on providers that have never reported a cache figure (zeros are
  silence, not a miss), and warns on the one remaining shape — a large
  re-payment with nothing changed. Verdicts go to tracing only; the model
  and the loop never see them.

- **`kind = "landlock"`: file confinement that needs no privilege.** A
  fourth sandbox backend for machines where `bwrap` fails even installed
  (Ubuntu 23.10+'s AppArmor userns switch): Landlock LSM rules applied in
  the child between fork and exec — ruleset built in the parent, the
  post-fork closure makes raw syscalls only, and a kernel reporting
  `NotEnforced` fails the spawn rather than running unconfined. Priced
  honestly: Landlock cannot close the network (TCP denied on 6.7+ kernels,
  UDP unrestrictable at any ABI), so a landlocked `shell` **never earns the
  interlock relaxation** — what it buys is the file story, hard-required at
  ABI 3 because older kernels cannot restrict truncate. Preflight proves
  the *denial*, not just the apply: it plants a file in the real home and
  requires the confined read to fail.

- **Gossip adjudicates the queue instead of adding to it.** After building
  deep context on an entity, a gossip probe now pulls the pending claims
  about that same entity and judges them under vet's existing
  `verification` mechanism — the one output that makes the review backlog
  smaller. Verdicts are opinions filed beside still-pending candidates;
  `--adjudicate` caps the batch (default 25) so the owner node's ~1,800
  claims cannot absorb a whole night.

### Fixed

- **A run that compacts itself no longer loses its own head.** Front-ends
  record at run end, so turns of the current run that a mid-run rewrite
  replaced were never anyone's to write. The states a rewrite replaces now
  ride on the `Conversation` (`rewritten`, cleared at run start), and
  `Session::record_run` — which now takes the conversation, so a caller
  cannot record the destination while skipping the journey — walks
  snapshot → snapshot → final. `load()` still replays to the live state;
  `recall` reads back what the summary dropped; the taint timeline only
  gains rewrite records, which drop checkpoints in the over-tainting
  direction.

- **A reader's searches no longer feed the probe selector.** Gossip's
  `LensedSearch` passes `probe: true` to `kg_search`, so pkg's Selector —
  which ranks probe targets by retrieval demand — stops counting a probe's
  own reads as the owner reaching for something. One probe had taken its
  target from 2 touches to 28 and re-elected the same person out of a pool
  of nine.

### Changed

- **`trusted_output` must now name what it vouches for.** A subagent profile
  setting `trusted_output = true` without an `answer_shape` refuses to
  construct — the old semantics let one config line disarm the trifecta's
  untrusted leg for every answer the child ever gave, with nothing checking
  anything. Trust is now granted per answer: declare `answer_shape =
  "number"`, `"boolean"`, or a list of allowed strings, and an answer earns
  the vouch by parsing as that shape at return time. Prose never parses, a
  mismatch comes back marked untrusted with a note saying why, and the
  subagent's static capability now stays honest (`untrusted_input` derives
  from the child's tools unconditionally — the loop's `untrusted_input &&
  external` rule is what lets a shape-proven answer through clean). There is
  deliberately no bounded-string shape. `docs/TRIFECTA.md` (new) maps all
  four trifecta channels and which mechanism owns each.

- **A trifecta refusal now names the durable fix, not just the workaround.**
  Tools gained `denial_remedy()` — the tool's own account of what would make a
  call like the refused one safe, relayed at the end of the interlock's
  message. `shell` uses it to point at the actual remedy (`[sandbox]` with
  `network = false`, or just the network flag when the sandbox is already on),
  which ends the measured dead end where denials advised delegating to
  subagents that had no shell. The delegate suggestion itself now says it is
  for *reading* the outside world, so the model stops treating it as a route
  for local work. Security is unchanged: the same calls are refused, the
  refusal just routes somewhere real.

## [0.1.4] - 2026-08-15

Slack stops being a place to read about work and becomes a place to do it,
and five stores that each knew something was wrong get something that reads
across them.

### Added

- **`mecha doctor` — one pass over every store, no network, no model, no
  tokens.** The 2026-08-11 incident in one sentence: five stores each recorded
  a real failure correctly and the operator learned nothing, because nothing
  read across them. Doctor is that read — dead auth markers, releases that
  errored, drafts and requests waiting past a threshold, triggers whose slots
  stopped advancing, failed `mecha-*` units.
- **`/doctor` in the TUI, one keystroke from where the error surfaced.**
  Findings from `mecha doctor --json` as a child process, broken before
  attention, a component's findings kept together. Acting on a finding
  dispatches on the remedy's shape: one whose surface is already a modal
  deep-links to it rather than spawning a nested CLI over the modal that *is*
  the review, and a remedy needing a terminal suspends the TUI and inherits
  the real one.
- **Executable actions from Slack.** Buttons and modals that do the thing
  rather than describe it — the button carries an id, the enum carries the
  verb, so the prose that shares a context window with third-party text never
  becomes the instruction. Trigger enable/disable, frontdoor close and
  needs-info by modal, and a session-scoped `review now|later|auto` taken only
  as the owner's exact command word, expiring with the thread state, with
  tainted drafts carded in every mode.
- **Tier-2 gossip and the vet mechanism** (`mecha gossip --out`,
  `mecha vet`) — corroboration and queue-verification over pkg's review queue,
  origin-family exclusion computed rather than reconstructed, batch runs that
  keep their evidence as JSONL with per-candidate error containment.
- **Recorded denials and structured corrections**, mecha's half of pkg's D3:
  the graph can say "no", and the distiller can say "that was wrong".
- **A start script for Qwen3.8-27B** (`scripts/start-qwen38.sh`) as a fourth
  local arm on port 8083 — dense rather than MoE, with the MTP draft model
  loaded separately because unsloth ships no MTP variant for 3.8. It carries
  the measurements and the two traps this model brings: llama.cpp#20837 is
  still open against the `qwen35` architecture it reuses, and its chat
  template rejects any system message that is not first.

### Changed

- **One release policy instead of two.** `review_policy.rs` holds the mode and
  `auto_releases(mode, tainted, finished_clean)`, so the tainted exclusion and
  the early-stop exclusion are the function's signature rather than a check
  each surface has to remember. Slack auto no longer releases what a stopped
  run produced.
- **Nothing blocks the dispatch loop.** A button press reads its one item file
  — id shape-checked before it touches a path — instead of parsing the whole
  outbox, twice, for tainted. The ledger append rides inside the spawned task,
  and the TUI's restart guard probes on a watched thread so a probe that never
  answers cannot hang the interface.
- **`cargo fmt --all`**, which CI had been failing on for four commits. No
  semantic change; the drift spanned 20 files and predated the commits it was
  failing on.

### Fixed

- **Nothing fails silently: the staged jail, the unanswered card, the revoked
  token.** Three field failures with one shape — a component that knew
  something was wrong and told no one. A staged call now records the jail its
  tool really runs under, via `Tool::fixed_workspace`.

## [0.1.3] - 2026-08-10

Dependency maintenance, and the one migration hiding among the bumps.

### Changed

- **The minimum supported Rust version is 1.89**, raised from 1.88 to take
  `rustyline` 18, which uses `file_lock`. `CONTRIBUTING.md` now carries the
  rule for when it may move again — with a dependency that needs it and never
  to use a new feature ourselves, raised in the same change as the dependency
  that forced it and naming it. The old number had no rationale attached,
  which is why every bump reopened the argument.
- **Nine dependency majors** — `rand` 0.8 → 0.10, `sha2` 0.10 → 0.11,
  `toml` 0.8 → 1.1, `htmd` 0.1 → 0.5, `base64` 0.22 → 0.23,
  `rustyline` 15 → 18 (the line editor behind `mecha chat`), `clap` 4.6.6,
  and `ratatui` 0.29 → 0.30 with `crossterm` 0.28 → 0.29, which cannot move
  separately because ratatui pins it. The ratatui major was a migration rather than a
  bump: 0.30 makes `Backend::Error` an associated type, so `terminal.draw(..)?`
  inside a function returning `anyhow::Result` stops compiling — anyhow needs
  the error to be `Send + Sync + 'static` and an opaque associated type
  promises none of it. Fixed with a bound,
  `Terminal<impl Backend<Error: Send + Sync + 'static>>`, rather than pinning
  the concrete `io::Error`: ratatui's own `TestBackend` uses `Infallible`, and
  the concrete type would have locked every rendering test out of the
  functions it tests.
- **Google's PKCE verifier is filled in one call** from `rand::rng()`, the
  thread-local CSPRNG, rather than a byte at a time — the one code change the
  `rand` major required. The function had no test at all, which is how a
  security-relevant generator gets its random source swapped with nothing
  watching; it has two now, the load-bearing one recomputing the challenge from
  the verifier exactly as the authorisation server does, because a wrong
  transform fails every sign-in with an error naming neither end.
- **Four GitHub Action majors** — `checkout` 4 → 7, `setup-node` 4 → 7,
  `upload-pages-artifact` 3 → 5, `deploy-pages` 4 → 5.

### Added

- **A test that an HTML-only email keeps its links, tables and lists.** `htmd`
  is a behavioural dependency rather than an API one — it decides what a mail
  body looks like to the model, and four minor versions can change that while
  every type still lines up. Asserted on content surviving rather than exact
  output: a golden-output test would break on every bump of a converter whose
  formatting is its own business, where "the link's URL is still in there" is
  what must never stop being true.

## [0.1.2] - 2026-08-10

### Fixed

#### Reasoning, and the empty turns it turned out to be causing

A reasoning model's thinking arrives on its own channel, and this harness read
none of it. `reasoning_content` appeared nowhere in the tree, so on any local
model the reasoning toggle did nothing, the TUI showed silence where thinking
should stream, and the transcript kept no trace. `Block::Thinking` and
`StreamEvent::ThinkingDelta` had existed since the Anthropic backend was
written; the plumbing was all there and nothing was ever fed into it.

It was also, measurably, the cause of the empty turns this project has been
mitigating since 2026-08-07. Replaying the exact prefixes that went quiet
during the 2026-08-10 benchmark reproduced one, and its "reasoning" was a
complete, unparsed tool call — in one case 120 characters that were *only* a
tool call, with no deliberation at all. `finish_reason: "stop"`, no content,
no `tool_calls`. The model had emitted its call before closing `</think>`, so
llama.cpp filed the whole turn as reasoning (upstream ggml-org/llama.cpp
[#20837], [#22684], [#20809] — all unfixed, and the same failure is reported
against ollama). Note what that rules out: 120 characters is nowhere near a
limit, so `--reasoning-budget`, `max_tokens` and the context window were never
relevant. Every earlier mitigation aimed at "the model reasons too long" was
aimed at the wrong failure.

And half of it was ours. Because the history sent back stripped every
`<think>` block, the model was shown turn after turn of itself apparently
calling tools without thinking, and it obliged. Same server, same template,
same prompt, varying only whether the history carried reasoning: **6 of 6
empty turns without it, 0 of 6 with it** (Fisher exact p ≈ 0.001, on a
reproducer that fails byte-identically).

- `reasoning_content` decodes into a `Block::Thinking` and streams as
  `ThinkingDelta`, so reasoning is visible live and recorded in the
  transcript. It is never *output*: `produced_output` is the same definition
  the loop decides `produced_nothing` on, so a reasoning-only turn is still
  nudged rather than ending a run with an empty answer.
- It rides back to the provider on the next request. Self-gating by
  construction — a `Block::Thinking` exists on this path only because a server
  sent the field, so it returns only to servers that speak it and never to an
  endpoint that would reject an unknown one. No provider name is tested.
  Affordable because the prefix cache absorbs it: measured, turns with
  5,000-token prompts prefill 16–211 tokens, better than 95% reuse.
- An empty turn now records what actually arrived — its full trace at debug,
  with a tail and a family-agnostic lost-call marker at warn. Such a turn is
  in no transcript (the loop nudges and continues before pushing, and holds no
  session to record into), so this is its only durable record.
- A run that ends having only reasoned hands that reasoning back, labelled as
  deliberation rather than a committed answer, instead of reporting that the
  model said nothing.
- `usage.prompt_tokens_details.cached_tokens` is read, so cache reuse is no
  longer invisible. The cached half is *subtracted* from `input_tokens`:
  Anthropic reports the tiers beside the prompt count and OpenAI reports them
  inside it, and `total_input` sums all three — carrying it over unchanged
  would report a prompt at nearly twice its size, and the compaction threshold
  reads exactly that number.

[#20837]: https://github.com/ggml-org/llama.cpp/issues/20837
[#22684]: https://github.com/ggml-org/llama.cpp/issues/22684
[#20809]: https://github.com/ggml-org/llama.cpp/issues/20809

#### The benchmark adapter

- **A task whose instruction opens with `-` reaches mecha as a prompt, not a
  flag.** `terminal-bench/pytorch-model-recovery` describes itself as a
  bulleted list, so clap read `- ` as an argument and exited 2 before the run
  started; Harbor records that as `NonZeroAgentExitCodeError` and scores 0.0,
  indistinguishable from a model that tried and failed. `shlex.quote` never
  helped — it makes the text one argv entry, and the problem is that entry's
  first character. Fixed with `--`, rather than `allow_hyphen_values` on the
  CLI, which would let a mistyped flag be swallowed as the prompt and run
  anyway.

## [0.1.1] - 2026-08-09

### Added

#### Slack as a remote control

- **`mecha slack`** — `auth` (tokens read from the environment rather than from
  flags, so neither reaches shell history or `ps`), `link` (a one-time code
  binds this install to a workspace), `connect` (holds the socket open and
  drives runs from threads; this is what the systemd unit runs), `threads`,
  `sweep` and `status`. The transport is a fourth crate, `mecha-slack`, which
  joins the crates published to crates.io.
- **A Slack thread is a `Conversation`**, which hands the trifecta interlock the
  right granularity for free: a new thread is an honest clean slate, and a
  thread that fetched a hostile page on Monday still remembers on Tuesday.
  Everything per-thread — jail, budget, cancel token, steering queue, approver —
  rides on `RunContext`, because one agent serves every thread.
- **Unfurling is off on everything the model authors, and there is no parameter
  to turn it on.** A model-emitted URL that unfurls becomes an outbound GET that
  no tool call made and no interlock saw, so it is a property of the transport
  rather than an argument some call site can forget.
- **A private file download that comes back as a sign-in page is refused.**
  `files.slack.com` redirects to the workspace host, HTTP clients strip
  `Authorization` across hosts, and Slack answers with an HTML login page at
  status 200 — so the header is sent explicitly, no redirect is followed,
  `text/html` is rejected even at 200, and the byte count is cross-checked
  against the size Slack reported. Without them a sign-in page reaches the model
  labelled as the user's screenshot.
- **`[slack]` config** — `max_concurrent` (threads with a run in flight at once;
  at the cap the connector refuses and says so rather than queueing),
  `approval_timeout_secs`, `default_mode`, `max_turns`, `max_cost_usd`,
  `stream_flush_chars` / `stream_flush_ms`, `max_upload_mb`, and `tools` to
  narrow the surface a Slack-driven run gets — measured on the first live run,
  the schemas of every wired MCP server cost ~7–8k input tokens *per turn*
  before any work happened.

#### Inter-agent messaging

- **`mecha msg`** — `send`, `list`, `show`, `dismiss` and `agents` over a
  file-based mailbox between sessions (`~/.mecha/messages/<recipient>/`). One
  agent, or a person, leaves a short text for another by producer name; the
  recipient's run claims it at the top of a turn and folds it into the same user
  message that carries the tool results — the identical fold point as steering,
  because it is the identical problem. A recipient with no live run loses
  nothing: the message waits in the store until its producer next runs.
- **Taint travels with the message.** A message is otherwise a laundering point,
  since the receiving conversation's interlock never saw what the sender read.
  The harness — never the model — stamps the sender's conversation taint onto
  every message, and delivery merges it into the receiver's before the text
  lands. A tainted overnight run can still report to `chat`; the morning session
  then treats external sends exactly as if it had read the hostile page itself.
- **A message is structurally not the user.** It cannot answer an approval
  prompt, cannot change config, and arrives labelled as another agent's words;
  the receiver's own permissions, hooks, outbox route and interlock govern
  everything it provokes. A full mailbox refuses new sends, which is what
  `dismiss` is for.

#### Review, in the TUI

- **`/outbox` and `/frontdoor`** are modals on the `/triggers` pattern: the
  store read for display, every mutation a `mecha …` child process, and slow
  work — a release's MCP startup, an extraction, a triage run — spawned detached
  and watched, with the outcome reported as a notice when the store shows it.
  Every send confirms, and a draft staged with the trifecta armed is shown in
  red with its full arguments on screen.
- **`/review now|later|auto`** decides what happens when a run stages drafts.
  It is set only by slash command and never inferred from the prompt, because
  release policy must not be decidable by anything sharing a context window with
  third-party text. Scope is an id-diff between submit and completion, so no
  mode touches items another session staged; tainted drafts never auto-release,
  and an errored or early-stopped run releases nothing.
- **`/polls`** — the polls open on the public gate: tallies, close and export.

#### mecha-mail: availability and bookings

- **`calendar_freebusy`** merges busy intervals across every configured account.
  `mecha-mail freebusy --json` is the same answer as data, for a scheduled
  pipeline with no model in it — and unlike the MCP surface, which reports a
  failed account beside the others' results, it fails when *any* account is
  unreadable: a mailbox that could not be read is not a mailbox with free time,
  and a booking page built from a partial answer offers strangers slots the user
  does not have.
- **`mecha-mail bookings`** turns drained booking records into calendar events —
  the inbound sibling of `freebusy`, deterministic and with no model in it. The
  invite is the provider's own, from the user's real mailbox; a cancellation
  withdraws the event and the ledger remembers both directions; reminders fire
  once per tier. Idempotent against `~/.mecha/mail/bookings.jsonl` under a lock,
  so re-running after a partial failure picks up exactly where it stopped and
  two sweeps cannot both create an event.

#### The work directory

- **`~/.mecha/work/<producer>/` is a run's workspace as well as its output
  directory**, where a producer is a trigger's name, or `chat`, or a session id.
  It is stable across runs of the same producer, so yesterday's output is an
  ordinary file in today's run rather than something that has to be plumbed.
- **`mecha work`** — `list` (what each producer has generated, with counts,
  size, and the newest entry), `path <producer>` (for `cd $(mecha work path x)`),
  and `clean` with `--keep`, `--producer` and `--dry-run`. Retention is a policy
  rather than an intention: the last `[work] keep` entries per producer survive,
  the nightly job runs it, and it says exactly what it removed.
- `clean` **never removes anything a published bundle names as a source**, and
  names the entries it kept for that reason instead of skipping them silently.
- `[work] keep` in config (default 10).

#### The public surface, outbound

- **Publishing rides on the outbox.** The factory's MCP tools render a bundle
  locally and stage the publish for review, so nothing reaches a public URL
  without a human releasing it.
- **`[outbox] publish_tools`** names which routed tools are publications. An
  item now carries a kind (`message` or `publish`) that decides how it is
  *reviewed*: `show` leads with the rendered page rather than the arguments,
  `edit` is refused with the real action named (edit the source, re-render,
  publish again), and the writing-reflection miner excludes publishes so a
  changed directory path can never be mined into a voice rule.
- **The kind is config's to declare, never the tool's.** Anything unnamed is a
  `message`, and a name in `publish_tools` that is not in `tools` warns on every
  start.
- **An item records the workspace it was drafted under**, and a release rebuilds
  its tool surface rooted there. A batch release builds one surface per distinct
  workspace, lazily.

#### The public surface, inbound

- **`mecha frontdoor`** — `list`, `show`, `extract` and `next` over the requests
  drained from the public surface into `~/.mecha/requests/`. The whole layer
  serves one sentence: the privileged run sees the extraction, never the prose a
  stranger wrote.
- The extractor is issued a request with an **empty tool list and no
  conversation**, so there is nothing for an injected instruction to reach; an
  extraction failure routes to a human rather than passing the prose through.
- `Record::for_privileged_run` is a function with no argument that returns the
  original text, so the boundary is structural rather than a rule to remember.

- **`triage`, `needs-info` and `close` are how a request reaches an answer**,
  and the queue stops being somewhere requests only accumulate. `triage` drafts
  a reply per extracted request into the outbox and **refuses to run without the
  outbox route** — unrouted, a `mail_send` the model makes actually sends, and a
  stranger's inbox is not where you want to discover `[outbox] tools` was unset.
  Each request gets a fresh `Conversation`, so flagged prose cannot arm the
  interlock for the request behind it. `close` requires a reason. A **rejected
  draft returns the request to `extracted`, never to `closed`** — "not this
  reply" is not "not this request". `reconcile` reads the outbox and updates the
  request store on its own rather than on a verb someone has to remember, and
  leaves a partly-resolved set alone, because some sent and some pending is a
  person mid-review.

#### Elsewhere

- **`mecha outbox review`** walks pending items one at a time, deciding each —
  the overnight-triage case, where nine drafts used to mean nine invocations and
  nine startups of every MCP server. Batching saves the invocations and never
  the reading. Ids may be given several at a time; `--all` is narrowed by
  `--kind` and `--via`; `list` is grouped by kind. A selection naming nothing,
  or a filter matching nothing, is an error rather than "everything".
- **`Tool::carried_state`** lets a tool hand state to a compaction to be carried
  verbatim. The `todo` list reached the model only through the echo in its last
  result — a message, and therefore exactly what a compaction summarises away.
  Exactly one copy survives a second compaction.

### Fixed

#### The agent loop, after the 2026-08-07 Terminal-Bench diagnosis

Twenty-one trials of the arm64 subset broke down as 8 passes, 5 genuine model
failures, and 8 deaths the harness owned some share of. Each fix below names
the trial class that motivated it.

- **Overflow recovery is no longer disabled by a summary that wasn't
  worthwhile.** The recovery arm's give-up flag was set when `compact()` found
  nothing worth summarising — a normal answer for a short transcript already
  saved by thinning — and the flag gated the whole arm, so the next overflow
  propagated as a raw fatal 400 with eviction and thinning never attempted
  (`path-tracing`, dead at 45k tokens in a 32k window). Eviction and thinning
  now run on every overflow; only a summary *request* that failed stops
  further summary attempts.
- **The empty-turn retry allowance resets on any productive turn.** It was
  cumulative across the run, so a long run that recovered from silence early
  was left one empty turn from death for the rest of its life — two trials
  died `NoOutput` mid-task that way, while two others recovered from a nudge
  and passed. `max_turns` is what bounds the total; consecutive silence is
  still bounded by the same retry count.
- **`mecha run` exits non-zero only when the run produced nothing.** Every
  exhausted stop used to exit 3, which benchmark harnesses read as an agent
  crash: a MaxTurns trial was recorded as an agent error while its verifier
  scored the work 1.0. MaxTurns and budget stops with an answer now exit 0;
  `--json`'s `stop_cause` still says which ceiling stopped the run.
- **A crashed run keeps its transcript.** `mecha run` recorded messages only
  after a successful return, so the trials that died mid-flight — the ones
  whose transcripts get read — left three-line session files. Messages and
  taint are now recorded before the error propagates in the unattended
  front-ends (`run`, triggers, frontdoor triage); `chat` and the TUI keep
  their deliberate interactive semantics — a failed turn is dropped so the
  prompt can be retried — but now restore from a snapshot instead of
  truncating a list whose length a mid-run compaction may have changed.
- **A compacted run's transcript records what actually happened.** Recording
  sliced "what the run added" off the end of the message list, but compaction
  rewrites the list in place — the file kept the stale head, skipped the
  rebuilt one (summary included), and a 28-turn trial recorded as 8 assistant
  turns starting mid-conversation. A run that rewrote history now writes a
  `rewrite` record carrying the current state; `load` replaces, the taint
  timeline clamps stale positions (over-taint, never under), and the
  interactive fronts restore from a snapshot on error instead of truncating
  a list whose length compaction may have changed.
- **The per-turn tool-output budget derives from the context window** when
  `[tools] output_budget_bytes` is unset: an eighth of the window in tokens at
  ~3 bytes each, clamped to [6,000, 24,000] — 12,288 bytes at a 32k window,
  the old 24,000 at wide ones. The flat 24 KB was ~8–12k tokens of numeric
  data, larger than the gap between the compaction threshold and a 32k
  window, so one turn's results could leap from under the threshold straight
  past the window.
- **Benchmark trials capture mecha's stderr and tracing.** Harbor recorded
  `stderr: None`, discarding compaction notices and the loop's own account of
  every death; the bench adapter now redirects stderr to a file beside the
  session transcript and runs with `MECHA_LOG=debug`.

#### Jails, workspaces, and who is refusing

- **A path jail rooted where the secrets live.** `setup` now refuses any
  workspace that *contains* the mecha home, because `$HOME` contains
  `~/.mecha/` — the mail OAuth tokens, every session transcript, the learning
  store. `mecha chat` from a home directory was jailed over all of it, and an
  unattended trigger with no explicit workspace was worse: it fell through to
  `current_dir()`, and the shipped systemd unit sets `WorkingDirectory=%h`. A
  workspace *inside* the mecha home is fine and is now the default.
- **`mecha trigger add` writes the workspace down** rather than leaving it
  implicit, and `trigger show` prints the resolved default — "where is this
  jailed" must not be answered by an omitted line.
- **An unconfined MCP server now starts in the run's workspace**, like a
  confined one always did. It used to inherit mecha's own working directory, so
  a server resolving a relative path resolved it against wherever the user
  launched mecha.
- **A refusal by the machine is no longer recorded as a correction by the
  user.** `Decision` is now `Allow | Deny | Blocked`: `Deny` is a human saying
  no and is mined as a correction, `Blocked` is policy and never is. The prefix
  is chosen by the loop from the variant, never by the approver, so no wording a
  front-end picks can mislabel itself. This exposed a live bug —
  `ModeApprover`'s own refusals (a read-only run's, an unattended run's "nothing
  is watching to answer") had been arriving as user denials, so every such run
  was feeding the learning miner corrections from a person who never spoke.
- **A trigger's `notify` command runs in the run's workspace**, as a hook
  already did. It inherited the daemon's directory, so the only way to put the
  answer somewhere was to spell out an absolute path — which is how the morning
  briefing came to end in `mkdir -p ~/.mecha/briefings && cat > …`, outside
  every path jail and unreadable by any later run.

## [0.1.0] - 2026-08-05

First public release. Everything is new, so the whole feature surface is listed
under Added; later releases will record only what changed.

### Added

#### The loop and the library

- **Two crates** — `mecha-core` is a plain Rust library that knows nothing about
  any CLI or application; `mecha` is a thin binary over it. Implement `Tool` to
  add a native tool, `Provider` to add a backend, `Approver` to control what
  needs permission. A provider-agnostic message vocabulary means a transcript
  recorded against one backend can be replayed against another.
- **The agent loop** — ask the model, run the tools it asked for, feed the
  results back, repeat until it stops calling tools. The loop never learns which
  provider is behind it or where a tool came from; both are trait objects.
- **An Anthropic provider** over raw HTTP, with adaptive thinking, thinking
  blocks echoed across tool turns, and `stop_reason: "refusal"` recognised as
  the HTTP 200 it arrives as.
- **An OpenAI-compatible provider** covering llama-server, vLLM and Ollama, with
  streamed tool-call reassembly across arbitrary chunk boundaries, parallel
  calls interleaved by index, and tool calls that survive the
  `finish_reason: "stop"` llama-server reports alongside them.
- **Prompt caching on Anthropic** — a fixed breakpoint covering tools and system
  prompt plus a second moving breakpoint on the last message block, so an
  append-only transcript reads from cache instead of being re-sent uncached
  every turn.
- **Classified failures, with transient ones retried** — rate limits (honouring
  `Retry-After` up to a cap), overload, server and transport errors back off and
  retry; auth, billing, invalid-request and context-overflow never do. A retry
  covers the send only, so it can never duplicate work already shown or acted
  on. `[providers.X] fallbacks` then tries other configured providers on
  exhaustion, turn-local, each answering under its own model name.
- **A sampler you can pin** — `temperature` and `seed` on the OpenAI-compatible
  provider, refused at startup on Anthropic rather than silently dropped, and
  both recorded in the session so a transcript says whether its run was
  repeatable.
- **Budgets** — `max_turns`, `max_output_tokens` and `max_cost_usd`. All three
  end a run the same way: one final turn with the tools removed, so there is an
  answer rather than silence, and `stop_cause` says which ceiling fired. Cost
  prices cache reads and writes separately from fresh input, and reports `null`
  rather than a misleading zero where a provider has no prices configured.
- **Per-run context** — `RunContext` carries the path jail, the approver, the
  budget, a cancellation token and a steering queue, so one agent with one
  provider connection can serve concurrent runs jailed to different directories
  under different permissions.
- **Cancellation** that stops a run at the next safe point and keeps the partial
  answer, the partial assistant turn and the tokens already spent. Tools are
  never interrupted mid-call.
- **Steering** — text queued mid-run is folded into the message carrying the
  tool results, so the model reads the results and the new instruction as one
  user turn and keeps working, without being stopped and restarted.
- **Subagents** — an agent wrapped as a tool, given a rebuilt registry as an
  allowlist rather than an inheritance, with optional per-profile model,
  provider, turn limit and system prompt. A child's output is untrusted by
  default; `trusted_output` overrides that as a deliberate risk decision.
- **Layered TOML configuration** — built-in defaults, `~/.mecha/config.toml`, a
  project-local `./mecha.toml`, environment variables, then CLI flags, each
  level overriding only the fields it names. A global-only load exists for runs
  that no working directory should shape.

#### Interfaces

- **`mecha run`** — one task, one answer, with `--json` for machine-readable
  output, `--resume` to continue a recorded session, and exit codes that
  distinguish success, error, refusal and turn exhaustion.
- **`mecha chat`** — a readline REPL with slash commands and input history saved
  after every accepted line, so a killed process keeps it.
- **`mecha tui`** — full-screen, with the input line live while the agent works:
  Enter starts a run when idle and steers one already going. Streaming output,
  scrollback that re-arms follow mode, session persistence and `--resume`.
- **TUI slash commands** — `/help /tools /model /provider /mode /mcp /usage
  /todo /triggers /clear /session /exit`, with modal pickers, name completion,
  and mid-session switching of model, provider, permission mode and individual
  MCP servers. A switch appends a configuration record, so a replay diffs
  against what actually ran.
- **TUI keys** — `?` for a full key reference, `^O` to reveal tool output and
  reasoning retroactively, `^G` to compose in `$EDITOR`, `!command` to run a
  shell command locally with no model and no taint, `@path` completion against
  the workspace, and Shift+Enter for a newline where the terminal supports the
  kitty keyboard protocol.
- **TUI rendering** — a context fuel gauge that colours at 75% and 90%, a live
  todo pane, subagent work rendered nested under the call that spawned it (still
  correct when delegations run in parallel), atomic frame presentation, a tab
  title that says whether a run is in flight, and cached history cells so
  drawing no longer costs O(transcript) per streamed token.
- **`mecha batch`** — the same agent over a JSONL file of prompts at bounded
  concurrency, results streamed to the output file as they finish and keyed by
  id, so a killed run leaves everything completed so far on disk. Every item
  gets its own conversation.
- **`mecha tools`** — the tool surface with no provider configured, including
  each tool's declared capabilities, the active sandbox, which MCP servers are
  unconfined, and `--schema` for exactly what the model sees.
- **`mecha sessions list | show | path | stats`** and
  **`mecha config show | path | init`** — inspect saved transcripts, roll up
  tokens, turns and cost by provider and model over a window of days, and see
  what settings are in effect.

#### Tools

- **Six built-in tools** — `fs_read`, `fs_write`, `fs_edit`, `fs_list`, `shell`
  and `http_fetch` — plus `todo`, a task list the model rewrites as it goes,
  kept outside the message history so it survives compaction intact.
- **`ask_user`**, registered only by front-ends that own a human, so the model
  can stop and ask instead of guessing at an under-specified task. Declining is
  a legitimate answer and returns a tool result, not a failed run.
- **`web_search`** behind a `SearchBackend` trait with SearXNG, Exa and Tavily
  tried in order and falling through on failure, and a `depth` argument
  selecting a cheap round trip or a deep one.
- **An approval gate** with `ask`, `allow` and `read-only` permission modes,
  plus a planning phase that does not offer writing tools at all rather than
  offering them and refusing the call — enforced on both the advertised list and
  the dispatch path, and inherited by subagents.
- **A per-turn tool output budget** divided across a turn's concurrent calls, so
  one runaway tool cannot starve its siblings. What gets cut is written to a
  spill file, and the marker names the path and the line the elision starts on,
  so recovering the rest is one read.
- **An MCP stdio client** that surfaces remote tools as the same `Tool` trait,
  namespaced `<server>__<tool>` so two servers can both expose a `search`. It
  follows `nextCursor` pagination, accepts JSON-RPC ids in either numeric or
  string spelling, and routes a server's stderr through tracing instead of the
  terminal.

#### Security

- **The path jail** — every model-supplied path is canonicalized and proven to
  sit inside the workspace before anything touches disk; `..`, symlinks and
  absolute paths outside the root are refused.
- **The lethal-trifecta interlock** — tools declare `private_data`,
  `untrusted_input`, `external_send` and `destructive`; the loop tracks which
  have entered the conversation and refuses any sending tool once both private
  data and untrusted content are present. It sits ahead of the approver, because
  a human clicking yes is what an injection is trying to engineer.
  `trifecta = "ask" | "allow"` changes the policy deliberately and visibly.
- **Taint is a property of the conversation**, not of one run, and it is
  recorded in the session file — so a new turn does not reset it, resuming does
  not launder it, and compaction does not summarise it away. A new conversation
  (a batch item, a subagent, an eval case, a trigger fire) starts clean.
- **`block_sends_after_private`** — an opt-in second control aimed at ordinary
  privacy leaks rather than injection: any outbound tool is refused once private
  data is in context.
- **SSRF protections on `http_fetch`** — hostnames are resolved and loopback,
  private, link-local (including the cloud metadata endpoint) and CGNAT
  addresses refused; the connection is pinned to the addresses that passed, so a
  short-TTL DNS answer cannot swap them afterwards; redirects are not followed;
  `allowed_domains` and `blocked_domains` narrow it further.
- **A sandbox for `shell`** — `[sandbox] kind = "bwrap" | "docker" | "none"`. A
  confined command gets the workspace, a read-only system, no home directory, no
  environment beyond a named allowlist and by default no network. A configured
  sandbox that does not work stops the run at startup rather than degrading to
  unconfined execution.
- **MCP servers get the same treatment** — the child environment is an allowlist
  (`PATH`, `HOME`, `LANG`, `LC_ALL`, `TZ`, plus whatever `env_passthrough` names
  and `env` sets) rather than an inheritance, per-server `sandbox = true`
  confines the process, and per-server `network` overrides the global switch.
  `[mcp.capabilities]` can distrust a server further than its own annotations
  claim, never less.
- **Bounded output and owner-only stores** — `shell` discards beyond a per-stream
  cap as it arrives rather than buffering without bound and kills a command at
  its timeout; sessions, the outbox, the learning store, the spill directory and
  the mail credential directory are created at mode 0700, token files at 0600.

#### Hooks

- **`[[hook]]` commands** at `pre_tool`, `post_tool` and `session_end`, with the
  event as one JSON object on stdin, so policy, redaction and logging attach
  without editing the loop.
- **The dispatch order is interlock, then hook, then approver** — a hook can
  narrow policy and never loosen security, and a `pre_tool` denial never reaches
  the human.
- **`pre_tool` fails closed** — exit 0 allows, exit 2 denies, and an undefined
  exit code, a spawn failure or a timeout also deny. `post_tool` and
  `session_end` are observers whose failures are swallowed. Subagents inherit
  the parent's hooks, and a typo'd event name is a startup error on every run
  rather than a warning only on the runs that needed it.

#### The outbox

- **`[outbox] tools`** names tools whose calls are staged rather than executed:
  the loop intercepts the call, writes it to `~/.mecha/outbox/`, and tells the
  model it is a draft awaiting release. Draft-only becomes structural, so an
  email tool — including a third-party MCP server's — needs no knowledge of the
  outbox to be covered by it.
- **`mecha outbox list | show | edit | send | reject`** — review the queue, edit
  a draft in `$EDITOR`, release it (executing the real tool under the store lock
  so two sends cannot double-fire), or reject it with a reason.
- **Staging skips the interlock and the approver** because nothing leaves the
  machine at stage time; the item records the conversation's taint snapshot, and
  review warns and confirms when a draft was written with the trifecta armed. A
  staging that fails returns an error to the model rather than falling through
  to execution. Subagents inherit the route, and a routed name matching no
  registered tool warns at every start.

#### Learning

- **`mecha reflect`** mines recorded transcripts for the moments the user
  stepped in — a mid-run steer, a denied tool call, a corrective follow-up turn,
  an edited outbox draft — and asks a model for the reusable lesson behind each,
  appending it with the session id that proves it.
- **`mecha learn`** consolidates unprocessed reflections into
  `rules/<domain>.learned.toml` within a fixed budget and records which
  reflections it consumed. `--holdout` keeps a deterministic every-k-th slice
  out of the pass, so measurement has data the rules never saw.
- **`mecha validate`** probes whether the rules change an answer. Follow-up
  probes re-ask the corrective turn and are judge-graded; steer and denial
  probes are counterfactual replays graded structurally on the trace — did the
  model do the steered thing without the steer, did it repeat the exact call the
  user refused.
- **A file-based learning store** under `~/.mecha/learning/`, which is a git
  repository: `git log` is the learning history and `git revert` is the undo.
  User rules are never written by code, learned rules are freely editable, and
  an audit record per pass names the rules before and after. Learned rules ride
  in the system prompt inside the cached prefix, changing only at consolidation
  time; `--no-learned-rules` opts out anywhere.
- **Provenance gates what may become a rule** — every reflection carries an
  origin classified by deterministic code from the transcript's recorded taint,
  and `mecha learn` excludes anything not clean before a prompt is built. It
  fails closed: an unknown position, a torn transcript or a reflection written
  before the field existed all classify as untrusted. Excluded evidence stays in
  the archive.
- **Unattended learning never applies its own output** — `mecha learn --propose`
  measures a candidate rule set by counterfactual replay against the deployed
  one, rejects a candidate that regresses any probe before a human sees it, and
  stages the rest. `mecha proposals list | show | accept | reject` is the
  review, and acceptance checks the live rules still match what the candidate
  was measured against.
- **Rule tenure** — rules carry an id, sources and a creation time; `mecha
  validate` appends every probe outcome to a validation ledger keyed to the
  exact rule set measured, and a regressed trace-graded probe bisects the active
  rules to name the one that flips it. **`mecha rules`** folds that ledger into
  per-rule tallies and stages retirement
  through the same proposal gate as any other rule change once a rule
  accumulates attributed regressions. Retirement is a flag, never a deletion:
  the rule stays as evidence, the learner is shown it as measured harmful, and
  `rules restore` undoes it. A hard per-domain cap on the always-loaded block is
  warned about at startup and refused in `mecha learn`, so consolidation may
  shrink or rewrite an over-cap set but never grow past it.
- **A nightly cycle** — `scripts/ruminate.sh` chains reflect, distill, validate,
  learn and the retirement scan, with a systemd user timer to fire it. Every
  stage is idempotent, a store writer lock serialises concurrent passes, and a
  night with the model server down defers entirely rather than half-running.

#### Distillation

- **`mecha distill`** summarises each closed session into an episode staged to a
  knowledge-graph MCP server through its `kg_upsert` tool — evidence rather than
  belief, so the extracted facts wait in that graph's own review queue. It is
  idempotent at both ends: a local ledger under the learning store's writer
  lock, and the graph's own source key making a re-push an update.
- **A tainted session still distills**, with its taint snapshot recorded on the
  episode instead, because losing the record of a real afternoon because a web
  page was open would gut the memory. Unknown taint is recorded as unknown.

#### Triggers

- **`mecha trigger add | list | show | edit | rm | enable | disable | next |
  run | tick | daemon | cancel | runs`** — a prompt on a five-field cron
  schedule, run unattended. `tick` fires what is due and exits; `daemon` is a
  loop over it, so a crontab line or a systemd timer reaches the same answer and
  `tick --dry-run` is an honest preview rather than a second implementation of
  the schedule. `scripts/mecha-triggers.service` ships the daemon as a systemd
  user unit.
- **A hand-rolled cron parser** resolved in an IANA zone recorded on the trigger
  at authoring time, handling daylight saving in both directions: a job inside
  the spring-forward gap fires at the first instant that exists, and one inside
  the repeated autumn hour fires once.
- **Missed slots collapse** — a machine off for a week owes one run of each
  trigger rather than a week's worth, and `--catch-up` (`always`, `never`, or a
  duration) decides whether a stale slot still runs. A skip is written to the
  ledger, so "why did I not get my briefing" is answerable.
- **Triggers live in `~/.mecha/triggers/`, never in the layered config**, and a
  fire loads the global config only — a project file arrives with a cloned
  repository, and a scheduled agent run is not something a repository should be
  able to declare. Runs are read-only unless the file says otherwise, with
  outbox-routed calls still staging under read-only because staging executes
  nothing.
- **One run per trigger at a time** via a flock the kernel releases if the
  process dies, with the overlap skip recorded. The timeout, a daemon SIGTERM
  and `trigger cancel` all cancel rather than abort, so the partial answer and
  the ledger row survive. A manual `trigger run` records a row with no slot, so
  testing a trigger cannot silently disarm its schedule.
- **`/triggers` in the TUI** — see, edit, enable, run and cancel what is
  scheduled, with the detail view reading the last answer back from the session
  transcript. Every action shells out to `mecha trigger`, so firing cannot
  freeze the interface and the TUI can do nothing the command line cannot.

#### Compaction

- **`[agent] compact_at_tokens`** summarises the middle of a transcript once the
  provider-reported prompt size passes it, keeping the task at the top and the
  recent turns verbatim. Off by default, and derived from `context_window` at
  two thirds where that is configured.
- **Superseded tool results are evicted first** — when a later call covers the
  same target, the older result is replaced with a marker naming the recovery,
  so a write supersedes an earlier read of the file it changed. Errors neither
  supersede nor get evicted.
- **Old tool results are then thinned** — results are truncated from the head
  with a marker saying so, while the calls themselves are left alone, which
  keeps the sequence and therefore the agent's place in a traversal. Only if
  that is not enough is a summary taken, and the cut is chosen so no
  `tool_result` is ever orphaned from its `tool_use`.
- **Summaries are validated before installing** — one truncated by its own token
  limit is refused deterministically, and a second tool-less call reads the
  summary beside the transcript it replaces and names what is missing,
  triggering exactly one regeneration. An unusable verdict installs with a
  warning, because a run that needs to compact to survive must still compact.
- **Overflow recovery** — a prompt the provider refuses as too large is
  recognised across backends, compacted and retried once instead of ending the
  run.
- **A loop guard armed by compaction** — an identical call with an identical
  result, repeated within a short window after a compaction, stops the run with
  a distinct `loop` stop cause rather than burning the turn budget re-living
  what the summary dropped. Polling never trips it.

#### Sessions and replay

- **Append-only JSONL transcripts** in `~/.mecha/sessions`, recording messages,
  taint checkpoints, usage summaries and a configuration record per attach —
  provider, model, system prompt, tool list, effort, budgets, permission mode,
  sandbox, sampler, timezone — so a replay knows what shaped the run rather than
  diffing two variables at once.
- **`mecha replay <session>`** re-drives a recorded session against the current
  build using recorded tool results and a sequential loop, rebuilding the run
  from the recorded configuration rather than today's flags; `-p`/`-m` replay
  against a different model. A structural divergence refuses the call and stops
  the replay, while an argument-only difference replays the recorded result and
  is reported for the caller to judge. `scripts/replay-regression.sh` replays
  pinned sessions and fails on any drift, turning recorded real work into
  standing regression cases; pins are machine-local, because transcripts are
  personal data.

#### The eval rig

- **`mecha eval [cases.jsonl]`** scores a model on a case set, graded on the
  tool-call trace first and the text second, with a scorecard broken down by tag
  and `--compare` to put models side by side. It exits non-zero on failure, so it
  doubles as a regression gate on the harness.
- **Deterministic checks** — which tools were called, in what order, with what
  arguments, and what the answer said. Two apply to every case whether it asks
  or not, because they disqualify a model regardless of the answer: malformed
  tool arguments and invented tool names.
- **`expect.verify`** runs a command in the case's workspace afterwards and
  grades the exit code, hashing the test file first so a model that edits the
  tests until they pass still fails. **`expect.judge`** grades a rubric with a
  second model, for cases whose right answer is a judgement.
- **Run-metadata checks** — `stop_cause`, `taint`, `blocked_sends` and
  `min_compactions`, which are the only way to grade the harness rather than the
  model, since none of it appears in the answer text.
- **Per-case controls** — `sandbox` for a private copy of the fixture with
  writes allowed, `max_turns` for a case that genuinely takes twenty steps,
  `compact_at_tokens` to force compaction for one case alone, and a list-valued
  `prompt` for several turns on one conversation.
- **`--runs k`** repeats every case k times and reports pass^k beside pass@k,
  with independent workspaces per run and a warning when a pinned seed at
  concurrency 1 would make the k samples one sample counted k times.
  **`--ab-rules`** runs the set rules-free and then rules-on and reports the
  per-case flips as their own artifact rather than as a comparable scorecard.
- **Reproducibility by construction** — eval forces MCP, hooks, learned rules,
  the outbox and provider fallback off, because a scorecard shaped by one
  machine's local configuration is not comparable to anyone else's.
- **Generated fixtures and a second case set** — `scripts/build-eval-fixtures.py`
  rewrites the workspace, prints the gold answers the cases must assert, and
  checks that each code kata fails as shipped and is solvable by a reference
  fix; `eval/pkg-cases.jsonl` runs against fixture MCP servers via `--mcp-file`
  and grades the trifecta interlock end to end, offline.

#### mecha-mail

- **A library plus three MCP binaries** — Gmail and Google Calendar v3, Outlook
  mail and calendar over Graph, both OAuth flows and the token lifecycle. The
  library is what a GUI would depend on directly; `mecha-google` and
  `mecha-outlook` each serve one provider with its own credential store.
- **`mecha-mail`, the account-based surface** deployments should wire: every
  account in `~/.mecha/mail/` behind one provider-neutral set of tools, so
  neither mecha nor the model knows Google or Microsoft exists. Account names
  are baked into every tool schema as an enum at startup, so the model picks
  from real names. Reads fan out across every mailbox and tag each row with its
  account; item operations name the account their id came from; creates use the
  configured default or ask the user. A failed account is reported beside the
  other accounts' results rather than sinking the whole call.
- **`mecha-mail auth <name> --provider ...`**, with `import` to copy a legacy
  per-provider login in and `default <name>` to name the sending account.
  Microsoft signs in with device code, so it needs no redirect URI and no
  forwarded port, and works over SSH.
- **Unified replies** — `mail_reply` takes a thread id and answers the newest
  message, synthesizing Gmail's addressing (answer the sender, keep everyone on
  reply-all, never the user's own address) where Graph does it natively.
- **The token lifecycle in Rust** — credentials at mode 0600, refresh ahead of
  expiry behind a lock so two concurrent tool calls cannot race, one forced
  refresh and retry on a 401, retry with backoff on 429 and 5xx, and an
  HTML-to-text fallback so an HTML-only email no longer reaches the model as an
  empty body.
- **Capability labelling that matches the risk** — reads are untrusted sources
  but not send sinks, because a search query travels only to the party that
  already custodies the mailbox; sends and calendar writes reach third parties,
  are marked open-world, and are named in `[outbox] tools` so they stage rather
  than deliver. A drafted `to`, `cc`, `bcc` or `subject` containing CR, LF or
  NUL is refused rather than stripped, so it cannot smuggle a hidden recipient
  into a raw message.
- **Timezone rendering** — `[agent] timezone` is an IANA name that rides in the
  system prompt and reaches the mail servers as `MECHA_TZ`, so event times are
  rendered in the user's zone before the model ever sees them.

#### Testing, benchmarking and docs

- **A three-layer test suite** — unit tests for anything that is a function of
  our own code (including a `ScriptedProvider` that exercises tool dispatch,
  denials, exhaustion and recovery with no network), integration tests for what
  is deterministic but needs real execution (docker actually confining a
  command, an MCP server actually receiving an environment), and eval cases for
  what only emerges with a model in the loop. `MECHA_TEST_REQUIRE_BACKENDS=1`
  turns every environment-based skip into a failure, because in CI a silently
  skipped test reads exactly like a passing one.
- **A Harbor adapter** (`bench/`) that installs the `mecha` binary inside a
  Terminal-Bench task container and runs it there, so mecha's own loop, tools,
  path jail and budgets are what get measured, under the same
  no-MCP/no-hooks/no-outbox posture eval forces.
- **Research and design notes** under `docs/` covering context management,
  memory and rule lifecycle, verification, sandboxing, prior art, public
  benchmarks, the TUI survey, and a branching design recorded as a deliberate
  non-implementation.

[Unreleased]: https://github.com/ljchang/mecha/compare/v0.1.6...HEAD
[0.1.6]: https://github.com/ljchang/mecha/releases/tag/v0.1.6
[0.1.5]: https://github.com/ljchang/mecha/releases/tag/v0.1.5
[0.1.4]: https://github.com/ljchang/mecha/releases/tag/v0.1.4
[0.1.3]: https://github.com/ljchang/mecha/releases/tag/v0.1.3
[0.1.2]: https://github.com/ljchang/mecha/releases/tag/v0.1.2
[0.1.1]: https://github.com/ljchang/mecha/releases/tag/v0.1.1
[0.1.0]: https://github.com/ljchang/mecha/releases/tag/v0.1.0
