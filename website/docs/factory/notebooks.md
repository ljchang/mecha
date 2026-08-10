---
title: Notebooks
sidebar_position: 5
description: Publishing a marimo notebook that runs in the reader's browser — the vendored runtime, the compute policy, and what the export does and does not execute.
---

# Notebooks

A notebook is the one artifact that **runs**. A reader opens the URL and the code
executes in their browser under Pyodide — no server, no session, no kernel of
yours doing work for a stranger. That is what makes it publishable in the same
breath as a static report.

```bash
factory-publish notebook analysis.py --out ./bundle --vendor-runtime 0.28.0
factory-publish publish notes-2026 ./bundle --title "Where the effect went"
```

The second command is an ordinary publish, so everything in
[Artifacts](/docs/factory/artifacts) applies: versions, an alias, visibility,
sharing, takedown.

## The export does not run your notebook

This is worth stating plainly because the tooling itself claimed otherwise for a
long time, and the wrong claim was load-bearing — it was the stated reason
notebooks were kept off unattended paths.

`marimo export html-wasm` **parses** the notebook. It does not execute the cells,
and it does not even import the module: a statement at file top level does not
run either. Measured rather than reasoned about — a notebook whose cell writes a
file exports without writing it, and a notebook that imports a package which does
not exist and then raises `SystemExit` exports cleanly.

Two things follow:

- **Rendering somebody's notebook is not running it.** The dangerous-looking step
  is not the dangerous step.
- **The timeout is still there**, and still worth having, because any subprocess
  can hang. It just is not protecting you from your own cells.

What *does* reach the network at render time is `--vendor-runtime`, which fetches
Pyodide. That, not execution, is the one thing this template does that a confined
renderer would have to be allowed.

## `--vendor-runtime` is not optional in practice

Without it the bundle keeps marimo's CDN loader, and **it will not boot** on an
origin that enforces the policy — which this one does. The command will let you
build one anyway with `--allow-unvendored-runtime`, but that is a diagnostic;
the result must never be published, and the render says so.

Vendoring downloads Pyodide at the version you name, from a pinned allowlist of
hosts, and records a digest for every file it embeds. The bundle is then
self-contained: it boots with no third-party request at all, which is what lets
it be served under a policy that forbids them.

```
  runtime pyodide 0.28.0 — 11 files, 4 package(s), 12.4 MB
  pinned  runtime/pyodide.asm.wasm  sha256:…
```

Expect the bundle to be **large** — a vendored runtime is tens of megabytes.
That is the price of a page that boots from one origin and keeps working when a
CDN does not.

## Why it is `compute` class, and what that buys

Bundles are served under one of three policies. A notebook is `compute`, which is
the only one that grants `wasm-unsafe-eval` — Pyodide cannot run without it.

Two details in that policy matter to anyone debugging a notebook that works
locally and not when published:

- **`wasm-unsafe-eval`, never `unsafe-eval`.** The narrow directive permits
  WebAssembly compilation and nothing else. Python-in-the-browser works;
  `eval()` of arbitrary JavaScript still does not.
- **COOP and COEP are on**, which is what unlocks `SharedArrayBuffer`. It also
  means any resource the page pulls must be same-origin or explicitly
  cross-origin-isolated — another reason the runtime is vendored rather than
  fetched.

### What framing costs, exactly

Cross-origin isolation has to be granted by the *top-level* document and every
frame above yours. The [viewer page](/docs/factory/artifacts#two-urls-and-which-one-to-send)
is not isolated, so a notebook opened there has no `SharedArrayBuffer` — while
the same notebook opened at its bare URL does.

What that costs is one feature, and it is worth knowing which. marimo
feature-detects: with isolation it allocates an interrupt buffer, and without it
it logs `Not running in a secure context; interrupts are not available.` and
carries on. So a framed notebook boots, runs its cells, and renders exactly as it
otherwise would — **you just cannot interrupt a running cell.**

Send someone the page. If interrupting long-running cells is the point of a
particular notebook, send the bare URL for that one.

The class is read from what the renderer recorded, never assumed. A `compute`
bundle published as `static` would be served from the wrong origin under a policy
it cannot boot under, so `publish` reads the record rather than taking your word
for it.

## Checking it before anyone else sees it

```bash
factory-publish serve ./bundle --class compute
```

Loopback only, with the **real headers for its class**. That last part is the
point: a bundle checked without its Content-Security-Policy is a bundle checked
against something the world never sees. There is a `--no-csp` escape for finding
out *what* a bundle needs, but a bundle that works that way has been told
nothing.

## Agents

`notebook_render` is on the [MCP surface](/docs/factory/onboarding#4-wire-it-into-an-agent),
so an agent can render one; publishing it goes through the outbox like every
other publish. It takes the same `vendor_runtime`, and the tool's answer says
explicitly when no runtime was embedded — because the failure mode is a bundle
that publishes fine and then does not boot, which is discovered by a reader
rather than by the run.

## Where to go next

- [Artifacts](/docs/factory/artifacts) — versions, visibility, sharing, takedown
- [The component gallery](/docs/factory/gallery) — what the rendered surfaces look like
