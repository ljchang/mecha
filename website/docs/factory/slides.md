---
title: Live polls on a slide
sidebar_position: 8
description: The projector page, and the PowerPoint content add-in that puts it on a slide — install, insert, and what fails soft.
---

# Live polls on a slide

Every way a poll reaches a room consumes **one URL**: the projector page, minted
as a creator capability when the poll is created.

```
https://<gate>/p/<handle>/<poll>/screen/<token>
```

It renders results only — big type, no form, the join URL printed large across
the top — and refreshes every two seconds. One projector is one client, so the
interval can afford lecture speed. [See one](/docs/factory/gallery#the-projector),
rendered by the code that serves it.

Nothing about the poll pipeline knows which presentation app is running. A
browser window on the second display, an `<iframe>` in a reveal.js or Quarto
deck, an OBS pipe, and the PowerPoint add-in below are four consumers of that
same page. The per-app story is deployment advice, not code.

:::tip[The browser window is not the compromise]
A second-display browser window is the first-lecture flow and the permanent
fallback. It is also what PollEverywhere actually ships on the Mac, and the only
option on two of the three big presentation apps. Everything below is an
optimisation on top of a thing that already works — and the one that cannot
break.
:::

## Getting the projector URL

`polls create` prints it on the **`projector:`** line. After that, `mecha tui` →
`/polls` → **`s`** shows it for the selected poll.

For a roster poll the line comes with a reminder attached — *aggregates only;
prose stays off the wall* — because the projector page and the presenter's own
screen deliberately show different things. See [the prose
boundary](/docs/factory/polls#the-prose-boundary).

## The PowerPoint add-in

A **content add-in** puts the chart on the slide as an object, saved in the
deck. It is the one native embed cheap enough to have earned a build slot,
because it is *not an app*: no crate, no MCP server, no process, nothing in
`mecha-core`. The whole install artifact is ~60 lines of XML pointing at a
wrapper page the box already serves.

### 1. Write the manifest

```sh
factory-publish polls addin
# manifest → mecha-polls.xml
#
# Sideload it once per machine: …
```

`--out <path>` puts it somewhere else. The command fails if your gate is not
HTTPS, because Office loads add-ins over HTTPS only — better a refusal naming
the gate than an add-in that installs and shows nothing.

:::note[Regenerating upgrades; it does not multiply]
The manifest's `<Id>` is **derived from the gate URL** by sha-256, not minted
fresh per run. Office treats that GUID as the add-in's identity, so running
`polls addin` again produces the *same* add-in — an upgrade — rather than a
second one accumulating in the sideload folder. Two gates get two ids, which is
also right: they are two add-ins pointing at two wrappers. A trailing slash on
the gate does not change who you are.
:::

### 2. Sideload it, once per machine

**PowerPoint for Mac** — drop it in the `wef` folder and restart:

```sh
cp mecha-polls.xml \
  ~/Library/Containers/com.microsoft.Powerpoint/Data/Documents/wef/
```

Then **Home → Add-ins** lists **Live poll**.

**PowerPoint on the web** — Home → Add-ins → More Settings → Upload My Add-in,
and pick the file. Useful for a look; not the lecture surface, because the web
slideshow does not persist content add-ins.

**Windows** is the same manifest through a different door: "Upload My Add-in"
for a single machine, or a network-share catalog or M365 centralized deployment
for a department. The CLI prints the Mac and web routes because those are the
ones that are one command; the Windows catalog setup is Microsoft's own
documentation and not something this tooling can shorten.

### 3. Insert it on a slide

Insert the add-in on the slide you want the chart on. In edit view it asks once
for the poll's projector URL. Paste it, click **Use this poll**, and the object
goes to standby — *the chart appears when the slideshow starts*. **preview now**
shows it immediately without starting the show; **change poll** puts a different
poll on the same object.

The wrapper checks what you paste: it must be the same origin as the gate
serving the wrapper, and its path must start with `/p/`. A URL from somewhere
else is refused in words rather than becoming a blank frame — and the page's own
Content-Security-Policy enforces the same thing regardless.

### 4. Advance from question to question

There is no series object and no activation pointer. Each poll is its own URL,
and putting a question in front of the room is putting *its* join URL on the
slide — so one add-in per slide, each pointing at that slide's poll, is the
whole choreography.

Set the polls `show = "creator"` and it completes itself: student phones show
the ballot and never the results, the room sees results exactly when the slide
with the add-in is up. **Presenter-controlled reveal with no new enum** — the
existing [visibility policy](/docs/factory/polls#who-sees-what) doing the work.

## What the deck carries

The saved `.pptx` holds a **URL, never content**. The wrapper persists the
projector URL per insertion through Office.js document settings — the Mentimeter
pattern — with `localStorage` as a same-machine backstop. A deck mailed to a
colleague carries the pointer, not last week's results, and a poll that has
since closed shows its closed page.

That is also why the object is empty until you paste something: there is no
state in the manifest, and the manifest is identical for every deck on the
machine.

## Everything fails toward the chart

`ActiveViewChanged` — the Office.js event that says "the slideshow started" — is
the joint Microsoft has broken before; a Mac 16.94 regression killed it for
months. So the wrapper has exactly one hard rule: **a blank object on a slide
mid-lecture is the outcome this file exists to prevent.**

| what breaks | what you get |
|---|---|
| `getActiveViewAsync` fails | the chart, live in both edit and show views |
| `ActiveViewChanged` won't register | the chart in both views, plus a line saying this PowerPoint doesn't announce the slideshow |
| Office.js never readies (a dead CDN mid-lecture) | after four seconds, the chart from this machine's last poll |
| No Office at all (a plain browser on the wrapper URL) | a small launcher for the projector page |

Each of those degrades to *more* visible, never less.

## The two routes' own CSP

The `/slides/addin` routes declare their own Content-Security-Policy instead of
inheriting the gate's form policy, and it is worth knowing why the exception
exists rather than discovering it:

- `office.js` loads from Microsoft's CDN — self-hosting it is unsupported — so
  `script-src` names `appsforoffice.microsoft.com`.
- The chart is this origin's own projector page in a frame, so `frame-src`
  allows `'self'`.
- `frame-ancestors` is omitted deliberately: the page exists to be embedded by
  PowerPoint's webview.

The header middleware only fills in what a handler left unset, so declaring here
*is* the override. Nowhere else on the gate gets either allowance.

## Known costs, accepted

- **HTTPS only**, and desktop PowerPoint only — no web-slideshow persistence, no
  iPad.
- **WebView2** on Windows.
- Clicks inside the add-in region don't advance the slide.
- Office.js regressions arrive on Microsoft's schedule.

All tolerable for a self-sideloaded tool with a browser tab as the fallback that
cannot break.

## The other presentation apps

**Keynote** gets nothing to build, by evidence rather than neglect: it has no
add-in model. The browser window on the second display is the story there.

**Google Slides** is an open empirical question — whether a `batchUpdate`
renders into an active Present session is undocumented, and the incumbents route
around it. Until someone answers it, a browser window.

**Quarto revealjs is the deck an agent should author.** It is text, so `.qmd`
flows through [the outbox](/docs/features/outbox) and *the diff of the source is
the review* — which is the harness's whole value. `quarto render` runs in the
sandbox, and the rendered deck is a bundle [the publish
path](/docs/features/publishing) already stages. A poll slide is one line of
markdown plus the join URL. Editing a `.pptx` through a binary-format MCP server
defeats exactly this: the outbox could only show it as arguments.

## Where to go next

- [Polls](/docs/factory/polls) — the spec, the audiences, and who sees what.
- [Publishing](/docs/features/publishing) — how a rendered deck reaches a URL.
- [The outbox](/docs/features/outbox) — why a draft deck is reviewable as a diff.
