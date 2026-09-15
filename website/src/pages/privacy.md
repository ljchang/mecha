---
title: Privacy policy
description: What mecha does with the accounts you connect, where the data is stored, and who else can see it.
---

# Privacy policy

_Last updated: 15 September 2026_

**mecha** is an open-source agent harness that runs on a person's own computer.
It is maintained by Luke Chang and distributed under the MIT license at
[github.com/ljchang/mecha](https://github.com/ljchang/mecha).

This policy describes what happens to your data when you connect an account to
your own installation of mecha — Google, Microsoft, Slack, or anything else
below. Google is named throughout because its scopes are the most detailed and
because Google reads this page when reviewing the app, but nothing here is
Google-only: where tokens live, who else can see your data, what sends without
review and how to delete it apply the same way to every account you connect.

**The agent runs on your machine.** Every copy of mecha holds its own
credentials and talks to each provider directly; no part of the agent, its mail
handling, its calendar work or its model calls passes through anything the
maintainer runs. Your OAuth tokens never leave your computer.

**One optional piece is a hosted service, and it is worth reading about
separately.** If you want a public surface — a booking page strangers can use,
a form they can submit — that surface is served by *mecha-factory*, which is a
real multi-user server. It is optional, it is off unless you publish something
to it, and it is described in [its own section below](#the-public-surface). It
never receives your mail, and it never holds an OAuth token.

## In short

- **The agent runs on your machine.** Your credentials never leave it, and
  nothing about your installation or your use of it is reported to anyone. Your
  mail, calendar and documents stay there too — except where you have pointed
  mecha at something that is not on your machine, which is the next two points.
- **Nothing is connected until you connect it.** Every integration below is
  optional and off by default, and each one names itself in your own
  configuration file.
- **The exceptions are yours to switch on**, and each has its own section: the
  language model you choose, and the public surface if you publish a booking
  page or a form.

| What | What it reaches | Where its credentials live |
| --- | --- | --- |
| [mecha-mail](#mecha-mail--mail-and-calendar) | your mail and calendar, at Google or Microsoft | `~/.mecha/mail/<account>/` |
| [mecha-docs](#mecha-docs--documents) | only documents you pick or it creates | `~/.mecha/docs/<account>/` |
| [mecha-slack](#mecha-slack--the-remote-control) | one Slack workspace you install it into | `~/.mecha/slack/` |
| [a knowledge graph](#a-knowledge-graph) | a store outside `~/.mecha/`, if you connect one | its own |
| [search and MCP servers](#web-search-and-mcp-servers) | whatever you point them at | your config |
| [your model provider](#your-model-provider) | the text you ask about | your config |
| [the public surface](#the-public-surface) | strangers, deliberately | not applicable |

## Where the data goes

**It stays on your computer.** Specifically:

- **OAuth tokens** are written on your own machine, under `~/.mecha/`, with
  owner-only permissions (`0600` on the file, `0700` on the directory). Each
  integration below names its own path. They are never transmitted anywhere
  except back to the provider that issued them, to refresh themselves.
- **Message, calendar and document content** is read on demand over the network
  from the provider to your machine. Anything cached is written under
  `~/.mecha/` on the same machine.
- **Nothing is reported about you.** There is no telemetry, no analytics and no
  crash reporting anywhere in mecha — it never phones home, and the maintainer
  learns nothing about your installation or your use of it. The one service in
  the picture is the optional public surface described below, which you reach
  only by deliberately publishing something to it.

Each integration below is optional, off until you connect it, and named in your
own configuration file. What follows is what each one reaches — and what it
cannot.

## mecha-mail — mail and calendar

One component, two providers. Connect either, both, or neither; mecha runs with
no mail or calendar access at all. Tokens for both live at
`~/.mecha/mail/<account>/oauth.json` with owner-only permissions (`0600` on the
file, `0700` on the directory) and are never sent anywhere except back to the
provider that issued them, to refresh themselves.

### Google

| Scope | What it allows |
| --- | --- |
| `gmail.modify` | Read your mail, change labels and read state, **report a thread as spam, and move a thread to the trash**. Trashing is recoverable and spam also trains your provider's filter. It stops deliberately short of `https://mail.google.com/`, so it does **not** allow permanent deletion. |
| `gmail.send` | Send mail. |
| `calendar` | Full access to your calendars — read, create, update and **delete** events, and read free/busy time. Deletion is a shipped capability, not a theoretical one. |
| `calendar.events` | Create, update and delete individual events — for example, turning a confirmed booking into a meeting on your calendar, or removing one that was cancelled. |

Consent happens in your browser and the token comes back to a listener on your
own machine; nothing brokers it.

### Microsoft

| Scope | What it allows |
| --- | --- |
| `Mail.ReadWrite` | Read your mail and change its state — the same triage moves as Google's `gmail.modify`. |
| `Mail.Send` | Send mail. |
| `Calendars.ReadWrite` | Read, create, update and delete calendar events, and read free/busy time. |
| `offline_access` | Keep working without re-consenting every hour. This is the scope that yields a refresh token. |

Two differences from Google worth knowing:

- **Updating an event notifies attendees.** Microsoft Graph mails them on
  create, update *and* delete; Google is silent on update alone. Both notify on
  create when there are attendees, and both notify on delete unconditionally —
  so the difference is one verb, and it is the one you would least expect to
  send mail.
- **Sign-in uses a device code** — mecha shows you a code, you enter it at
  Microsoft. Some organisations block that flow, or require an administrator to
  approve the app before a member can consent at all.

From 31 December 2026 Microsoft moves changes to *sensitive* mail properties
behind a further scope. mecha does not touch those properties, so the list above
is unchanged by it.

## mecha-docs — documents

A **separate, optional grant with its own OAuth client and its own token file**
at `~/.mecha/docs/<account>/oauth.json`. Removing your mail account does not
revoke it, and removing this one does not affect mail.

It asks for exactly one scope, `drive.file`. That scope reaches **only files you
explicitly pick and files the app itself creates** — never your whole Drive,
never anything you have not handed it. Within that set it can read, write and
move a file to your trash, which is recoverable.

## mecha-slack — the remote control

Optional, and only if you install mecha's Slack app into a workspace yourself.
It lets you drive mecha from Slack instead of a terminal.

Its credentials live at `~/.mecha/slack/credentials.json`, owner-only. It dials
Slack rather than listening, so **no port on your machine is ever exposed**. What
reaches Slack is what you send through it — the messages of the conversation you
are having, and any files you move in either direction — and that is handled
under Slack's terms and your workspace's retention settings, not this policy.

## A knowledge graph

If you connect one, mecha can distil what a session left behind into an
*episode* and file it there as evidence. A graph keeps **its own store, outside
`~/.mecha/`** — so deleting mecha's directory does not empty it, and removing it
is a separate step. See [Retention and deletion](#retention-and-deletion).

## Web search and MCP servers

Both are off until you point them somewhere.

- **Web search backends** receive the queries mecha issues. A query is itself a
  channel — what you search for says something — which is why mecha treats
  search as outbound rather than as reading.
- **MCP servers** you connect receive whatever the tool call you asked for sends
  them. What that is depends entirely on the server.

mecha ships with neither configured.

## Your model provider

mecha is an agent harness, which means it passes text to a language model in
order to answer questions about it. **Which model, and therefore where that text
goes, is set by you** in `~/.mecha/config.toml`:

- **A local model.** mecha exists to make a local, open-weight model usable as a
  personal assistant, and many installations point it at a model served on the
  same machine. Configured that way, your mail and calendar content does not
  leave your computer at all.
- **A hosted provider.** mecha also supports hosted APIs, and its built-in
  default configuration names one (Anthropic). Configured that way, the content
  you ask it to work with is sent to that provider, and is handled under that
  provider's own terms and privacy policy rather than this one.

Neither choice sends anything to the maintainer of mecha. If you want to know
which applies to your installation, `default_provider` in your configuration
file is the answer.

## Who else can see it

Nobody beyond the destinations you configured yourself — the model provider, any
search backend, any MCP server, the Slack transport, a knowledge graph, and the
public surface if you publish one. Every one is off until you turn it on, and
each is named in your own configuration file.

mecha never sells your data, never uses it for advertising, and never uses it to
train a model.

### If you publish a booking page

This one is different from the others, because it is the only place a **stranger**
reads something derived from your data. If you publish a booking page, mecha
computes free slots from your calendars' busy time and pushes them to that page.
What is published is availability — when you are free, in the windows you chose —
never event titles, attendees, or any other detail of what you are busy *with*.
Take the page down and nothing further is published.

## The public surface

Publishing a booking page or a public form means putting something on a server
that strangers can reach. That server is *mecha-factory*, and this is what it
holds:

- **Your published availability** — the free slots computed from your calendars,
  pushed to it so the booking page can show them. Availability only: when you
  are free, in the windows you chose, never event titles, attendees or anything
  about what you are busy with.
- **Inbound submissions, until your machine collects them.** A booking or a form
  submission is queued there and stays queued until your mecha drains it. How
  long an undrained row survives is set by that request type's retention policy,
  which you configure when you publish the type.

If you publish documents or pages through it, those are there too — but you
wrote those deliberately, and they are not derived from any account you
connected.

It never receives your mail, your calendar events, your documents, or any OAuth
token — those stay between your machine and the provider.

This is the one part of mecha that somebody else can operate on your behalf, and
whoever operates the instance you publish to can see what is on it. mecha-factory
is open source and can be self-hosted, in which case that person is you.

## Other people's data

Everything above is about *your* data. A booking page also collects data from the
people who use it: the name, email address and purpose they type into the form,
plus the slot they chose. That information is queued on the public
surface until your machine collects it, and then lands in a request file under
`~/.mecha/` and in the calendar event created for the meeting.

It may also reach your model. Working out how to answer a submission means
reading it, so if you have configured a hosted provider, a visitor's words can
travel the same hop your own mail does — described under *Who else can see it*
above, and true of their data for the same reason. mecha is built so that free
text a stranger typed is read by a quarantined pass with no tools and no
history, and only typed fields reach anything with access to your accounts;
that is a safety boundary, not a privacy one, and it does not change where the
bytes go.

If you publish such a page, you are the one collecting that information and the
one answerable for it. mecha gives visitors a link to cancel, which frees the
slot and withdraws the meeting.

## Limited Use

mecha's use of information received from Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

## Sending is not automatic

mecha can route outbound tools through a local review queue: a call becomes a
draft, and nothing leaves until a person reads it and releases it. **This is off
until you turn it on** — the setting is `[outbox] tools` and it is empty by
default, because routing a tool is a policy decision rather than something to
assume. Configured, it is the strongest guarantee here. Unconfigured, a send is
a send.

**Two things bypass that queue entirely, even when it is configured**, because
neither is a tool call:

- **Bookings taken through your own published booking page** become calendar
  events on a scheduled, deterministic path with no model and no review step.
  That is deliberate: you approved those slots when you published them, and a
  visitor who books one should not wait on you to find out whether it took.
- **Meeting polls**, if you run one, mail each invitee their own link and one
  reminder from your account on that same path, and book the winning slot. Same
  reasoning, same absence of a review step.

Everything else that sends is an ordinary tool call and is covered by the queue
if you have routed it — including the calendar ones.

Worth knowing separately, because it is about *who hears about it* rather than
about review: **creating a calendar event with attendees notifies them
immediately**, and **deleting one mails a cancellation unconditionally**. On
Google, updating an event is the single calendar operation that notifies
nobody; on Microsoft there is no quiet one — Graph mails attendees on create,
update and delete alike. The same difference is stated under
[mecha-mail](#microsoft), which is two places to keep in step the next time a
provider changes a default.

## Retention and deletion

Most of it is on your machine, so you control it directly:

- **Disconnect one mail/calendar account:** delete `~/.mecha/mail/<account>/`.
- **Disconnect a documents account:** delete `~/.mecha/docs/<account>/`. This is
  a separate grant, so removing the mail one does not revoke it.
- **Disconnect Slack:** delete `~/.mecha/slack/`.
- **Remove everything local:** delete `~/.mecha/`. This also catches credentials
  from older installs, which kept them in a per-provider file rather than per
  account.

Deleting a token stops mecha using it. **Revoking at the provider invalidates
it**, whether or not you deleted the local copy, and that is the stronger move
if you are not sure what a machine still holds. It is a different place for
each:

| Provider | Where |
| --- | --- |
| **Google** | [myaccount.google.com/permissions](https://myaccount.google.com/permissions) — remove mecha. This covers the mail/calendar grant and the documents grant separately, since they are two clients; revoke both if you connected both. |
| **Microsoft** | Your account's app permissions. On a **work or school** account your administrator can also revoke it for you, and on some tenants only they can. |
| **Slack** | Remove the app from the workspace, in that workspace's app settings. A workspace owner can do this whether or not you can. |

**Two things live outside `~/.mecha/`, and `rm` does not reach either.**

*The public surface*, if you publish one:

- **Collect what is queued:** draining brings submissions to your machine and
  clears them from the server. Undrained rows also age out under the retention
  policy you set for that request type.
- **Stop new arrivals:** unpublish the booking page or form. Nothing further is
  accepted or stored for it.
- **Your availability:** published slots are replaced on every refresh and stop
  being pushed once the page is gone.

This is also the part that holds other people's data — worth knowing before you
publish rather than after.

*A connected knowledge graph*, if you connect one. mecha can distil what a
session left behind into an episode and file it there as evidence. That graph
keeps its own store, in its own place, and removing mecha does not empty it —
so deleting it is a step of its own, wherever you put it.

Uninstalling mecha removes the software. Removing the data means `~/.mecha/`
plus whichever of those two apply to you.

## About this website

Everything above describes the software. This documentation site is a separate
thing and deserves its own sentence, because "no telemetry, no analytics" is a
claim about mecha and a reader could reasonably hear it as a claim about the page
they are reading.

The site runs no analytics and sets no cookies. It does load its typefaces from
Google Fonts, which means opening any page here — including this one — sends your
IP address and browser user-agent to Google, as loading any third-party asset
does. If you use the light/dark switch, your choice is remembered in your own
browser's local storage and is never transmitted. That is everything.

## Children

mecha is developer tooling and is not directed at children under 13.

## Changes

Material changes to this policy will be published on this page with an updated
date above.

## Contact

Questions about this policy: [lukejchang@gmail.com](mailto:lukejchang@gmail.com),
or open an issue at
[github.com/ljchang/mecha/issues](https://github.com/ljchang/mecha/issues).
