---
title: Privacy policy
description: What mecha does with Google account data, where it is stored, and who else can see it.
---

# Privacy policy

_Last updated: 15 September 2026_

**mecha** is an open-source agent harness that runs on a person's own computer.
It is maintained by Luke Chang and distributed under the MIT license at
[github.com/ljchang/mecha](https://github.com/ljchang/mecha).

This policy describes what happens to Google account data when someone connects
a Google account to their own installation of mecha.

**The agent runs on your machine.** Every copy of mecha holds its own
credentials and talks to Google directly; no part of the agent, its mail
handling, its calendar work or its model calls passes through anything the
maintainer runs. Your OAuth tokens never leave your computer.

**One optional piece is a hosted service, and it is worth reading about
separately.** If you want a public surface — a booking page strangers can use,
a form they can submit — that surface is served by *mecha-factory*, which is a
real multi-user server. It is optional, it is off unless you publish something
to it, and it is described in [its own section below](#the-public-surface). It
never receives your mail, and it never holds an OAuth token.

## What mecha asks for, and why

When you connect a Google account, mecha requests these OAuth scopes:

| Scope | What it allows |
| --- | --- |
| `gmail.modify` | Read your mail, change labels and read state, **report a thread as spam, and move a thread to the trash**. Trashing is recoverable and spam also trains your provider's filter. It stops deliberately short of `https://mail.google.com/`, so it does **not** allow permanent deletion. |
| `gmail.send` | Send mail. |
| `calendar` | Full access to your calendars — read, create, update and **delete** events, and read free/busy time. Deletion is a shipped capability, not a theoretical one. |
| `calendar.events` | Create and update individual events — for example, turning a confirmed booking into a meeting on your calendar. |

Google Docs support is a **separate, optional grant** with its own OAuth client
and its own token file. It asks for one scope, `drive.file`, which gives access
only to files you explicitly pick and files the app itself creates — never your
whole Drive. Within that set it can read, write and **move a file to your
trash**, which is recoverable.

mecha asks for any of this only if you choose to connect an account. Connecting
one is optional; mecha runs with no mail, calendar or document access at all.

## Where the data goes

**It stays on your computer.** Specifically:

- **OAuth tokens** are written on your own machine — mail and calendar at
  `~/.mecha/mail/<account>/oauth.json`, documents at
  `~/.mecha/docs/<account>/oauth.json` — with owner-only permissions (`0600` on
  the file, `0700` on the directory). They are never transmitted anywhere except
  to Google, to refresh themselves.
- **Message and calendar content** is read on demand over the network from Google
  to your machine. Anything cached is written under `~/.mecha/` on the same machine.
- **Nothing is reported about you.** There is no telemetry, no analytics and no
  crash reporting anywhere in mecha — it never phones home, and the maintainer
  learns nothing about your installation or your use of it. The one service in
  the picture is the optional public surface described below, which you reach
  only by deliberately publishing something to it.

## Who else can see it

Nobody, with one disclosure you should read carefully.

mecha is an agent harness, which means it passes text to a language model in order
to answer questions about it. **Which model, and therefore where that text goes,
is set by you** in `~/.mecha/config.toml`:

- **A local model.** mecha exists to make a local, open-weight model usable as a
  personal assistant, and many installations point it at a model served on the
  same machine. Configured that way, your mail and calendar content does not
  leave your computer at all.
- **A hosted provider.** mecha also supports hosted APIs, and its built-in
  default configuration names one (Anthropic). Configured that way, the content
  you ask it to work with is sent to that provider, and is handled under that
  provider's own terms and privacy policy rather than this one.

Neither choice sends anything to the maintainer of mecha. If you want to know
which applies to your installation, `default_provider` in your configuration file
is the answer.

Two further destinations exist if you enable them, and both are your
configuration to make: **web search backends**, which receive the search queries
mecha issues (a query is itself a channel, which is why mecha treats search as
outbound), and **MCP servers** you connect, which receive whatever the tool call
you asked for sends them. mecha ships with neither pointed anywhere by default.

mecha never sells Google user data, never uses it for advertising, and never uses
it to train a model. Beyond the destinations you configure yourself — the model
provider, any search backend, any MCP server, the Slack transport if you connect
one, and the booking page described below if you publish one — it is not shared
with anyone. Every one of those is off until you turn it on, and each is named in
your own configuration file.

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
  long an undrained row survives is set by the request type's own retention
  policy; a type that sets none keeps its rows until they are drained.

It never receives your mail, your calendar events, your documents, or any OAuth
token — those stay between your machine and Google.

This is the one part of mecha that somebody else can operate on your behalf, and
whoever operates the instance you publish to can see what is on it. mecha-factory
is open source and can be self-hosted, in which case that person is you.

## Other people's data

Everything above is about *your* data. A booking page also collects data from the
people who use it: the name, email address and purpose they type into the form,
plus the slot they chose. That information is handled the same way as everything
else here — it lands in a request file under `~/.mecha/` on your machine and in
the calendar event created for the meeting, and it goes nowhere else.

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

Four things send without passing through that queue even when it is configured,
and all are worth knowing:

- **Creating a calendar event with attendees notifies them immediately.** mecha
  sets `sendUpdates=all`, so the provider mails an invitation from your account
  the moment the event exists. This is true of any event with attendees, not
  only booked meetings.
- **Deleting a calendar event mails a cancellation to every attendee**, and
  unlike creation this is unconditional. Of the three calendar verbs, updating
  an event is the only quiet one.
- **Bookings taken through your own published booking page** become calendar
  events by a scheduled, deterministic path with no model and no review step.
  That is deliberate: you approved those slots when you published them, and a
  visitor who books one should not wait on you to find out whether it took.
- **Meeting polls**, if you run one, mail each invitee their own link and one
  reminder from your account on the same scheduled path, and book the winning
  slot. Same reasoning, same absence of a review step.

## Retention and deletion

All of it is on your machine, so you control it directly:

- **Disconnect one mail/calendar account:** delete `~/.mecha/mail/<account>/`.
- **Disconnect a documents account:** delete `~/.mecha/docs/<account>/`. This is
  a separate grant, so removing the mail one does not revoke it.
- **Remove everything:** delete `~/.mecha/`.
- **Revoke access from Google's side:** visit
  [myaccount.google.com/permissions](https://myaccount.google.com/permissions)
  and remove the app. This invalidates the stored tokens immediately.

Uninstalling mecha removes the software; deleting `~/.mecha/` removes the data.

## About this website

Everything above describes the software. This documentation site is a separate
thing and deserves its own sentence, because "no telemetry, no analytics" is a
claim about mecha and a reader could reasonably hear it as a claim about the page
they are reading.

The site runs no analytics and sets no cookies. It does load its typefaces from
Google Fonts, which means opening any page here — including this one — sends your
IP address and browser user-agent to Google, as loading any third-party asset
does. There is nothing else.

## Children

mecha is developer tooling and is not directed at children under 13.

## Changes

Material changes to this policy will be published on this page with an updated
date above.

## Contact

Questions about this policy: [lukejchang@gmail.com](mailto:lukejchang@gmail.com),
or open an issue at
[github.com/ljchang/mecha/issues](https://github.com/ljchang/mecha/issues).
