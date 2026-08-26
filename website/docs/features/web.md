---
title: The web surface
sidebar_position: 23.5
description: mecha serve — the tailnet web app. What the door is, why it is a network identity rather than a password, and what each page can do that the terminal already could.
---

# The web surface

`mecha serve` is mecha on your phone: a small web app, served by the same
process that holds the agent, reachable from anywhere your tailnet reaches
and from nowhere else.

It exists because the terminal is where mecha lives and the terminal is not
where you are. A draft that needs approving, a thread that needs reading,
a queue that needs clearing — none of that wants a laptop, and all of it was
previously stuck behind one.

## The door

Three rules, and the first two are the whole security model:

- **The bind is `127.0.0.1`, and there is no flag to widen it.** Reaching
  the app from your phone is [`tailscale serve`](https://tailscale.com/kb/1242/tailscale-serve)'s
  job. Reaching it from the internet is nobody's.
- **Identity is the network, verified.** Every request must carry a
  `Tailscale-User-Login` header equal to `[web] owner_login`, which is the
  header `tailscale serve` injects for the authenticated tailnet user.
  Missing header, wrong value, or unset config all fail closed — and the
  server **refuses to start** with no owner configured, because a door with
  no owner check should not open at all.
- **`[web]` is global-file-only.** Like `[slack]`, it is stripped out of
  project-level `mecha.toml` files, so a cloned repository cannot describe
  the door to your own machine.

```toml
[web]
owner_login = "you@example.com"
port = 63242
assets = "~/.mecha/web/dist"
```

Then front it:

```bash
tailscale serve --bg 63242
```

There is no password, no session cookie and no login page, and that is
deliberate: the tailnet already proved who you are, and a second secret
would be one more thing to leak. If you can reach the port, Tailscale says
you are the owner; if the header disagrees, you get a `403` before the
router even looks at the path — including for paths that do not exist, so
an unauthenticated probe learns nothing about what the app contains.

## What the pages do

Every page is a **thin shell over the command line**. Reads come from the
same stores the CLI reads; every mutation runs `mecha <verb>` as a child
process. Nothing is reachable from a browser that a script could not do,
and there is exactly one implementation of each verb.

| Page | What it is |
|---|---|
| **Home** | The dashboard: what is waiting in each store, plus [doctor](/docs/features/run-quality)'s findings. Cards with a surface behind them navigate there on tap. A store that cannot be read shows a dash, never a zero |
| **Chat** | A streaming conversation with the agent — steering, cancel, a context gauge, and a session drawer holding both live and recorded conversations |
| **Mail** | Two tabs: the triage queue (what needs you, classified) and a plain inbox (what just arrived). Reading, triaging, and drafting — see [mail](/docs/features/mail) |
| **Notes** | Capture into the [knowledge graph](/docs/graph/overview) as evidence, a recent list, and a search box |
| **Review** | [Outbox](/docs/features/outbox), the graph's merge queue, and the [front door](/docs/features/frontdoor) — every approval surface in one place |
| **Tasks** | The GTD board, with the views in a drawer and one tap per status change |

### Chat, and what a session is

The agent lives in the serve process, so the phone is a view onto it rather
than a second copy. One agent, one provider connection, one cached prefix —
and **many conversations**, each with its own `RunContext`: its own
workspace jail under `~/.mecha/work/web/<key>/`, its own permission mode,
its own cancel token and steering queue.

The drawer lists the conversations this process is holding *and* the ones
recorded earlier, including voice calls. Opening a recorded one **resumes**
it: the messages come back and so does the
[taint](/docs/features/security) — a conversation that read a hostile page
last Tuesday still remembers on Thursday, because resuming must not launder
what a session touched.

### Permission modes, and answering from the phone

A web session starts **read-only**: reads run, and anything that would send
is staged in the [outbox](/docs/features/outbox) instead. Switching a
session to `ask` turns every other tool call into an approval card on the
page — with a real reason field, because a denial with a reason is a
correction the [learner](/docs/features/learning) can use and a bare "no" is
not. `allow` runs them without asking.

The chip in the header is the control and the display, and it cycles in
ascending order of what runs unasked. **Entering `allow` asks first; leaving
it does not** — every other change only ever *adds* a gate, and a
confirmation on a harmless change is what teaches people to tap through the
ones that matter.

Four properties worth knowing:

- **The chip tracks the session, not your tap.** The mode travels as its own
  event, so changing it on the phone moves the chip on the laptop watching
  the same session, and a request whose response was lost leaves the chip
  where the server actually is. What the chip is *for* is telling you whether
  the next write stops to ask, so a stale one is a security cost rather than
  a cosmetic one.
- **A card shows the call the way a person reads one.** A calendar call leads
  with its title and when it is — in reading order, not alphabetical, where
  an event reads end before start — and a letter leads with its addressing
  and its prose. The whole call is one tap away and nothing is hidden: an
  argument with no header or body shape, which is where `shell` keeps its
  entire contents, is shown outright.
- A card is **claimed atomically**. Answer on the laptop and the phone's
  copy goes stale rather than double-answering.
- **A pending card survives a locked phone.** The card rides the transcript
  read, so reloading the page lands you back on the question instead of on a
  run that silently parked.

Read-only tools never generate a card in any mode — `web_search`, `fs_read`
and `recall` declare themselves read-only and are allowed without asking,
which is why turning on `ask` does not make a research run unusable.

An unanswered card times out as *blocked by policy*, not as a user denial —
machine refusals and human corrections are different facts and only one of
them should teach the learner anything.

### Files

Attach a file in Chat and it lands in that session's `inbox/` inside the
workspace jail; the **path** is what goes into the message, so the model
reaches it with `fs_read` and the taint arms through the ordinary file tool
rather than a parallel route. Downloads prove containment the same way every
model-supplied path does — canonicalize, then require the result to sit
inside the jail — and anything outside it and anything missing are the same
`404`.

Only images are served with a renderable content type. Everything else
downloads as inert bytes, because a file in the jail may be model-written
and HTML served from your own origin would run script against this very API.

### Dictation

The mic button on the notes and task capture boxes records, encodes the clip
in the page, and posts it to **Parakeet running on your own machine**. The
audio does not leave the box — which is the entire reason not to use the
browser's built-in speech APIs, which ship your voice to a third party.

Dictation needs the speech server from the [voice stack](/docs/features/voice);
without it, the button reports that the transcriber is unreachable and
typing still works.

## What it does not do

- **It is not a second agent.** The TUI and the Slack connector still build
  their own; three agent-owning processes against one llama-server is the
  live shape, and whether serve should become the shared backend is an open
  question rather than a plan.
- **It does not put `allow` one tap from the default.** The mode exists on
  the page now, and the argument against it was never wrong — a surface that
  can grant blanket permission from a phone is a surface that will, one
  distracted tap at a time. What answers it is a confirmation on the way in
  and nowhere else, plus what `allow` still cannot waive: the
  [interlock](/docs/features/security) refuses a send once the conversation
  holds both private and outside content, whoever approved what, and
  outbox-routed calls stage rather than send. `allow` removes the tap, not
  the boundary.
- **There is no push yet.** A page that is open streams; a page that is
  closed catches up on reload. [Slack](/docs/features/slack) remains the
  channel that can reach you when nothing is open.

## Installing the app itself

The web assets are a **build artifact**, not part of the crate — so
`cargo install` updates the binary and not the pages:

```bash
cd <checkout>/web && npm ci && npm run build
rsync -a --delete dist/ ~/.mecha/web/dist/
systemctl --user restart mecha-serve.service
```

Verify the *served* page rather than the directory: load the door and check
the bundle changed. A stale `dist` next to a fresh binary is the failure
that looks exactly like nothing happening.

## The commands

```bash
mecha serve                          # the door, on [web] port
mecha serve --port 8080              # override for one run
mecha serve --owner-login you@ex.com # override the owner for one run
mecha serve --voice-port 8990        # mount the voice facade in-process
mecha serve --assets ./web/dist      # serve a build from somewhere else
```

`scripts/mecha-serve.service` is the systemd unit that keeps it up.
