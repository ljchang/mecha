---
title: Incognito chat
sidebar_position: 2.5
description: A web chat that leaves nothing behind once it closes — no transcript, no title, no count, no file — and what it gives up to keep that promise.
---

# Incognito chat

An incognito chat is a [web chat](/docs/features/interfaces/web) that mecha
does not keep. While it is open it works like any other chat: it can read your
mail and calendar, search the web, work with files and draw pictures. Once it
closes, **no file, record, log line or process on this machine holds anything
derived from it** — not its text, its tool calls, its uploads, its images, its
title, or the fact that it happened.

Open one with the second new-chat button, beside **+**. A banner stays at the
top of the page for as long as the chat is open, and **End** sits in the
header.

## When it closes

The first of these ends it:

- **End.** Everything in it is deleted at once, a reply in progress included.
- **30 minutes with no turn and no open page.** An open page counts as use: it
  pings the server once a minute, so a chat you are reading stays open. A tab
  you closed cannot tell the server, so the timeout is what ends that chat.
- **`mecha serve` stopping**, for any reason.

While it is open, the drawer lists it as *incognito chat*, with no title. A
closed chat is gone for good: the page says so and offers a new one, and the
drawer never shows a closed one among earlier conversations — there is
nothing to reopen, and no "save this conversation" button, by design.

## How it keeps nothing

An ordinary web chat writes a transcript to `~/.mecha/sessions/` from the
moment it opens, and every nightly loop — reflection, distillation into the
[knowledge graph](/docs/features/memory/graph), the
[run-quality corpus](/docs/features/learning/run-quality) — reads from those
transcripts. An incognito chat has **no transcript at all**: in the code, it
is a different kind of session with nowhere to write, rather than a normal
session marked "don't read me". So there is nothing for any later reader to
find, and nothing for the learner to learn from. Staying out of the learning
loop is the point, not a side effect.

The rest follows from that:

- **Its files live in RAM.** Uploads, the model's files and oversized tool
  output go in a folder under `$XDG_RUNTIME_DIR`. mecha checks that the
  folder is on tmpfs before offering incognito at all, so "deleted" means
  gone rather than unlinked on an SSD. The folder is removed when the chat
  closes. If `mecha serve` dies before it can clean up, the next start sweeps
  the leftovers before it accepts any connections.
- **No title, no situation brief, no hooks, no voice call.** The titler sends
  your first turns to the model and stores the result. The brief reads the
  task board through the graph server, and that read is itself a record.
  [Hooks](/docs/features/security/hooks) receive tool inputs and outputs, and
  the voice worker logs what it hears. So none of them runs.
- **The browser keeps nothing either.** Every incognito response carries
  `Cache-Control: no-store`, and a test in the web app's suite fails if it
  ever calls a browser storage API. A generated picture is shown but not linked,
  because opening it in a tab would put its address in your browser history.

## What it can reach

Incognito works from an **allowlist** — the tools it may call. The list is
checked against the live tool set on every turn, so a tool added later is
refused until someone adds it to the list, rather than reachable until
someone remembers to block it.

| Reachable | Not reachable |
|---|---|
| Mail and calendar **reads** | Anything that sends or writes outside the chat's folder, including tools the [outbox](/docs/features/security/outbox) would stage as a draft |
| `web_search` and `web_open` | The knowledge graph, reads included: it logs the text of every query |
| The file tools, confined to the chat's own folder | Creating documents, calendar holds, messages to other agents |
| `shell`, only inside a sealed sandbox (see below) | Every other MCP server's tools |
| `image_generate`, only where the image server's copies can be deleted (see below) | |
| `http_fetch`, until the [interlock](/docs/features/security) arms | |

Like any web chat, it starts **read-only**, and the permission chip works as
usual.

Three of those rows come with conditions:

- **`shell` runs only in a sealed sandbox**: `bwrap` or `docker`, with no
  extra readable or writable paths and no network. The file tools are
  confined by mecha's own path check, but `shell` is confined only by the
  [sandbox](/docs/features/security/sandbox). Under `none` or `landlock` it
  could write to a shared `/tmp` that closing the chat never cleans, so
  there `shell` is not offered.
- **`image_generate` needs `server_temp_dir`.** The image server keeps its own
  copies of the pictures it handles. mecha deletes them after every job, but
  it can only do that if it knows where they are. Set
  [`[image] server_temp_dir`](/docs/features/tools/image-generation) to a path
  on tmpfs, and incognito offers the tool. Without it, the tool is not
  offered, because that promise could not be kept.
- **`http_fetch` usually stops working partway through, and that is
  correct.** A typical incognito chat reads your mail (private data) and
  then searches the web (third-party content). That combination arms the
  trifecta interlock, which from then on refuses any tool where the model
  picks the destination. `web_open` still reads a search result's page.

## When it refuses to open

The button reports *incognito is unavailable* and gives the reason, rather
than opening a chat that can't keep its promise. The reasons:

- **The chat model is not on this machine**, or its provider has
  `fallbacks`. A cloud provider keeps the text on its own servers, and a
  fallback would silently re-send the whole conversation to one the moment
  the local server hiccuped. The model chip's tooltip says an incognito chat
  runs only on the local model. It follows a
  [model switch](/docs/features/models/switching) like any web chat, and the
  checks run again on every turn against the model actually loaded.
- **A deny-gate hook is configured** (`pre_tool` or `pre_task_close`).
  Incognito runs no hooks. Skipping a hook that exists to refuse calls would
  let the chat do more than you allowed, so it refuses to open instead.
- **The runtime directory is not tmpfs**, or `$XDG_RUNTIME_DIR` is not set.

## What it does not promise

- **Other parties see what they see.** A search reaches the search engine and
  a mail read reaches the mail provider's API — the usual private-browsing
  bargain. The banner says so from the start.
- **The model server's memory.** llama-server keeps the conversation's cache
  in RAM until that slot is reused or the server restarts. The image server
  keeps the last prompt in memory until its next job or its idle unload.
  Both are processes, not files, and both lose the data on restart.
- **Swap.** The kernel can page RAM, tmpfs included, out to disk under memory
  pressure. Encrypted swap or zram closes that; software alone cannot.
- **A debug log level.** `MECHA_LOG=debug` logs content on purpose, for
  diagnosis. The promise holds at the default level.
- **What you take with you.** A downloaded picture, copied text or a
  screenshot is yours to keep.

## How it is checked

An end-to-end test drives the real server routes. It puts a unique canary
string in a turn and in an uploaded file, then scans the whole mecha home: it
must find nothing while the chat is open and nothing after it closes. The test
also runs the same turn in an ordinary chat and requires the canary to turn up
there. Without that second half, a scan that looked in the wrong place would
pass just as easily.

The design, the owner's rulings and the audit of every place a chat writes are
in `docs/INCOGNITO-DESIGN.md` in the repository.
