# Agent TUIs, read against this one

Research pass, 2026-08-05. The question was what the good agent TUIs do that
`mecha tui` does not, and which of it is worth stealing.

| Project | Stack | What was read |
|---|---|---|
| [codex](https://github.com/openai/codex) | Rust, **ratatui + crossterm** — the same stack as mecha | TUI architecture (DeepWiki), the shortcut/slash-command reference, the keymap/theme/statusline customisation reference |
| [pi](https://github.com/badlogic/pi-mono) | TypeScript, custom `pi-tui` | pi-tui architecture, coding-agent README |
| [opencode](https://github.com/sst/opencode) | Go, Bubble Tea, **thin client over an HTTP server** | TUI docs, architecture notes |
| [crush](https://github.com/charmbracelet/crush) | Go, Bubble Tea / Charm stack | README |
| [Claude Code](https://claude.com/claude-code) | TypeScript, forked Ink + yoga-layout | architecture write-ups (third-party) |
| — | — | two TUI-design essays, for the principles rather than the products |

Docs and third-party write-ups, not source, for everything except mecha. Same
discount as `PRIOR-ART-RESEARCH.md`.

---

## Where mecha's TUI actually stands

Read from the source, so this part is not a guess. `mecha-cli/src/tui/` is
2,439 lines: `mod.rs` (1,519), `command.rs` (338), `transcript.rs` (199),
`approve.rs` (81), `ask.rs` (52), plus `render.rs` (250).

**Already there, and several of these are things other TUIs get wrong:**

| | Detail |
|---|---|
| Layout | vertical `[transcript, status(1), input(n)]`, input grows with content |
| Modals | question / picker / approval drawn as centred overlays; approval takes **every** key |
| Status line | model, provider, tool count, plan badge, elapsed timer, token counts, **context fuel gauge** colouring at 75%/90%, "scrolled" indicator |
| Steering | type while running; status says `· type to steer · ^C to stop` |
| Paste | bracketed paste on — a pasted newline inserts instead of submitting three half-prompts, and a dragged file arrives as one event |
| Mouse | capture on, wheel scrolls the transcript |
| Follow mode | auto-follows the tail; scrolling back detaches; scrolling to the bottom **re-arms it silently** |
| Panic safety | panic hook restores raw mode, alt screen, mouse, bracketed paste |
| Completion | Tab completes slash commands to their common prefix; Shift+Tab toggles plan mode with a transcript notice |
| Live switching | model / provider / permission mode / MCP mid-session, recorded into `RunConfig` |

That is a better starting point than the survey led me to expect. The plan
badge shown *only while planning* ("a badge that is always there stops being
read") is a considered decision that two of the surveyed projects do not make.

**Missing, and worth naming before the recommendations:** no help overlay, no
themes or configurable keys, no `@` file completion, no `!` shell escape, no
external-editor compose, no message editing or branching, no diff view, no
copy-to-clipboard, no notifications or terminal title, no live todo pane, and
subagent tool calls render flat rather than nested (the last two are already
in `HANDOFF.md` as asked-for).

---

## 1. The one architectural thing: immutable history cells

**The finding.** codex — same ratatui stack — splits its transcript into a
collection of **committed, immutable `HistoryCell`s** plus a single **mutable
`active_cell`** that changes during streaming. Only the active cell re-renders
per frame; committed cells are rendered once and cached. Its own note is that
this "optimizes for both scrollback efficiency and streaming latency".

**Why it matters here, concretely.** `Transcript::draw`
(`transcript.rs:154`) does this on **every frame**:

```rust
let lines = self.lines();                        // rebuild every line, every entry
let paragraph = Paragraph::new(lines).wrap(...);
let height = paragraph.line_count(area.width);   // re-wrap the entire transcript
```

So the cost of drawing one frame is O(whole transcript), and it is paid again
for every streamed token, every timer tick, every keystroke. A short session
will never notice. A long-horizon run — which is the workload this project
measures at ~17.5 turns and is building compaction for — will, and the failure
mode is the input line going sticky exactly when the run is most interesting.

**The port:** an `Entry` becomes a cell that caches `(width, Vec<Line>)` and
invalidates on width change; the transcript keeps a running wrapped-height
total so `line_count` is not recomputed; only the tail entry is rebuilt while
streaming. This is a contained change to `transcript.rs` and it is the one
item here with a measurable before/after — which is the kind this project
prefers.

**pi's version of the same insight** is lower-level and also worth taking:
differential rendering plus **CSI 2026 synchronized output**, so a frame is
never shown half-drawn. crossterm 0.28 already has
`BeginSynchronizedUpdate`/`EndSynchronizedUpdate`; wrapping `terminal.draw` in
them is a few lines and mostly pays off over SSH, which is how this box is
used.

---

## 2. Alt screen or scrollback — a fork mecha has taken by default

codex makes this **configurable** (`tui.alternate_screen`, with `never`) and
writes *finalized* transcript output into the terminal's real scrollback,
rebuilding it on resize. Claude Code renders inline for the same reason. mecha
enters the alt screen unconditionally (`mod.rs:1405`).

The consequences of the alt screen, all of which mecha currently has:

- the transcript **disappears on exit** — nothing to scroll back to, nothing to
  paste into a bug report;
- the terminal's own scrollback and search do not work on it;
- **mouse capture also takes native text selection**, so selecting an error
  message with the mouse does not work either. mecha enables capture and uses
  only the wheel, so the cost is being paid for scroll alone.

The alt screen is genuinely the right default for a full-screen app, and the
TUI-design essays list it as a non-negotiable. But three cheap mitigations are
worth more than the debate:

- **`[tui] alternate_screen = false`**, codex's escape hatch, for tmux/zellij
  users and for anyone who wants the transcript to persist;
- **a mouse toggle** (opencode exposes exactly this) so selection can be got
  back without losing wheel scroll permanently;
- **copy** — `Ctrl+X`/`/copy` copies the last response, and `/export` writes
  the transcript to Markdown. pi, codex and opencode all have one or both.
  mecha records session JSONL already, so `/export` is a formatter over data
  it holds.

---

## 3. Steer *and* queue, as two different keys

codex's composer draws a distinction mecha does not:

| Key | Behaviour |
|---|---|
| **Enter** | inject into the **running** turn — steer |
| **Tab** | **queue** for the next turn, do not interrupt |

mecha only has the first. That is the same `steer` / `followup` split
`PRIOR-ART-RESEARCH.md` picked up from openclaw's queue modes — and the
insight from surfacing it here is that it wants to be a **key, not a config
setting**, because the choice is per-message and made in the moment: "you're
going the wrong way, stop" versus "when you're done, also do this."

mecha's steering already folds text into the message carrying tool results, so
the queue variant is the easier of the two: hold the text, submit it as a
normal user turn when the run ends. Tab is currently completion, so this needs
a different binding — which is its own argument for §6.

---

## 4. Branching, and why mecha should have it before anyone else

Two independent implementations of the same idea:

- **codex**: `Esc Esc` on an empty composer edits your previous message; keep
  pressing to walk further back through the transcript. Plus `/fork`.
- **pi**: sessions are JSONL with a **tree** structure, so branching happens
  **in place without creating new files**. `/tree` navigates, `/fork` splits,
  and "all history remains recoverable within single files".

This is the item I would build second, because mecha is unusually ready for it
and unusually well-motivated:

- sessions are already append-only JSONL with a recorded `RunConfig` per
  attach;
- `replay_run.rs` already drives a recorded prefix through a fresh agent, and
  `replay.rs` already computes structural-vs-argument divergence;
- `counterfactual.rs` already exists on the premise that **an intervention is a
  test case** — "would the model do what the steer asked, without being
  steered".

A `/fork` in the TUI is that machinery pointed at the user instead of at the
nightly learning pass. "Rewind three turns and say it differently" is a
counterfactual the user runs by hand, and the diff view for it is already
written. No other surveyed project has the replay half; they have the UI half.

The one thing to get right is what pi got right: **branch in place, one file**.
mecha's session format is append-only, so a branch is a parent pointer on an
entry, not a copied file — and copying files would break the mined/unmined
bookkeeping the learning store keeps per session.

---

## 5. Progressive disclosure, and the cheap wins

The design essays converge on a three-tier rule: **a footer with 3–5 essential
keys, a `?` overlay with everything, docs for the rest.** mecha has tier one
(contextual hints in the status line, shown only while running — good) and
tier three, and nothing in between. A `?` overlay listing the bindings is an
hour's work and is the single highest ratio of usefulness to effort here.

Others in the same class, roughly by value:

- **Collapse/expand tool output** (pi `Ctrl+O`, codex `Ctrl+T` transcript
  overlay). `Transcript` already has a `verbose` flag — it is fixed at
  construction. Making it a live toggle is nearly free and directly addresses
  reading a 200KB `shell` result.
- **Nested tool-call rendering for subagents.** Already in `HANDOFF.md` as
  asked-for. Every surveyed TUI indents nested calls; mecha renders flat, which
  makes a subagent's work indistinguishable from the parent's.
- **A live todo pane.** Also already asked-for, and cheaper than it looks: the
  list is in a `Mutex` the TUI can read. Worth noting the finding it interacts
  with — the *model* has no read path to that list except the echo in its last
  `todo` result, so a pane helps the human and does nothing for the model. The
  model-side fix is re-injection at compaction time, which is a different item.
- **`@` file completion.** `command.rs` already implements completion with a
  common-prefix fill; pointing the same machinery at workspace paths is the
  same code with a different candidate source. Every surveyed TUI has this.
- **`!` shell escape** (opencode, codex) — run a command, show the output,
  **do not send it to the model**. Useful precisely because it is not a tool
  call: no approval, no taint, no tokens.
- **External-editor compose** (`Ctrl+G` in codex and pi). mecha already owns
  the one `$EDITOR` shell-out pattern in `outbox.rs`; reuse it.
- **Terminal title** (codex `tui.terminal_title`: spinner, project, model,
  branch). Cheaper than desktop notifications, works over SSH, and answers "is
  it still going" from a tab strip. Notifications (crush, opencode's
  "attention system", codex) are the richer version and matter for the
  minutes-long local-model runs this box does, but the title is the 20-line
  version of the same value.

---

## 6. Themes and keymaps — take the accessibility, defer the theming

codex ships **32 themes** and **seven context-specific keymap layers** (global,
chat, composer, editor, pager, list, approval) with every action rebindable.
opencode's theme type has **62 semantic colour properties**. mecha hardcodes
`Color::Cyan`, `Color::Yellow`, `Color::Magenta` at the call sites.

The full versions of these are a lot of surface for a single-user tool. Two
pieces of them are worth taking now, and they are the accessibility pieces
rather than the customisation ones:

- **Pull colours into one semantic table** — `ok`, `warn`, `danger`, `muted`,
  `accent` — rather than literal colours at call sites. That is the
  prerequisite for everything else and it is a mechanical refactor.
- **Honour `NO_COLOR`, and check the 16-colour rendering.** The design essays
  are unanimous: design monochrome first, add 16 colours, then true colour,
  each tier working alone. The fuel gauge going yellow then red is mecha's one
  load-bearing use of colour, and it must still be legible without it — today
  the percentage is in the text, so this is close to already true, and worth
  confirming rather than assuming.

Keymap configuration can wait. But note the constraint it would solve: Tab is
taken by completion, so §3's queue key has nowhere obvious to go, and that is
what a keymap layer is for.

---

## 7. Testing the thing

mecha's TUI has six unit tests — picker wrapping, input layout under wrapping,
pasted newlines, a zero-width terminal. All good, all about arithmetic.
Nothing renders a frame.

`CLAUDE.md` already records the pty method and its trap (`script -qec "stty
rows 45 cols 130; mecha tui" /dev/null`, because a pty with no window size
renders every frame into 0×0). That is the right tool for end-to-end checks and
too heavy for regressions.

The missing middle is ratatui's own `TestBackend`: render into an in-memory
buffer and assert on it, or snapshot it. That would cover the things most
likely to break silently — the status line under each state, a modal at a small
terminal size, the transcript at exactly the height where scrolling clamps —
none of which any current test touches. It is also the natural place to pin
§1's caching change, since a cached and an uncached render must produce
identical buffers.

---

## What not to take

- **The client/server split** (opencode: TUI as a thin Bubble Tea client over
  HTTP + SSE; crush's multi-client workspace sharing). It buys attaching
  several clients to one session, which mecha does not need — and mecha's
  steering works *because* the TUI owns the agent in-process and can push into
  a queue the loop is reading. Putting a socket in the middle would make the
  one thing this TUI does better than a REPL into the hard part.
- **Config as executable Bash** (crush's `crushrc`). mecha's layered TOML has a
  round-trip guard that exists because a config field was once unreachable and
  every unit test still passed. Executing config trades that for flexibility
  nobody asked for.
- **A flexbox layout engine** (Claude Code embeds yoga-layout). ratatui's
  constraint layout is not the bottleneck; the transcript re-wrap is.
- **Vim mode.** codex added it late, it is a large surface, and nothing in this
  project's usage suggests it.
- **Image rendering** (pi supports Kitty/iTerm2 graphics protocols). mecha is
  text-only end to end — a `message.rs` change, not a TUI one, and already
  recorded as deferred.

---

## Not researched

- **The Kitty keyboard protocol.** pi uses it to distinguish Shift+Enter from
  Enter, which is how it gets multi-line input without stealing a key. crossterm
  supports `PushKeyboardEnhancementFlags`, so this is available — but it needs a
  fallback path for terminals without it, and I did not check what mecha's
  current terminals report.
- **Crush's actual TUI**, beyond its README. Its distinctive claims — a
  permission *queue* rather than one modal at a time, and multiple clients
  attached to one workspace — are the two most interesting things in the survey
  I could not verify.
- **`ghostty-automator`-style terminal automation** ("Playwright for
  terminals": an agent reads the real rendered cells, colours and cursor). It
  surfaced as the direction for closing the loop on TUI development, and it is
  a much bigger idea than this file — possibly the right way to give the eval
  rig a TUI case at all.
- **How any of these render diffs.** codex has `/diff` and `/review`; mecha has
  no diff view despite `fs_edit` being a core tool, and a rendered edit is
  probably the most-read thing in a coding session.
